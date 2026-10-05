//! task_exec 测试（仅测试编译）。

use super::*;
use super::test_util::*;

    #[tokio::test]
    async fn decide_is_atomic_against_double_submit() {
        // N27 回归：原子关卡推进——同一审批关的第二次 decide（双击/重试）必须 409，
        // 不得双留痕、不得双 spawn。顺序执行即可复现（第一次推进后条件不再匹配）。
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-decide-atomic-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("awaiting_approval");
        task.repo = "ev-decide-atomic-test".into();
        task.id = "task-atomic".into();
        task.gate = Some("diff".into());
        task_repo.create(&task).await.unwrap();

        executor.decide("task-atomic", "approved", None, Some("diff")).await.unwrap();
        // 双击穿透复现：UI 仍停在 diff 关的第二次提交（声称 diff）→ 必须 409，不得推进到 report/done
        let err = executor.decide("task-atomic", "approved", None, Some("diff")).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))), "第二次 decide 必须 409: {err:?}");
        let t = task_repo.get("task-atomic").await.unwrap().unwrap();
        assert_eq!(t.gate.as_deref(), Some("report"), "只推进一次");
        let aps = approvals.list_by_task("task-atomic").await.unwrap();
        assert_eq!(aps.len(), 1, "只留痕一次（双审批留痕是 N27 的实弹症状）");
    }

    #[tokio::test]
    async fn decide_state_machine_guards() {
        // P0 审查后端#2：decision 白名单 + 仅 awaiting_approval 可审批（复活旁路封死）
        use easyvibe_db::{Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-decide-guard-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo_root = dir.clone();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&repo_root)]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );

        // pending 任务不可审批（此前会复活 spawn）
        let mut task = sample_task("pending");
        task.repo = "ev-decide-guard-test".into();
        task.id = "task-guard-pending".into();
        task_repo.create(&task).await.unwrap();
        let err = executor.decide("task-guard-pending", "approved", None, None).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))), "pending 任务必须 409: {err:?}");
        assert_eq!(task_repo.get("task-guard-pending").await.unwrap().unwrap().status, "pending", "状态不得被污染");

        // failed 任务不可审批（复活旁路）
        let mut task2 = sample_task("failed");
        task2.repo = "ev-decide-guard-test".into();
        task2.id = "task-guard-failed".into();
        task2.gate = Some("plan".into());
        task_repo.create(&task2).await.unwrap();
        let err = executor.decide("task-guard-failed", "approved", None, None).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))), "failed 任务必须 409");

        // 非法 decision 字符串一律 400（此前会被当 approved）
        let mut task3 = sample_task("awaiting_approval");
        task3.repo = "ev-decide-guard-test".into();
        task3.id = "task-guard-bad".into();
        task3.gate = Some("diff".into());
        task_repo.create(&task3).await.unwrap();
        let err = executor.decide("task-guard-bad", "whatever", None, None).await;
        assert!(matches!(err, Err(ApiError::BadRequest(_))), "非法 decision 必须 400: {err:?}");

        // 驳回无理由仍被拒（既有纪律不回归）
        let err = executor.decide("task-guard-bad", "rejected", None, None).await;
        assert!(matches!(err, Err(ApiError::BadRequest(_))));
    }

    #[tokio::test]
    async fn phase_review_merges_into_result_without_clobbering() {
        // 阶段初审结论入库语义：result 为 NULL 时新建骨架；二次合并不冲掉前一阶段结论
        use easyvibe_db::{Database, SqliteTaskRepository, TaskRepository as _};
        let db = Database::connect_memory().await.unwrap();
        let task_repo = SqliteTaskRepository::new(db.pool().clone());
        task_repo.create(&sample_task("awaiting_approval")).await.unwrap();

        merge_phase_review(&task_repo, "task-t1", "analysis", &ReviewVerdict { verdict: "pass".into(), summary: "矩阵完整".into() }).await;
        let r1: serde_json::Value =
            serde_json::from_str(&task_repo.get("task-t1").await.unwrap().unwrap().result.unwrap()).unwrap();
        assert_eq!(r1["phaseReviews"]["analysis"]["verdict"], "pass");

        merge_phase_review(&task_repo, "task-t1", "solution", &ReviewVerdict { verdict: "fail".into(), summary: "方案漏 R3".into() }).await;
        let r2: serde_json::Value =
            serde_json::from_str(&task_repo.get("task-t1").await.unwrap().unwrap().result.unwrap()).unwrap();
        assert_eq!(r2["phaseReviews"]["analysis"]["summary"], "矩阵完整", "analysis 结论必须保留");
        assert_eq!(r2["phaseReviews"]["solution"]["verdict"], "fail");
    }

    #[tokio::test]
    async fn manual_task_waits_for_approval_then_full_flow() {
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-task-exec-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo_root = dir.clone();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&repo_root)]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.repo = "ev-task-exec-test".into();
        task.trust = "manual".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-task-exec-test")).await;
        // manual：不 spawn，等待计划审批
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.status, "awaiting_approval");
        assert_eq!(t.gate.as_deref(), Some("plan"));
        // 分阶段执行流（2026-10-03）：plan → 阶段1 需求矩阵 → analysis 关 →
        // 阶段2 方案 → solution 关 → 阶段3 实施 →（审查不可用：true 无输出）→ diff → report → done。
        // true 立即成功，每关都异步回写——统一等关助手。
        async fn wait_gate(task_repo: &easyvibe_db::SqliteTaskRepository, want: &str) -> easyvibe_db::TaskRow {
            let mut t = task_repo.get("task-t1").await.unwrap().unwrap();
            for _ in 0..40 {
                if t.status == "awaiting_approval" && t.gate.as_deref() == Some(want) { return t }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                t = task_repo.get("task-t1").await.unwrap().unwrap();
            }
            panic!("未等到 {want} 关（当前 {:?}/{:?}）", t.status, t.gate);
        }
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // plan → 阶段1
        wait_gate(&task_repo, "analysis").await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // analysis → 阶段2
        wait_gate(&task_repo, "solution").await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // solution → 阶段3 实施
        wait_gate(&task_repo, "diff").await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // diff → report
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.gate.as_deref(), Some("report"));
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // report → done
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.status, "done");
        // 留痕：plan/analysis/solution/diff/report 五条 approved（分阶段全链路）
        let aps = approvals.list_by_task("task-t1").await.unwrap();
        assert_eq!(aps.len(), 5);
        assert!(aps.iter().all(|a| a.decision == "approved"));
    }

    #[tokio::test]
    async fn review_fail_auto_rejects_task() {
        // B案：独立子agent审查 fail → 任务自动打回（rejected），理由入留痕，不进 diff 关
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-task-review-fail-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        // echo 同时充当执行 agent 与审查 agent：打印 REVIEW 结论行（fail）
        // 主会话 echo 该行至 stdout——collect 无 RESULT 行不阻断；审查会话解析同一行 → fail
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("echo".into()),
            Arc::new(vec!["[EASYVIBE-REVIEW] {\"verdict\":\"fail\",\"summary\":\"存在阻断性问题\"}".into()]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.repo = "ev-task-review-fail-test".into();
        task.trust = "manual".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-task-review-fail-test")).await;
        // 分阶段流：plan → analysis → solution 三关都过，阶段3 实施后审查会话（echo 打 fail）→ 自动打回
        async fn wait_status(task_repo: &easyvibe_db::SqliteTaskRepository, want_status: &str, want_gate: Option<&str>) -> easyvibe_db::TaskRow {
            let mut t = task_repo.get("task-t1").await.unwrap().unwrap();
            for _ in 0..60 {
                let gate_ok = want_gate.map_or(true, |g| t.gate.as_deref() == Some(g));
                if t.status == want_status && gate_ok { return t }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                t = task_repo.get("task-t1").await.unwrap().unwrap();
            }
            panic!("未等到 {want_status}/{want_gate:?}（当前 {:?}/{:?}）", t.status, t.gate);
        }
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // plan → 阶段1
        wait_status(&task_repo, "awaiting_approval", Some("analysis")).await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // analysis → 阶段2
        wait_status(&task_repo, "awaiting_approval", Some("solution")).await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // solution → 阶段3 实施
        let t = wait_status(&task_repo, "rejected", None).await;
        assert_eq!(t.status, "rejected", "审查 fail 必须自动打回，不进 diff 关");
        assert!(t.error.as_deref().unwrap_or("").contains("存在阻断性问题"), "打回理由必须入 error 留痕");
        // 留痕：diff 关有一条 rejected 审批（审查打回），用户可见可溯源
        let aps = approvals.list_by_task("task-t1").await.unwrap();
        assert!(aps.iter().any(|a| a.gate == "diff" && a.decision == "rejected"), "审查打回必须留审批痕");
    }

    #[tokio::test]
    async fn review_unavailable_does_not_block_diff_gate() {
        // B案降级路径：审查会话产出无 [EASYVIBE-REVIEW] 行（如 true 命令）→ 审查不可用
        // → 不阻断，照常进 diff 关（人机审查兜底）
        use easyvibe_db::{Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-task-review-na-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()), // 无输出：审查会话拿不到结论行
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.repo = "ev-task-review-na-test".into();
        task.trust = "manual".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-task-review-na-test")).await;
        // 分阶段流三关走通；阶段3 实施后审查会话（true 无输出）拿不到结论 → 不阻断
        async fn wait_gate2(task_repo: &easyvibe_db::SqliteTaskRepository, want: &str) -> easyvibe_db::TaskRow {
            let mut t = task_repo.get("task-t1").await.unwrap().unwrap();
            for _ in 0..60 {
                if t.status == "awaiting_approval" && t.gate.as_deref() == Some(want) { return t }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                t = task_repo.get("task-t1").await.unwrap().unwrap();
            }
            panic!("未等到 {want} 关（当前 {:?}/{:?}）", t.status, t.gate);
        }
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // plan → 阶段1
        wait_gate2(&task_repo, "analysis").await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // analysis → 阶段2
        wait_gate2(&task_repo, "solution").await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // solution → 阶段3 实施
        let t = wait_gate2(&task_repo, "diff").await;
        assert_eq!(t.gate.as_deref(), Some("diff"), "审查不可用不得阻断 diff 关");
    }

    #[tokio::test]
    async fn auto_task_runs_straight_with_skipped_trace() {
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-task-exec-test2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.id = "task-auto".into();
        task.repo = "ev-task-exec-test2".into();
        task.trust = "auto".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-task-exec-test2")).await;
        let mut t = task_repo.get("task-auto").await.unwrap().unwrap();
        for _ in 0..10 {
            if t.status == "done" { break }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = task_repo.get("task-auto").await.unwrap().unwrap();
        }
        assert_eq!(t.status, "done");
        let aps = approvals.list_by_task("task-auto").await.unwrap();
        assert_eq!(aps.iter().filter(|a| a.decision == "skipped").count(), 3);
    }

    #[tokio::test]
    async fn auto_task_collects_result_and_archives() {
        use easyvibe_db::{Database, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-task-collect-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir.join(".easyvibe/map")).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(easyvibe_db::SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        // echo 打印 RESULT 行到 stdout（忽略 stdin prompt）——采集链路的零成本验证
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals,
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("echo".into()),
            Arc::new(vec!["[EASYVIBE-RESULT] {\"summary\":\"修复完成\",\"changed_modules\":[\"m1\"]}".to_string()]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.id = "task-collect".into();
        task.repo = "ev-task-collect-test".into();
        task.trust = "auto".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-task-collect-test")).await;
        let mut t = task_repo.get("task-collect").await.unwrap().unwrap();
        for _ in 0..10 {
            if t.status == "done" { break }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = task_repo.get("task-collect").await.unwrap().unwrap();
        }
        assert_eq!(t.status, "done");
        let result: serde_json::Value =
            serde_json::from_str(&t.result.expect("终态采集应写入 tasks.result")).unwrap();
        assert_eq!(result["result"]["summary"], "修复完成");
        assert_eq!(result["result"]["changed_modules"][0], "m1");
        assert!(result.get("diffFull").is_none(), "tasks.result 不背 diff 全文（列表载荷可控，M4-3）");
        // 归档文件落 development_docs/（§9 #2），且含 diffFull 键（按需端点读取）
        let archived = result["archivedPath"].as_str().expect("应返回归档路径");
        assert!(archived.contains("development_docs"), "归档须在 development_docs/: {archived}");
        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(archived).expect("归档文件应存在")).unwrap();
        assert_eq!(on_disk["taskId"], "task-collect");
        assert!(on_disk.get("diffFull").is_some(), "归档文件应含 diffFull（按需读取）");
    }

    #[tokio::test]
    async fn supervised_low_risk_runs_high_risk_stops_at_plan() {
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let dir = std::env::temp_dir().join("ev-supervised-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        let executor = TaskExecutor::new(
            task_repo.clone(), approvals.clone(), sessions, maps,
            harness_stub("框架"), Arc::new("true".into()), Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        // 低危（盲测 P0 新语义）：计划关自动通过，执行完成后停 diff 关等人工审批——
        // 不再直通 done（此前 diff/report 两关留痕 skipped，监督与自动无法区分）
        let mut low = sample_task("pending");
        low.id = "task-low".into();
        low.repo = "ev-supervised-test".into();
        low.trust = "supervised".into();
        task_repo.create(&low).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-supervised-test")).await;
        let mut t = task_repo.get("task-low").await.unwrap().unwrap();
        for _ in 0..10 {
            if t.status == "awaiting_approval" { break }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = task_repo.get("task-low").await.unwrap().unwrap();
        }
        assert_eq!(t.status, "awaiting_approval", "低危执行完成后必须停 diff 关（盲测 P0：监督要有真审批）");
        assert_eq!(t.gate.as_deref(), Some("diff"), "停在 diff 关");
        let aps = approvals.list_by_task("task-low").await.unwrap();
        assert!(aps.iter().any(|a| a.decision == "skipped" && a.note.as_deref().unwrap_or("").contains("计划关自动通过")), "低危计划关自动通过须留痕带理由");
        // diff 关通过 → 报告关；报告关通过 → done
        executor.decide("task-low", "approved", None, Some("diff")).await.unwrap();
        let t = task_repo.get("task-low").await.unwrap().unwrap();
        assert_eq!((t.status.as_str(), t.gate.as_deref()), ("awaiting_approval", Some("report")), "diff 通过后停报告关");
        executor.decide("task-low", "approved", None, Some("report")).await.unwrap();
        let t = task_repo.get("task-low").await.unwrap().unwrap();
        assert_eq!(t.status, "done", "报告关通过后 done");
        // 高危：多模块 + 高危词 → 停 plan 关 + flagged 留痕
        let mut high = sample_task("pending");
        high.id = "task-high".into();
        high.repo = "ev-supervised-test".into();
        high.trust = "supervised".into();
        high.modules = "[\"a\",\"b\",\"c\",\"d\"]".into();
        high.description = "整体重构".into();
        task_repo.create(&high).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-supervised-test")).await;
        let t = task_repo.get("task-high").await.unwrap().unwrap();
        assert_eq!(t.status, "awaiting_approval", "高危应停计划关");
        assert_eq!(t.gate.as_deref(), Some("plan"));
        let aps = approvals.list_by_task("task-high").await.unwrap();
        assert!(aps.iter().any(|a| a.decision == "flagged" && a.note.as_deref().unwrap_or("").contains("风险预评估")), "高危须 flagged 留痕带理由");
    }

    #[tokio::test]
    async fn reject_requires_reason() {
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.trust = "manual".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("demo")).await;
        // M4-2：驳回无理由 → 400；有理由 → 终止且留痕
        let err = executor.decide("task-t1", "rejected", None, None).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)), "空理由驳回必须被拒: {err}");
        let err = executor.decide("task-t1", "rejected", Some("  "), None).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)), "空白理由同样被拒");
        executor.decide("task-t1", "rejected", Some("方案风险过大"), None).await.unwrap();
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.status, "rejected");
        let aps = approvals.list_by_task("task-t1").await.unwrap();
        assert_eq!(aps.len(), 1);
        assert_eq!(aps[0].decision, "rejected");
        assert_eq!(aps[0].note.as_deref(), Some("方案风险过大"));
    }

    #[tokio::test]
    async fn executes_to_terminal_with_stub() {
        use easyvibe_db::{Database, SqliteTaskRepository};
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![]);
        let approvals = Arc::new(easyvibe_db::SqliteApprovalRepository::new(db.pool().clone()));
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals,
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()), // stub：立即成功
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.trust = "auto".into(); // auto 直通 → spawn_and_watch → 仓库未注册 → failed
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("demo")).await;
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.status, "failed");
    }

    #[tokio::test]
    async fn conflict_arm_returns_task_to_pending() {
        // R3 P0-1 回归：写互斥（仓库已有活动会话）时 spawn_and_watch 必须把任务退回 pending——
        // execute 已置 running，若不退回，retry 循环只扫 pending，任务假活 running 至重启
        // （N25 幽灵任务在 permits 满路径防过、Conflict 路径漏掉的孪生 bug）
        use easyvibe_db::{Database, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-conflict-arm-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let repo = easyvibe_map::repo_from_root(&dir);
        let repo_id = repo.id.clone();
        let maps = MapService::new(vec![repo]);
        // 占住仓库：一个活动会话（等价于归纳/巡检进行中）
        sessions
            .try_register(easyvibe_api_types::SessionStatusChanged {
                repo: repo_id.clone(),
                session_id: "ind-blocker".into(),
                status: easyvibe_api_types::SessionStatus::Running,
            })
            .await
            .unwrap();
        let executor = TaskExecutor::new(
            task_repo.clone(),
            Arc::new(easyvibe_db::SqliteApprovalRepository::new(db.pool().clone())),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.repo = repo_id.clone();
        task.id = "task-conflict".into();
        task.trust = "auto".into();
        task_repo.create(&task).await.unwrap();

        executor.clone().execute(task).await;
        // 第一时间：execute 曾把状态推进 running，Conflict 臂必须已退回 pending
        let t = task_repo.get("task-conflict").await.unwrap().unwrap();
        assert_eq!(t.status, "pending", "写互斥必须退回 pending，不得滞留 running（假活）");

        // 等过 retry 节拍（5s 后 enqueue 重扫）： Conflict 依旧（会话仍占用），仍应停在 pending
        tokio::time::sleep(std::time::Duration::from_secs(7)).await;
        let t = task_repo.get("task-conflict").await.unwrap().unwrap();
        assert_eq!(t.status, "pending", "retry 重扫撞上持续互斥，任务应排队而非假活");

        // 2026-10-05 实弹回归（治理任务进度清零）：gate 带 p:implement 的 manual 任务
        // 被写互斥退回后，retry 必须原地保留阶段重跑——不得重置回 plan 关
        // （此前 manual 分支把 gate 重置回 plan = 已评审的矩阵/方案全部作废）。
        let mut task2 = sample_task("pending");
        task2.repo = repo_id.clone();
        task2.id = "task-conflict-phase".into();
        task2.trust = "manual".into();
        task2.gate = Some("p:implement".into());
        task_repo.create(&task2).await.unwrap();
        executor.clone().execute(task2).await;
        let t2 = task_repo.get("task-conflict-phase").await.unwrap().unwrap();
        assert_eq!(t2.status, "pending", "互斥退回 pending");
        assert_eq!(t2.gate.as_deref(), Some("p:implement"), "退回不得清阶段标记");
        tokio::time::sleep(std::time::Duration::from_secs(7)).await;
        let t2 = task_repo.get("task-conflict-phase").await.unwrap().unwrap();
        assert_eq!(t2.status, "pending", "持续互斥仍排队");
        assert_eq!(
            t2.gate.as_deref(),
            Some("p:implement"),
            "retry 重扫不得把 manual 任务重置回 plan 关（进度清零 bug 回归）"
        );
        assert_ne!(t2.status, "awaiting_approval", "绝不允许回退到计划审批关");
    }

    // ---------- 方案 v3 §4.4：路径换姓 + 版本迁移 ----------
