//! 增量/全量归纳的终态收尾（自 `reinduce.rs` 的终态段原样搬迁，零语义改动）。
//!
//! 判定与渲染（纯函数）已上移 `easyvibe-map::induction`；本层只保留吃 `&AppState` 的编排。
//! 三条语义防线原样保留：① 阈值规则不在本文件；② `decision_head` 单次读、终态只接收不重读；
//! ③ `force_full` 防无限环 + strict 失败还原旧图不进重试链。

use easyvibe_event_bus::queue::{JobKind, QueuedJob};
use easyvibe_map::induction::{head_sha, should_advance_anchor, IncrementalContext};


/// 锚点推进：HEAD 未漂移才写 induction-state（归纳中用户提交 → 不推进，超阈值自然转全量）
async fn advance_anchor(root: &std::path::Path, decision_head: &Option<String>, mode: &str) {
    let Some(sha) = should_advance_anchor(decision_head, head_sha(root).as_deref()) else {
        tracing::warn!("[reinduce] 决策后 HEAD 已漂移（或 git 不可用）——不推进归纳锚点（下次 range 覆盖漏掉的提交）");
        return;
    };
    let state = easyvibe_map::synthesis::InductionState {
        head_sha: sha,
        mode: mode.into(),
        completed_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Err(e) = easyvibe_map::synthesis::write_induction_state(root, &state) {
        tracing::warn!("[reinduce] 归纳锚点写入失败（不影响本次产物）: {e}");
    }
}

/// 增量 patch 校验失败 → 入队一个强制全量的归纳任务（R3 契约修复：走队列入库，
/// 前端 SessionBubble/运行页自然可见，不静默重试；force_full 防增量无限环）。
/// 本任务终态触发的 drain 已先于此执行，入队项由 12s 清扫器兜底排空。
async fn enqueue_full_fallback(st: &crate::state::AppState, repo_id: &str) {
    let job = QueuedJob {
        kind: JobKind::Reinduce,
        module_id: None,
        label: "归纳".into(),
        enqueued_at: chrono::Utc::now(),
        force_full: true,
    };
    st.session_queue.enqueue_or_replace(st, repo_id, job).await;
}

/// 增量会话终态收尾：读 patch → 归属 → 合成 → 校验 → candidate 原子落盘；
/// 任一步失败 → 清理坏 patch + 入队全量回退（全量再失败才按现状语义报错给用户）。
/// 会话 succeeded 但 patch 缺失/坏/越权都走失败分支（无人值守纪律：不留半成品）。
pub(crate) async fn handle_incremental_terminal(
    st: &crate::state::AppState,
    repo: &easyvibe_map::Repo,
    ctx: &IncrementalContext,
    decision_head: &Option<String>,
) {
    let now = chrono::Utc::now().to_rfc3339();
    match easyvibe_map::synthesis::apply_incremental(&repo.root, &ctx.diff_paths, &now).await {
        Ok(()) => {
            tracing::info!(
                "[reinduce] 增量合成落盘成功（锚点 {}..{}，{} 个 diff 文件）",
                ctx.prev_anchor,
                ctx.head,
                ctx.diff_paths.len()
            );
            advance_anchor(&repo.root, decision_head, "incremental").await;
        }
        Err(e) => {
            tracing::warn!("[reinduce] 增量 patch 不可用（{e}）——保留旧图，入队全量回退");
            easyvibe_map::synthesis::remove_patch(&repo.root).await;
            enqueue_full_fallback(st, &repo.id).await;
        }
    }
}

/// 全量会话终态收尾（评审 M3 缺口收口）：产物必须过 validate_strict——
/// 失败 = 产物非法 = 还原旧图 + 告警，**不进自动重试链**（坏产物诱导无限重试）；
/// 通过才推进归纳锚点。spawn 失败（CLI 缺失）在 start_induction 处直接报错，不入队（现状语义）。
pub(crate) async fn handle_full_terminal(
    st: &crate::state::AppState,
    repo: &easyvibe_map::Repo,
    snap_before: &Option<easyvibe_map::MapSnapshot>,
    decision_head: &Option<String>,
) {
    match st.map_service.load_map(repo).await {
        Ok(snap) => {
            if let Err(e) = easyvibe_map::validate_strict(&snap.json) {
                tracing::warn!("[reinduce] 全量产物未过严格校验（{e}）——保留旧图（strict 失败不进自动重试链）");
                if let Some(before) = snap_before {
                    if let Err(re) = easyvibe_map::atomic_write_json(&repo.map_path(), &before.json).await {
                        tracing::warn!("[reinduce] 旧图还原写回失败: {re}");
                    }
                }
                return; // 产物非法：不推进锚点
            }
            advance_anchor(&repo.root, decision_head, "full").await;
        }
        Err(e) => tracing::warn!("[reinduce] 终态后读图失败（agent 未写 map.json？）: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    // ---------- 终态收尾集成（临时 git 仓库 + test_state 夹具） ----------

    // 测试夹具（`easyvibe-map::induction` 的纯判定测试有同形副本——跨 crate 的 cfg(test) 项不可见）
    struct TempRepo {
        dir: std::path::PathBuf,
    }
    impl TempRepo {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&dir).status().unwrap().success());
            git(&["init", "-q"]);
            git(&["config", "user.email", "t@t"]);
            git(&["config", "user.name", "t"]);
            Self { dir }
        }
        fn commit(&self, msg: &str) -> String {
            std::fs::write(self.dir.join("a.txt"), format!("{msg}\n")).unwrap();
            let git = |args: &[&str]| assert!(std::process::Command::new("git").args(args).current_dir(&self.dir).status().unwrap().success());
            git(&["add", "."]);
            git(&["commit", "-q", "-m", msg]);
            head_sha(&self.dir).unwrap()
        }
    }
    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
    
    // ---------- 终态收尾集成（临时 git 仓库 + test_state 夹具） ----------
    
    /// 最小 strict 合法地图（generated_at 可解析——增量校验的时钟单调前提）。
    /// 以 JSON 文本常量表达（本文件在 `contract_guard` 的内联构造棘轮扫描面内，禁止内联字面量构造）。
    const STRICT_MAP_JSON: &str = r#"{
        "version": "1.0",
        "meta": {"generated_at": "2026-01-01T00:00:00Z"},
        "layers": [{"id": "foundation", "order": 0}],
        "modules": [
            {"id": "core", "layer": "foundation", "files": ["src/core/**"], "dependencies": [],
             "health": {"coupling": "low", "complexity": "low", "churn": "low"}}
        ],
        "edges": [{"id": "e1", "from": "core", "to": "core", "type": "call", "strength": "weak"}],
        "health": {}
    }"#;

    fn strict_map() -> Value {
        serde_json::from_str(STRICT_MAP_JSON).expect("测试夹具 JSON 必须合法")
    }
    
    async fn state_with_repo(root: &std::path::Path) -> crate::state::AppState {
        let svc = easyvibe_map::MapService::new(vec![easyvibe_map::repo_from_root(root)]);
        crate::test_support::test_state_with(svc).await
    }
    
    #[tokio::test]
    async fn full_terminal_advances_anchor_and_head_drift_blocks_it() {
        let repo = TempRepo::new("ev-full-terminal");
        repo.commit("init");
        std::fs::create_dir_all(repo.dir.join(".easyvibe/map")).unwrap();
        let map = strict_map();
        easyvibe_map::atomic_write_json(&repo.dir.join(".easyvibe/map/map.json"), &map).await.unwrap();
        let st = state_with_repo(&repo.dir).await;
        let repo_obj = easyvibe_map::repo_from_root(&repo.dir);
        let head1 = head_sha(&repo.dir).unwrap();
    
        // 产物合法 + HEAD 未漂移 → 锚点推进（全量成功后补锚点，零迁移路径）
        let snap = st.map_service.load_map(&repo_obj).await.ok();
        handle_full_terminal(&st, &repo_obj, &snap, &Some(head1.clone())).await;
        let st1 = easyvibe_map::synthesis::read_induction_state(&repo.dir).expect("全量成功后必须补锚点");
        assert_eq!(st1.head_sha, head1);
        assert_eq!(st1.mode, "full");
    
        // R2 实弹：归纳期间用户又提交 → 终态裁决不推进锚点
        repo.commit("user-commit-during-induction");
        let head2 = head_sha(&repo.dir).unwrap();
        assert_ne!(head1, head2);
        handle_full_terminal(&st, &repo_obj, &snap, &Some(head1.clone())).await;
        let st2 = easyvibe_map::synthesis::read_induction_state(&repo.dir).unwrap();
        assert_eq!(st2.head_sha, head1, "HEAD 漂移不得推进锚点（下次 range 覆盖漏掉的提交）");
    }
    
    #[tokio::test]
    async fn full_terminal_strict_failure_restores_old_map_and_skips_anchor() {
        let repo = TempRepo::new("ev-full-terminal-strict");
        repo.commit("init");
        std::fs::create_dir_all(repo.dir.join(".easyvibe/map")).unwrap();
        let good = strict_map();
        easyvibe_map::atomic_write_json(&repo.dir.join(".easyvibe/map/map.json"), &good).await.unwrap();
        let st = state_with_repo(&repo.dir).await;
        let repo_obj = easyvibe_map::repo_from_root(&repo.dir);
        let snap_before = st.map_service.load_map(&repo_obj).await.ok();
        let head = head_sha(&repo.dir).unwrap();
    
        // agent 写出 strict 非法产物（悬空边）→ 还原旧图 + 不写锚点 + 不进重试链
        let mut bad = good.clone();
        bad["edges"][0]["to"] = "ghost".into();
        easyvibe_map::atomic_write_json(&repo.dir.join(".easyvibe/map/map.json"), &bad).await.unwrap();
        handle_full_terminal(&st, &repo_obj, &snap_before, &Some(head)).await;
        let restored: Value =
            serde_json::from_str(&std::fs::read_to_string(repo.dir.join(".easyvibe/map/map.json")).unwrap()).unwrap();
        assert_eq!(restored, good, "strict 失败必须还原旧图");
        assert!(easyvibe_map::synthesis::read_induction_state(&repo.dir).is_none(), "产物非法不得推进锚点");
    }
    
    #[tokio::test]
    async fn incremental_terminal_bad_patch_enqueues_forced_full_fallback() {
        let repo = TempRepo::new("ev-inc-terminal-fallback");
        repo.commit("init");
        std::fs::create_dir_all(repo.dir.join(".easyvibe/map")).unwrap();
        let map = strict_map();
        easyvibe_map::atomic_write_json(&repo.dir.join(".easyvibe/map/map.json"), &map).await.unwrap();
        let st = state_with_repo(&repo.dir).await;
        let repo_obj = easyvibe_map::repo_from_root(&repo.dir);
        let head = head_sha(&repo.dir).unwrap();
    
        // agent 写了坏 patch（JSON 非法）→ 旧图保持 + 入队 force_full 全量回退（R3 契约）
        std::fs::write(repo.dir.join(".easyvibe/map/map.patch.json"), "{not json").unwrap();
        let ctx = IncrementalContext {
            head: head.clone(),
            prev_anchor: head.clone(),
            commit_log: String::new(),
            diff_numstat: String::new(),
            diff_content: String::new(),
            diff_paths: vec!["src/core/a.rs".into()],
        };
        handle_incremental_terminal(&st, &repo_obj, &ctx, &Some(head)).await;
        let kept: Value =
            serde_json::from_str(&std::fs::read_to_string(repo.dir.join(".easyvibe/map/map.json")).unwrap()).unwrap();
        assert_eq!(kept, map, "增量失败旧图必须原样");
        let queued = st.session_queue.peek(&repo_obj.id).await.expect("坏 patch 必须入队全量回退");
        assert_eq!(queued.kind, JobKind::Reinduce);
        assert!(queued.force_full, "回退任务必须强制全量（防增量无限环）");
        assert!(!easyvibe_map::synthesis::patch_path(&repo.dir).exists(), "坏 patch 必须清理");
    }
    
    #[tokio::test]
    async fn incremental_terminal_happy_path_applies_and_advances() {
        let repo = TempRepo::new("ev-inc-terminal-happy");
        repo.commit("init");
        std::fs::create_dir_all(repo.dir.join(".easyvibe/map")).unwrap();
        let map = strict_map();
        easyvibe_map::atomic_write_json(&repo.dir.join(".easyvibe/map/map.json"), &map).await.unwrap();
        let st = state_with_repo(&repo.dir).await;
        let repo_obj = easyvibe_map::repo_from_root(&repo.dir);
        let head = head_sha(&repo.dir).unwrap();
    
        // 合法 patch：core 降分（diff 归属 core）
        let mut m = map["modules"][0].clone();
        m["health"]["score"] = 55.into();
        let mut patch = serde_json::Map::new();
        patch.insert("updated_modules".into(), Value::Array(vec![m]));
        let patch = Value::Object(patch);
        std::fs::write(repo.dir.join(".easyvibe/map/map.patch.json"), patch.to_string()).unwrap();
        let ctx = IncrementalContext {
            head: head.clone(),
            prev_anchor: head.clone(),
            commit_log: String::new(),
            diff_numstat: "1\t1\tsrc/core/a.rs".into(),
            diff_content: String::new(),
            diff_paths: vec!["src/core/a.rs".into()],
        };
        handle_incremental_terminal(&st, &repo_obj, &ctx, &Some(head.clone())).await;
        let after: Value =
            serde_json::from_str(&std::fs::read_to_string(repo.dir.join(".easyvibe/map/map.json")).unwrap()).unwrap();
        assert_eq!(after["modules"][0]["health"]["score"], 55, "合成产物必须落盘");
        assert!(after["meta"]["generated_at"].as_str().unwrap() > "2026-01-01T00:00:00Z", "generated_at 由后端覆写");
        let anchor = easyvibe_map::synthesis::read_induction_state(&repo.dir).expect("增量成功必须推进锚点");
        assert_eq!(anchor.head_sha, head);
        assert_eq!(anchor.mode, "incremental");
        assert!(st.session_queue.peek(&repo_obj.id).await.is_none(), "成功路径不得入队回退");
    }
}
