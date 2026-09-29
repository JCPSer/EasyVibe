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

/// 独立轻量自检（不依赖 MapService 实例；PatrolService 等复用）
pub fn validate_minimum(json: &Value) -> Result<(), ApiError> {
    let obj = json.as_object().ok_or_else(|| ApiError::MapInvalid("根节点必须是对象".into()))?;
    for key in ["version", "meta", "layers", "modules", "edges", "health"] {
        if !obj.contains_key(key) {
            return Err(ApiError::MapInvalid(format!("缺少必填字段 {key}")));
        }
    }
    if obj["modules"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
        return Err(ApiError::MapInvalid("modules 不能为空".into()));
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
            let new_part = if reset { raw.as_str() } else { &raw[offset as usize..] };
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
            "version": "1.0", "meta": {}, "layers": [], "modules": [{"id": "m"}],
            "edges": [], "health": {}
        });
        assert!(svc.check_minimum(&good).is_ok());
    }

    #[test]
    fn content_hash_stable() {
        assert_eq!(content_hash("abc"), content_hash("abc"));
        assert_ne!(content_hash("abc"), content_hash("abd"));
    }
}
