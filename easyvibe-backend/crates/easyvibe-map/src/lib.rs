//! 地图领域：仓库注册、map.json/growth.log 读取、轻量校验、内容哈希监听。
//! 原则：不出残图——校验失败的事件以 map.invalid 上报，合法缓存继续服务。
use easyvibe_common::ApiError;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{info, warn};

/// 模块/层 id 的合法字符集（与 Schema pattern 一致）——用于校验 URL 路径参数，防路径遍历
pub fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// 独立轻量自检（不依赖 MapService 实例）：结构级最低要求。
/// 读路径（load_map/缓存）用这一级——存量 v1.0 地图（无 edge id）必须可读；
/// 写验收（巡检产物）用 validate_strict。
pub fn validate_minimum(json: &Value) -> Result<(), ApiError> {
    let obj = json.as_object().ok_or_else(|| ApiError::MapInvalid("根节点必须是对象".into()))?;
    for key in ["version", "meta", "layers", "modules", "edges", "health"] {
        if !obj.contains_key(key) {
            return Err(ApiError::MapInvalid(format!("缺少必填字段 {key}")));
        }
    }
    let modules = obj["modules"].as_array().ok_or_else(|| ApiError::MapInvalid("modules 必须是数组".into()))?;
    if modules.is_empty() {
        return Err(ApiError::MapInvalid("modules 不能为空".into()));
    }
    Ok(())
}

/// 严格验收（写路径）：结构 + v1.1 一致性规则（边 id/端点/层/依赖/封闭枚举）。
/// 审查 Y5 的落地：后端验收不能比外部 agent 还松。
pub fn validate_strict(json: &Value) -> Result<(), ApiError> {
    validate_minimum(json)?;
    let obj = json.as_object().expect("validate_minimum 已确认是对象");
    let modules = obj["modules"].as_array().expect("validate_minimum 已确认");
    let layer_ids: std::collections::HashSet<&str> = obj["layers"]
        .as_array()
        .map(|a| a.iter().filter_map(|l| l["id"].as_str()).collect())
        .unwrap_or_default();
    let module_ids: std::collections::HashSet<&str> = modules.iter().filter_map(|m| m["id"].as_str()).collect();
    let mut edge_ids = std::collections::HashSet::new();
    for e in obj["edges"].as_array().into_iter().flatten() {
        let id = e["id"].as_str().ok_or_else(|| ApiError::MapInvalid("边缺少 id（v1.1 必填）".into()))?;
        if !edge_ids.insert(id) {
            return Err(ApiError::MapInvalid(format!("边 id 重复: {id}")));
        }
        for ep in ["from", "to"] {
            let v = e[ep].as_str().ok_or_else(|| ApiError::MapInvalid(format!("边 {id} 缺少 {ep}")))?;
            if !module_ids.contains(v) {
                return Err(ApiError::MapInvalid(format!("边 {id} 的 {ep}={v} 不存在")));
            }
        }
    }
    for m in modules {
        let mid = m["id"].as_str().unwrap_or("?");
        let layer = m["layer"].as_str().ok_or_else(|| ApiError::MapInvalid(format!("模块 {mid} 缺少 layer")))?;
        if !layer_ids.contains(layer) {
            return Err(ApiError::MapInvalid(format!("模块 {mid} 的 layer={layer} 不存在")));
        }
        for dep in m["dependencies"].as_array().into_iter().flatten() {
            let d = dep.as_str().unwrap_or("");
            if !module_ids.contains(d) {
                return Err(ApiError::MapInvalid(format!("模块 {mid} 依赖 {d} 不存在")));
            }
        }
        // 封闭枚举校验
        let h = &m["health"];
        for (field, allowed) in [
            ("coupling", ["low", "medium", "high", "critical"].as_slice()),
            ("complexity", ["low", "medium", "high"].as_slice()),
            ("churn", ["low", "medium", "high"].as_slice()),
        ] {
            if let Some(v) = h[field].as_str() {
                if !allowed.contains(&v) {
                    return Err(ApiError::MapInvalid(format!("模块 {mid} health.{field}={v} 非法")));
                }
            }
        }
    }
    Ok(())
}

/// 原子写入 JSON 文件（临时文件 + rename，同目录保证原子性）
pub async fn atomic_write_json(path: &Path, value: &Value) -> Result<(), ApiError> {
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| ApiError::Internal(e.to_string()))?;
    tokio::fs::write(&tmp, bytes).await.map_err(|e| ApiError::Internal(format!("写临时文件失败: {e}")))?;
    tokio::fs::rename(&tmp, path).await.map_err(|e| ApiError::Internal(format!("原子替换失败: {e}")))?;
    Ok(())
}

/// 一个已注册的代码仓库（域 1 数据的归属者）
#[derive(Debug, Clone)]
pub struct Repo {
    pub id: String,
    pub name: String,
    pub root: PathBuf,
}

impl Repo {
    pub fn map_path(&self) -> PathBuf {
        self.root.join(".easyvibe/map/map.json")
    }
    pub fn growth_path(&self) -> PathBuf {
        self.root.join(".easyvibe/map/growth.log")
    }
    pub fn submap_path(&self, module_id: &str) -> PathBuf {
        self.root.join(".easyvibe/map/modules").join(format!("{module_id}.json"))
    }
}

/// 地图读取结果：解析 + 轻量一致性自检（结构化校验留给 fixtures 级测试与后续 JSON Schema 接入）
#[derive(Debug, Clone)]
pub struct MapSnapshot {
    pub json: Value,
    pub content_hash: u64,
}

pub struct MapService {
    repos: Vec<Repo>,
    /// 每个仓库最后一次"合法"地图快照（内存缓存）
    cache: tokio::sync::RwLock<std::collections::HashMap<String, MapSnapshot>>,
}

impl MapService {
    pub fn new(repos: Vec<Repo>) -> Arc<Self> {
        Arc::new(Self { repos, cache: Default::default() })
    }

    pub fn repos(&self) -> Vec<Repo> {
        self.repos.clone()
    }

    pub fn find_repo(&self, id: &str) -> Option<Repo> {
        self.repos.iter().find(|r| r.id == id).cloned()
    }

    /// 读取并自检一张地图；通过则更新缓存
    pub async fn load_map(&self, repo: &Repo) -> Result<MapSnapshot, ApiError> {
        let raw = tokio::fs::read_to_string(repo.map_path())
            .await
            .map_err(|e| ApiError::NotFound(format!("map.json 不可读: {e}")))?;
        let json: Value = serde_json::from_str(&raw)
            .map_err(|e| ApiError::MapInvalid(format!("JSON 解析失败: {e}")))?;
        self.check_minimum(&json)?;
        let snap = MapSnapshot { json, content_hash: content_hash(&raw) };
        self.cache.write().await.insert(repo.id.clone(), snap.clone());
        Ok(snap)
    }

    /// 轻量自检：结构性最低要求（完整 JSON Schema 校验在测试/fixtures 层做）
    fn check_minimum(&self, json: &Value) -> Result<(), ApiError> {
        validate_minimum(json)
    }

    pub async fn cached(&self, repo_id: &str) -> Option<MapSnapshot> {
        self.cache.read().await.get(repo_id).cloned()
    }

    /// growth.log → 事件数组（容忍空文件；单行坏 JSON 跳过并告警，不中断）
    pub async fn load_growth(&self, repo: &Repo) -> Result<Vec<Value>, ApiError> {
        let raw = match tokio::fs::read_to_string(repo.growth_path()).await {
            Ok(r) => r,
            Err(_) => return Ok(Vec::new()),
        };
        let mut events = Vec::new();
        for (i, line) in raw.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() { continue; }
            match serde_json::from_str::<Value>(line) {
                Ok(v) => events.push(v),
                Err(e) => warn!("growth.log 第 {} 行跳过: {e}", i + 1),
            }
        }
        Ok(events)
    }

    pub async fn load_submap(&self, repo: &Repo, module_id: &str) -> Result<Value, ApiError> {
        let raw = tokio::fs::read_to_string(repo.submap_path(module_id))
            .await
            .map_err(|e| ApiError::NotFound(format!("子图不存在: {e}")))?;
        serde_json::from_str(&raw).map_err(|e| ApiError::MapInvalid(format!("子图解析失败: {e}")))
    }
}

/// 文件内容哈希
pub fn content_hash(raw: &str) -> u64 {
    let mut h = DefaultHasher::new();
    raw.hash(&mut h);
    h.finish()
}

/// growth.log 增量监视：按字节偏移读取新增行，逐行解析（坏行跳过告警），
/// 文件截断/轮换（len < offset）时从头重发全量。
pub fn spawn_growth_watcher(repo: Repo) -> watch::Receiver<Vec<Value>> {
    let (tx, rx) = watch::channel::<Vec<Value>>(Vec::new());
    tokio::spawn(async move {
        let path = repo.growth_path();
        let mut offset: u64 = 0;
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            let len = match tokio::fs::metadata(&path).await {
                Ok(m) => m.len(),
                Err(_) => continue,
            };
            if len == offset {
                continue;
            }
            let reset = len < offset;
            let raw = match tokio::fs::read_to_string(&path).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            // 防 panic（审查 🟡）：offset 可能越界或落在多字节字符中间（metadata 与 read 间文件被换写），
            // get() 在两个场景都返回 None → 回退从头读
            let new_part = match raw.get((if reset { 0 } else { offset as usize })..) {
                Some(p) => p,
                None => {
                    offset = 0;
                    raw.as_str()
                }
            };
            offset = len;
            let mut batch = Vec::new();
            for (i, line) in new_part.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() { continue; }
                match serde_json::from_str::<Value>(line) {
                    Ok(v) => batch.push(v),
                    Err(e) => warn!("[growth-watch] {} 新行{} 跳过: {e}", repo.id, i + 1),
                }
            }
            if !batch.is_empty() {
                info!("[growth-watch] {} +{} 事件", repo.id, batch.len());
                let _ = tx.send(batch);
            }
        }
    });
    rx
}

/// progress.json 监视：内容变化即推送解析结果（M2-2 起广播 progress.updated）
pub fn spawn_progress_watcher(repo: Repo) -> watch::Receiver<Value> {
    let (tx, rx) = watch::channel::<Value>(Value::Null);
    tokio::spawn(async move {
        let path = repo.root.join(".easyvibe/map/progress.json");
        let mut last: Option<u64> = None;
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            interval.tick().await;
            let raw = match tokio::fs::read_to_string(&path).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            let hash = content_hash(&raw);
            if last == Some(hash) { continue; }
            match serde_json::from_str::<Value>(&raw) {
                Ok(v) => {
                    last = Some(hash);
                    let _ = tx.send(v);
                }
                Err(_) => continue, // 写一半（临时文件 rename 前），等下一轮
            }
        }
    });
    rx
}

/// 地图变更监视：1s 轮询内容哈希，变化即上报（返回 watch::Receiver，消费者订阅）
pub fn spawn_map_watcher(
    service: Arc<MapService>,
    repo: Repo,
) -> watch::Receiver<Result<MapSnapshot, String>> {
    let (tx, rx) = watch::channel::<Result<MapSnapshot, String>>(Err("尚未加载".into()));
    tokio::spawn(async move {
        let mut last: Option<u64> = None;
        let mut interval = tokio::time::interval(Duration::from_millis(1000));
        loop {
            interval.tick().await;
            let raw = match tokio::fs::read_to_string(repo.map_path()).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            let hash = content_hash(&raw);
            if last == Some(hash) { continue; }
            match service.load_map(&repo).await {
                Ok(snap) => {
                    last = Some(hash);
                    info!("[watch] {} map.changed", repo.id);
                    let _ = tx.send(Ok(snap));
                }
                Err(e) => {
                    last = Some(hash);
                    warn!("[watch] {} map.invalid: {e}", repo.id);
                    let _ = tx.send(Err(e.to_string()));
                }
            }
        }
    });
    rx
}

/// 便于测试：从路径构造（不读文件）
pub fn repo_from_root(root: &Path) -> Repo {
    let name = root.file_name().and_then(|n| n.to_str()).unwrap_or("repo").to_string();
    Repo { id: name.clone(), name, root: root.to_path_buf() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_minimum_rejects_incomplete() {
        let svc = MapService::new(vec![]);
        let bad = serde_json::json!({"version": "1.0"});
        assert!(svc.check_minimum(&bad).is_err());
        let good = serde_json::json!({
            "version": "1.0", "meta": {}, "layers": [{"id": "l1"}],
            "modules": [{"id": "m1", "layer": "l1", "dependencies": [],
                "health": {"coupling": "low", "complexity": "low", "churn": "low"}}],
            "edges": [{"id": "e1", "from": "m1", "to": "m1", "type": "call", "strength": "weak"}],
            "health": {}
        });
        assert!(svc.check_minimum(&good).is_ok());
        // 结构级对 v1.0 旧数据（无 edge id）保持可读
        let mut legacy = good.clone();
        legacy["edges"] = serde_json::json!([{"from": "m1", "to": "m1", "type": "call", "strength": "weak"}]);
        assert!(svc.check_minimum(&legacy).is_ok());
        // 严格验收（写路径）：重复边 id / 悬空边 / 非法枚举 / 未知 layer 必须被拒
        let mut dup = good.clone();
        dup["edges"].as_array_mut().unwrap().push(good["edges"][0].clone());
        assert!(validate_strict(&dup).is_err());
        let mut dangling = good.clone();
        dangling["edges"][0]["to"] = serde_json::json!("ghost");
        assert!(validate_strict(&dangling).is_err());
        let mut bad_enum = good.clone();
        bad_enum["modules"][0]["health"]["coupling"] = serde_json::json!("extreme");
        assert!(validate_strict(&bad_enum).is_err());
        let mut bad_layer = good.clone();
        bad_layer["modules"][0]["layer"] = serde_json::json!("nope");
        assert!(validate_strict(&bad_layer).is_err());
        assert!(validate_strict(&legacy).is_err()); // 无 edge id 的旧数据不能通过写验收
    }

    #[tokio::test]
    async fn submap_path_traversal_rejected() {
        // is_valid_id 防线（审查 🔴1 的回归测试）
        assert!(is_valid_id("exam-core"));
        assert!(!is_valid_id("../etc/passwd"));
        assert!(!is_valid_id("..%2F..%2Fetc"));
        assert!(!is_valid_id("a/b"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id("1abc"));
    }

    #[test]
    fn content_hash_stable() {
        assert_eq!(content_hash("abc"), content_hash("abc"));
        assert_ne!(content_hash("abc"), content_hash("abd"));
    }

    #[test]
    fn fixtures_pilot_map_passes_strict_validation() {
        // §7 契约回归：fixtures 是提示词试点的验收产物（v1.1 全字段），
        // Rust 校验器消费它——格式演进（edge id / meta.stats / 封闭枚举）有真实回归网
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../fixtures/hover-client-v2.1-pilot/expected/map.json");
        let raw = std::fs::read_to_string(&path).expect("fixtures map.json 不可读（提示词试点验收产物）");
        let map: Value = serde_json::from_str(&raw).expect("fixtures map.json 非法 JSON");
        validate_strict(&map).expect("fixtures 试点地图必须过严格验收（v1.1）");
        // 读路径底线：存量地图 minimum 必过
        let svc = MapService::new(vec![]);
        assert!(svc.check_minimum(&map).is_ok());
    }
}
