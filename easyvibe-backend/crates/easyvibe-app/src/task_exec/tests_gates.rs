//! task_exec 测试（仅测试编译）。

use super::*;
use super::test_util::*;

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
