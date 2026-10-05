//! 小步增量归纳（B 方案）的合成端：agent 只产出结构化 patch JSON，本模块负责
//! 归属引擎（files glob → 模块）、patch 解析、合成、确定性校验、candidate 原子落盘，
//! 以及归纳锚点（induction-state.json）读写。
//!
//! 设计红线（与「小步增量归纳」提示词的纪律一一对应）：
//! - 禁止删除任何旧模块（越权即拒）；
//! - patch 里出现的每个模块必须真的 affected（diff 文件按 CURRENT_MAP 的 files glob 归属，
//!   一个文件匹配多个模块时所有这些模块都算 affected——与 prompt 白名单同一规则）；
//! - meta.generated_at 由后端写当前时间（freshness 归零的唯一机制，不信 agent）；
//! - 校验全过才 atomic rename 成 map.json，失败删 candidate、旧图保持原样。
use easyvibe_common::ApiError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// agent 增量产物的落盘位置（agent 写它，绝不直接写 map.json）
pub fn patch_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".easyvibe/map/map.patch.json")
}

/// 合成产物的 candidate（校验通过才 rename 成 map.json）
pub fn candidate_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".easyvibe/map/map.json.candidate")
}

/// 归纳锚点：spawn 决策时用于 diff 的 HEAD。成功后推进；HEAD 漂移不推进
/// （下次 range 自然覆盖漏掉的提交，超阈值转全量——锚点吞提交盲区的修复）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InductionState {
    pub head_sha: String,
    pub mode: String,
    pub completed_at: String,
}

pub fn induction_state_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".easyvibe/map/induction-state.json")
}

/// head_sha 合法性：非空十六进制、7~64 位（rev-parse 短/长形均可作为锚点）
fn valid_sha(s: &str) -> bool {
    (7..=64).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// 读归纳锚点；不存在或非法（含 head_sha 缺失/非 hex——rebase 改写后的人工残留）→ None → 调用方转全量
pub fn read_induction_state(repo_root: &Path) -> Option<InductionState> {
    let raw = std::fs::read_to_string(induction_state_path(repo_root)).ok()?;
    let st: InductionState = serde_json::from_str(&raw).ok()?;
    if !valid_sha(&st.head_sha) {
        return None;
    }
    Some(st)
}

/// 写归纳锚点（临时文件 + rename，与 map.json 同一原子纪律）
pub fn write_induction_state(repo_root: &Path, state: &InductionState) -> Result<(), ApiError> {
    let dir = repo_root.join(".easyvibe/map");
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::Internal(format!("map 目录创建失败: {e}")))?;
    let tmp = dir.join("induction-state.json.tmp");
    let bytes = serde_json::to_vec_pretty(state).map_err(|e| ApiError::Internal(e.to_string()))?;
    std::fs::write(&tmp, bytes).map_err(|e| ApiError::Internal(format!("锚点临时文件写入失败: {e}")))?;
    std::fs::rename(&tmp, induction_state_path(repo_root)).map_err(|e| ApiError::Internal(format!("锚点原子替换失败: {e}")))?;
    Ok(())
}

// ---------- 归属引擎：files glob → 模块 ----------

/// glob 语义从简：`*` 段内任意字符（不含 `/`）、`?` 单字符（不含 `/`）、
/// `**` 跨段（匹配零个或多个完整路径段）。模式与路径均为 `/` 分隔的仓库相对路径。
pub fn glob_match(pattern: &str, path: &str) -> bool {
    let pats: Vec<&str> = pattern.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    match_segments(&pats, &segs)
}

fn match_segments(pats: &[&str], segs: &[&str]) -> bool {
    match pats.split_first() {
        None => segs.is_empty(),
        Some((pat, rest)) => {
            if *pat == "**" {
                // `**` 匹配零个或多个完整段
                return (0..=segs.len()).any(|i| match_segments(rest, &segs[i..]));
            }
            let Some((seg, segs_rest)) = segs.split_first() else { return false };
            seg_match(pat, seg) && match_segments(rest, segs_rest)
        }
    }
}

/// 单段匹配：`*` 任意字符（不含 `/` 已由 split 保证）、`?` 单字符
fn seg_match(pat: &str, seg: &str) -> bool {
    fn rec(p: &[u8], s: &[u8]) -> bool {
        match p.split_first() {
            None => s.is_empty(),
            Some((b'*', rest)) => (0..=s.len()).any(|i| rec(rest, &s[i..])),
            Some((b'?', rest)) => !s.is_empty() && rec(rest, &s[1..]),
            Some((&c, rest)) => !s.is_empty() && s[0] == c && rec(rest, &s[1..]),
        }
    }
    rec(pat.as_bytes(), seg.as_bytes())
}

/// 归属裁决：diff 文件 → 所有 files glob 匹配的模块 id（多匹配全部 affected——
/// 确定性规则，prompt 白名单与后端校验共用）。不属于任何现有模块的文件返回空集，
/// 由调用方按"允许新建模块"处理。
pub fn owners_of_file<'m>(map: &'m Value, path: &str) -> Vec<&'m str> {
    map["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| {
            m["files"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|g| g.as_str())
                .any(|g| glob_match(g, path))
        })
        .filter_map(|m| m["id"].as_str())
        .collect()
}

/// affected 集合：diff 中全部文件按归属引擎展开（含多匹配裁决）。
pub fn affected_modules(map: &Value, diff_paths: &[String]) -> BTreeSet<String> {
    diff_paths
        .iter()
        .flat_map(|p| owners_of_file(map, p))
        .map(str::to_string)
        .collect()
}

// ---------- patch 解析 ----------

/// agent 产出的结构化 patch（写进 map.patch.json 的契约形状）
#[derive(Debug, Clone)]
pub struct IncrementalPatch {
    pub new_modules: Vec<Value>,
    pub updated_modules: Vec<Value>,
    pub new_edges: Vec<Value>,
    pub removed_edge_ids: Vec<String>,
}

pub fn parse_patch(raw: &str) -> Result<IncrementalPatch, ApiError> {
    let v: Value = serde_json::from_str(raw).map_err(|e| ApiError::MapInvalid(format!("patch 不是合法 JSON: {e}")))?;
    let obj = v.as_object().ok_or_else(|| ApiError::MapInvalid("patch 根节点必须是对象".into()))?;
    let modules = |key: &str| -> Result<Vec<Value>, ApiError> {
        match obj.get(key) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(a)) => {
                for m in a {
                    if m["id"].as_str().is_none() {
                        return Err(ApiError::MapInvalid(format!("patch.{key} 含无 id 模块")));
                    }
                }
                Ok(a.clone())
            }
            Some(_) => Err(ApiError::MapInvalid(format!("patch.{key} 必须是数组"))),
        }
    };
    let removed_edge_ids = match obj.get("removed_edge_ids") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|e| e.as_str().map(str::to_string).ok_or_else(|| ApiError::MapInvalid("patch.removed_edge_ids 必须是字符串数组".into())))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(ApiError::MapInvalid("patch.removed_edge_ids 必须是数组".into())),
    };
    Ok(IncrementalPatch {
        new_modules: modules("new_modules")?,
        updated_modules: modules("updated_modules")?,
        new_edges: {
            match obj.get("new_edges") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(a)) => a.clone(),
                Some(_) => return Err(ApiError::MapInvalid("patch.new_edges 必须是数组".into())),
            }
        },
        removed_edge_ids,
    })
}

// ---------- 合成 ----------

/// 以旧图深拷贝为底 splice：updated/new 按 id 替换/插入；removed_edge_ids 删边
/// （id 不存在 = prompt 违规，判失败）；new_edges 追加（端点必须已存在）。
/// meta.generated_at 由调用方（后端）覆写为当前时间——freshness 归零的唯一机制。
pub fn synthesize(old: &Value, patch: &IncrementalPatch, now_iso: &str) -> Result<Value, ApiError> {
    let mut new = old.clone();
    let modules = new["modules"].as_array_mut().ok_or_else(|| ApiError::MapInvalid("旧图 modules 缺失".into()))?;
    let mut ids: BTreeSet<String> = modules.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect();
    for m in patch.updated_modules.iter().chain(patch.new_modules.iter()) {
        let id = m["id"].as_str().unwrap_or_default().to_string();
        match modules.iter_mut().find(|x| x["id"].as_str() == Some(id.as_str())) {
            Some(slot) => *slot = m.clone(),
            None => {
                modules.push(m.clone());
                ids.insert(id);
            }
        }
    }
    // 边：删（id 必须存在）+ 增（端点必须存在）
    let edges = new["edges"].as_array_mut().ok_or_else(|| ApiError::MapInvalid("旧图 edges 缺失".into()))?;
    for rid in &patch.removed_edge_ids {
        let before = edges.len();
        edges.retain(|e| e["id"].as_str() != Some(rid.as_str()));
        if edges.len() == before {
            return Err(ApiError::MapInvalid(format!("patch 请求删除不存在的边 {rid}（prompt 违规）")));
        }
    }
    for e in &patch.new_edges {
        let id = e["id"].as_str().unwrap_or("?").to_string();
        for ep in ["from", "to"] {
            let v = e[ep].as_str().ok_or_else(|| ApiError::MapInvalid(format!("新边 {id} 缺少 {ep}")))?;
            if !ids.contains(v) {
                return Err(ApiError::MapInvalid(format!("新边 {id} 的 {ep}={v} 不存在")));
            }
        }
        edges.push(e.clone());
    }
    // meta.generated_at 后端覆写（不信任 agent 提供的任何时间）
    let meta = new.get_mut("meta").and_then(Value::as_object_mut).ok_or_else(|| ApiError::MapInvalid("旧图 meta 缺失".into()))?;
    meta.insert("generated_at".into(), Value::String(now_iso.into()));
    Ok(new)
}

// ---------- 增量附加校验 ----------

/// 全过才落盘：
/// ① 旧模块 id 集合 ⊆ 新集合（禁删）；
/// ② patch 里每个 updated 模块必须 ∈ affected 且为旧模块；new 模块 id 不得与旧 id 重名；
/// ③ affected 之外的模块与旧版逐字段一致（构造保证，断言式复核）；
/// ④ generated_at 不早于旧值（parse_ts_like 解析，容许同秒）。
pub fn validate_incremental(old: &Value, new: &Value, patch: &IncrementalPatch, affected: &BTreeSet<String>) -> Result<(), ApiError> {
    let old_ids: BTreeSet<&str> = old["modules"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str()).collect();
    let new_ids: BTreeSet<&str> = new["modules"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str()).collect();
    for id in &old_ids {
        if !new_ids.contains(id) {
            return Err(ApiError::MapInvalid(format!("增量 patch 删除了旧模块 {id}（禁止删除）")));
        }
    }
    for m in &patch.updated_modules {
        let id = m["id"].as_str().unwrap_or("?");
        if !old_ids.contains(id) {
            return Err(ApiError::MapInvalid(format!("patch.updated_modules 的 {id} 不是旧模块（新建请用 new_modules）")));
        }
        if !affected.contains(id) {
            return Err(ApiError::MapInvalid(format!("patch 更新了非 affected 模块 {id}（越权）")));
        }
    }
    for m in &patch.new_modules {
        let id = m["id"].as_str().unwrap_or("?");
        if old_ids.contains(id) {
            return Err(ApiError::MapInvalid(format!("patch.new_modules 的 {id} 与旧模块重名")));
        }
    }
    // affected 之外的模块必须与旧版逐字段一致（合成路径上必然成立；断言式复核防未来回归）
    for m in new["modules"].as_array().into_iter().flatten() {
        let id = m["id"].as_str().unwrap_or("?");
        if affected.contains(id) {
            continue;
        }
        let old_m = old["modules"].as_array().into_iter().flatten().find(|x| x["id"].as_str() == Some(id));
        if old_m != Some(m) {
            return Err(ApiError::MapInvalid(format!("非 affected 模块 {id} 与旧版不一致")));
        }
    }
    // generated_at 单调（容许同秒——同秒内的连续归纳合法）
    let old_ts = old["meta"]["generated_at"].as_str().and_then(easyvibe_common::parse_ts_like);
    let new_ts = new["meta"]["generated_at"].as_str().and_then(easyvibe_common::parse_ts_like);
    match (old_ts, new_ts) {
        (Some(o), Some(n)) if n >= o => {}
        (Some(_), Some(_)) => return Err(ApiError::MapInvalid("generated_at 早于旧值（时钟回拨？）".into())),
        _ => return Err(ApiError::MapInvalid("generated_at 不可解析（后端写入应为 ISO-8601）".into())),
    }
    crate::validate_strict(new)
}

// ---------- candidate 落盘（端到端一步） ----------

/// 增量终态收尾：读 patch → 归属 → 合成 → 校验 → candidate → atomic rename。
/// 任一步失败：删 candidate（若已写），map.json 保持旧样，Err 交调用方走全量回退。
pub async fn apply_incremental(repo_root: &Path, diff_paths: &[String], now_iso: &str) -> Result<(), ApiError> {
    let map_path = repo_root.join(".easyvibe/map/map.json");
    let raw_old = tokio::fs::read_to_string(&map_path)
        .await
        .map_err(|e| ApiError::NotFound(format!("增量合成需要旧 map.json: {e}")))?;
    let old: Value = serde_json::from_str(&raw_old).map_err(|e| ApiError::MapInvalid(format!("旧 map.json 解析失败: {e}")))?;
    let patch_raw = tokio::fs::read_to_string(patch_path(repo_root))
        .await
        .map_err(|e| ApiError::NotFound(format!("patch 缺失（agent 未写 map.patch.json）: {e}")))?;
    let patch = parse_patch(&patch_raw)?;
    let affected = affected_modules(&old, diff_paths);
    let synthesized = synthesize(&old, &patch, now_iso)?;
    validate_incremental(&old, &synthesized, &patch, &affected)?;
    let candidate = candidate_path(repo_root);
    crate::atomic_write_json(&candidate, &synthesized).await?;
    // 全量校验已过（validate_incremental 收尾即 validate_strict），rename 换正
    if let Err(e) = tokio::fs::rename(&candidate, &map_path).await {
        let _ = tokio::fs::remove_file(&candidate).await;
        return Err(ApiError::Internal(format!("candidate 原子替换失败: {e}")));
    }
    Ok(())
}

/// patch 终态后清理（坏 patch 不留在账簿里混淆下次判定；成功路径已由 rename 消费）
pub async fn remove_patch(repo_root: &Path) {
    let _ = tokio::fs::remove_file(patch_path(repo_root)).await;
    let _ = tokio::fs::remove_file(candidate_path(repo_root)).await;
}

/// 便于测试的地图构造（最小合法 strict 形状）
#[cfg(test)]
pub(crate) fn test_map() -> Value {
    serde_json::json!({
        "version": "1.0",
        "meta": {"generated_at": "2026-01-01T00:00:00Z", "repo_profile": {"tech_stack": ["rust"]}},
        "layers": [{"id": "foundation", "order": 0}],
        "modules": [
            {"id": "core", "name": "核心", "layer": "foundation", "files": ["src/core/**/*.rs", "src/main.rs"],
             "key_entries": [], "responsibility": "核心", "dependencies": [],
             "health": {"score": 90, "coupling": "low", "complexity": "low", "churn": "low", "decay_flags": [], "review_note": "ok", "concerns": []}},
            {"id": "util", "name": "工具", "layer": "foundation", "files": ["src/util/**"],
             "key_entries": [], "responsibility": "工具", "dependencies": [],
             "health": {"score": 90, "coupling": "low", "complexity": "low", "churn": "low", "decay_flags": [], "review_note": "ok", "concerns": []}}
        ],
        "edges": [{"id": "e1", "from": "core", "to": "util", "type": "call", "strength": "weak"}],
        "health": {"score": 90, "concerns": []}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- glob 归属引擎 ----------

    #[test]
    fn glob_single_and_nested_match() {
        assert!(glob_match("src/core/**/*.rs", "src/core/a.rs"));
        assert!(glob_match("src/core/**/*.rs", "src/core/deep/b.rs"), "** 跨多层");
        assert!(!glob_match("src/core/**/*.rs", "src/other/a.rs"));
        assert!(glob_match("src/main.rs", "src/main.rs"));
        assert!(!glob_match("src/main.rs", "src/main2.rs"));
        assert!(glob_match("src/util/**", "src/util/x/y.txt"), "尾段 ** 吃零段或多段");
        assert!(glob_match("src/util/**", "src/util/a.txt"));
        assert!(!glob_match("src/util/**", "src/utilx/a.txt"), "段边界必须精确");
        assert!(glob_match("src/?.rs", "src/a.rs"));
        assert!(!glob_match("src/?.rs", "src/ab.rs"));
        assert!(glob_match("**/*.rs", "any/where/file.rs"), "前缀 ** 匹配任意深度");
    }

    #[test]
    fn ownership_single_multi_and_none() {
        let map = test_map();
        // 单匹配
        assert_eq!(owners_of_file(&map, "src/core/thing.rs"), vec!["core"]);
        // 多匹配裁决：一个文件命中两个模块的 glob → 全部 affected
        let mut m = test_map();
        m["modules"][1]["files"] = serde_json::json!(["src/**"]);
        let owners = owners_of_file(&m, "src/core/thing.rs");
        assert_eq!(owners.len(), 2, "多匹配全部算 affected: {owners:?}");
        // 无匹配（允许新建模块的文件）
        assert!(owners_of_file(&map, "docs/readme.md").is_empty());
    }

    #[test]
    fn affected_set_unions_all_owners() {
        let map = test_map();
        let affected = affected_modules(&map, &["src/core/a.rs".into(), "src/util/b.rs".into()]);
        assert_eq!(affected, BTreeSet::from(["core".to_string(), "util".to_string()]));
    }

    // ---------- patch 解析 ----------

    #[test]
    fn parse_patch_tolerates_empty_and_rejects_garbage() {
        let p = parse_patch("{}").unwrap();
        assert!(p.new_modules.is_empty() && p.updated_modules.is_empty() && p.new_edges.is_empty() && p.removed_edge_ids.is_empty());
        assert!(parse_patch("not json").is_err());
        assert!(parse_patch("{\"updated_modules\": [{\"noid\": 1}]}").is_err(), "无 id 模块必须拒");
        assert!(parse_patch("{\"new_modules\": 42}").is_err());
    }

    // ---------- 合成 ----------

    #[test]
    fn synthesize_splices_and_overwrites_generated_at() {
        let old = test_map();
        let patch = IncrementalPatch {
            updated_modules: vec![{
                let mut m = old["modules"][0].clone();
                m["responsibility"] = "核心（已演进）".into();
                m
            }],
            new_modules: vec![serde_json::json!({
                "id": "new-mod", "name": "新", "layer": "foundation", "files": ["new/**"],
                "key_entries": [], "responsibility": "新", "dependencies": [],
                "health": {"score": 80, "coupling": "low", "complexity": "low", "churn": "low", "decay_flags": [], "review_note": "ok", "concerns": []}
            })],
            new_edges: vec![serde_json::json!({"id": "e2", "from": "core", "to": "new-mod", "type": "call", "strength": "weak"})],
            removed_edge_ids: vec!["e1".into()],
        };
        let now = "2026-06-01T00:00:00Z";
        let new = synthesize(&old, &patch, now).unwrap();
        assert_eq!(new["meta"]["generated_at"], now, "generated_at 必须由后端覆写");
        assert_eq!(new["modules"].as_array().unwrap().len(), 3);
        assert_eq!(new["modules"][0]["responsibility"], "核心（已演进）");
        assert_eq!(new["modules"][2]["id"], "new-mod");
        assert_eq!(new["edges"].as_array().unwrap().len(), 1, "删 e1 加 e2");
        assert_eq!(new["edges"][0]["id"], "e2");
        // 旧图不被污染（深拷贝为底）
        assert_eq!(old["modules"].as_array().unwrap().len(), 2);
        assert_eq!(old["meta"]["generated_at"], "2026-01-01T00:00:00Z");
    }

    #[test]
    fn synthesize_rejects_unknown_edge_removal_and_dangling_endpoint() {
        let old = test_map();
        let bad_rm = IncrementalPatch {
            removed_edge_ids: vec!["ghost".into()],
            ..parse_patch("{}").unwrap()
        };
        assert!(synthesize(&old, &bad_rm, "2026-06-01T00:00:00Z").is_err(), "删不存在的边 = prompt 违规");
        let bad_ep = IncrementalPatch {
            new_edges: vec![serde_json::json!({"id": "e9", "from": "core", "to": "ghost", "type": "call", "strength": "weak"})],
            ..parse_patch("{}").unwrap()
        };
        assert!(synthesize(&old, &bad_ep, "2026-06-01T00:00:00Z").is_err(), "未知边端点必须拒");
    }

    // ---------- 增量附加校验 ----------

    fn affected_set(map: &Value, paths: &[&str]) -> BTreeSet<String> {
        affected_modules(map, &paths.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn validate_rejects_unauthorized_update() {
        let old = test_map();
        // diff 只动 src/util/**，patch 却更新 core → 越权
        let patch = IncrementalPatch {
            updated_modules: vec![old["modules"][0].clone()],
            ..parse_patch("{}").unwrap()
        };
        let affected = affected_set(&old, &["src/util/x.rs"]);
        let new = synthesize(&old, &patch, "2026-06-01T00:00:00Z").unwrap();
        assert!(validate_incremental(&old, &new, &patch, &affected).is_err(), "非 affected 更新必须拒");
    }

    #[test]
    fn validate_accepts_legit_update_and_checks_untouched_identical() {
        let old = test_map();
        let patch = IncrementalPatch {
            updated_modules: vec![{
                let mut m = old["modules"][0].clone();
                m["health"]["score"] = 70.into();
                m
            }],
            ..parse_patch("{}").unwrap()
        };
        let affected = affected_set(&old, &["src/core/a.rs"]);
        let new = synthesize(&old, &patch, "2026-06-01T00:00:00Z").unwrap();
        validate_incremental(&old, &new, &patch, &affected).expect("合法 affected 更新必须通过");
    }

    #[test]
    fn validate_rejects_module_deletion_and_new_id_collision() {
        let old = test_map();
        // 构造删模块的新图（合成器不会产出，校验必须能拒）
        let mut new = old.clone();
        new["modules"].as_array_mut().unwrap().retain(|m| m["id"] != "util");
        let patch = parse_patch("{}").unwrap();
        let affected = affected_set(&old, &["src/core/a.rs"]);
        assert!(validate_incremental(&old, &new, &patch, &affected).is_err(), "旧模块消失必须拒");
        // new_modules 与旧 id 重名
        let patch2 = IncrementalPatch {
            new_modules: vec![old["modules"][1].clone()],
            ..parse_patch("{}").unwrap()
        };
        let new2 = synthesize(&old, &patch2, "2026-06-01T00:00:00Z").unwrap();
        assert!(validate_incremental(&old, &new2, &patch2, &affected).is_err(), "重名新模块必须拒");
    }

    #[test]
    fn validate_generated_at_monotonic_same_second_ok() {
        let old = test_map();
        let patch = parse_patch("{}").unwrap();
        let affected = affected_set(&old, &["src/core/a.rs"]);
        // 时间早于旧值 → 拒
        let new = synthesize(&old, &patch, "2025-01-01T00:00:00Z").unwrap();
        assert!(validate_incremental(&old, &new, &patch, &affected).is_err());
        // 同秒 → 容许
        let new2 = synthesize(&old, &patch, "2026-01-01T00:00:00Z").unwrap();
        validate_incremental(&old, &new2, &patch, &affected).expect("同秒容许");
    }

    // ---------- 锚点 ----------

    #[test]
    fn induction_state_roundtrip_and_validation() {
        let dir = std::env::temp_dir().join(format!("ev-induction-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".easyvibe/map")).unwrap();
        // 不存在 → None（走全量）
        assert!(read_induction_state(&dir).is_none());
        let st = InductionState {
            head_sha: "abc1234".into(),
            mode: "full".into(),
            completed_at: "2026-10-01T00:00:00Z".into(),
        };
        write_induction_state(&dir, &st).unwrap();
        assert_eq!(read_induction_state(&dir), Some(st.clone()));
        // 非法 head_sha（人工改坏 / rebase 残留）→ None → 转全量
        std::fs::write(dir.join(".easyvibe/map/induction-state.json"), "{\"head_sha\": \"not a sha!!\", \"mode\": \"full\", \"completed_at\": \"x\"}").unwrap();
        assert!(read_induction_state(&dir).is_none());
        std::fs::write(dir.join(".easyvibe/map/induction-state.json"), "{\"mode\": \"full\"}").unwrap();
        assert!(read_induction_state(&dir).is_none(), "缺 head_sha 必须转全量");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- candidate 端到端 ----------

    #[tokio::test]
    async fn apply_incremental_happy_path_and_failure_keeps_old() {
        let dir = std::env::temp_dir().join(format!("ev-apply-incremental-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".easyvibe/map")).unwrap();
        let old = test_map();
        crate::atomic_write_json(&dir.join(".easyvibe/map/map.json"), &old).await.unwrap();

        // 合法 patch：更新 core（diff 归属 core）
        let mut m = old["modules"][0].clone();
        m["health"]["score"] = 66.into();
        let patch = serde_json::json!({"updated_modules": [m], "removed_edge_ids": ["e1"]});
        std::fs::write(dir.join(".easyvibe/map/map.patch.json"), patch.to_string()).unwrap();
        apply_incremental(&dir, &["src/core/thing.rs".into()], "2026-06-01T00:00:00Z").await.unwrap();
        let after: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(".easyvibe/map/map.json")).unwrap()).unwrap();
        assert_eq!(after["modules"][0]["health"]["score"], 66);
        assert_eq!(after["edges"].as_array().unwrap().len(), 0, "e1 已删");
        assert!(!candidate_path(&dir).exists(), "candidate 必须已 rename 消费");

        // 坏 patch（越权更新 util，diff 不含 util 文件）→ 失败且旧图保持
        let mut m2 = after["modules"][1].clone();
        m2["health"]["score"] = 10.into();
        let bad = serde_json::json!({"updated_modules": [m2]});
        std::fs::write(dir.join(".easyvibe/map/map.patch.json"), bad.to_string()).unwrap();
        assert!(apply_incremental(&dir, &["src/core/thing.rs".into()], "2026-06-02T00:00:00Z").await.is_err());
        let kept: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(".easyvibe/map/map.json")).unwrap()).unwrap();
        assert_eq!(kept["modules"][1]["health"]["score"], 90, "旧图必须原样保留");
        assert!(!candidate_path(&dir).exists(), "失败路径 candidate 必须清理");

        // patch 缺失 → NotFound（调用方转全量回退）
        std::fs::remove_file(dir.join(".easyvibe/map/map.patch.json")).unwrap();
        assert!(matches!(apply_incremental(&dir, &[], "2026-06-03T00:00:00Z").await, Err(ApiError::NotFound(_))));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
