//! task_exec 测试（仅测试编译）。

use super::*;
use super::test_util::*;

    #[tokio::test]
    async fn decide_is_atomic_against_double_submit() {
        // N27 回归：原子关卡推进——同一审批关的第二次 decide（双击/重试）必须 409，
        // 不得双留痕、不得双 spawn。顺序执行即可复现（第一次推进后条件不再匹配）。
        let dir = std::env::temp_dir().join("ev-decide-atomic-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let task_repo = Arc::new(InMemoryTaskStore::new());
        let approvals = Arc::new(InMemoryApprovalStore::new());
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        let executor = TaskExecutor::new(
     task_repo.clone(),
     approvals.clone(),
     sessions,
     maps,
     harness_stub("框架"),
     4,
     Arc::new(FixedAgentSlot::new(Arc::new("true".into()), Arc::new(vec![]))),
     None,
 );
        let mut task = sample_task("awaiting_approval");
        task.repo = "ev-decide-atomic-test".into();
        task.id = "task-atomic".into();
        task.gate = Some("diff".into());
        seed_task(task_repo.as_ref(), &task);

        executor.decide("task-atomic", "approved", None, Some("diff")).await.unwrap();
        // 双击穿透复现：UI 仍停在 diff 关的第二次提交（声称 diff）→ 必须 409，不得推进到 report/done
        let err = executor.decide("task-atomic", "approved", None, Some("diff")).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))), "第二次 decide 必须 409: {err:?}");
        let t = task_repo.get("task-atomic").await.unwrap().unwrap();
        assert_eq!(t.gate.as_deref(), Some("report"), "只推进一次");
        let aps = approvals_of(approvals.as_ref(), "task-atomic").await;
        assert_eq!(aps.len(), 1, "只留痕一次（双审批留痕是 N27 的实弹症状）");
    }

    #[tokio::test]
    async fn decide_state_machine_guards() {
        // P0 审查后端#2：decision 白名单 + 仅 awaiting_approval 可审批（复活旁路封死）
        let dir = std::env::temp_dir().join("ev-decide-guard-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo_root = dir.clone();
        let task_repo = Arc::new(InMemoryTaskStore::new());
        let approvals = Arc::new(InMemoryApprovalStore::new());
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&repo_root)]);
        let executor = TaskExecutor::new(
     task_repo.clone(),
     approvals.clone(),
     sessions,
     maps,
     harness_stub("框架"),
     4,
     Arc::new(FixedAgentSlot::new(Arc::new("true".into()), Arc::new(vec![]))),
     None,
 );

        // pending 任务不可审批（此前会复活 spawn）
        let mut task = sample_task("pending");
        task.repo = "ev-decide-guard-test".into();
        task.id = "task-guard-pending".into();
        seed_task(task_repo.as_ref(), &task);
        let err = executor.decide("task-guard-pending", "approved", None, None).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))), "pending 任务必须 409: {err:?}");
        assert_eq!(task_repo.get("task-guard-pending").await.unwrap().unwrap().status, "pending", "状态不得被污染");

        // failed 任务不可审批（复活旁路）
        let mut task2 = sample_task("failed");
        task2.repo = "ev-decide-guard-test".into();
        task2.id = "task-guard-failed".into();
        task2.gate = Some("plan".into());
        seed_task(task_repo.as_ref(), &task2);
        let err = executor.decide("task-guard-failed", "approved", None, None).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))), "failed 任务必须 409");

        // 非法 decision 字符串一律 400（此前会被当 approved）
        let mut task3 = sample_task("awaiting_approval");
        task3.repo = "ev-decide-guard-test".into();
        task3.id = "task-guard-bad".into();
        task3.gate = Some("diff".into());
        seed_task(task_repo.as_ref(), &task3);
        let err = executor.decide("task-guard-bad", "whatever", None, None).await;
        assert!(matches!(err, Err(ApiError::BadRequest(_))), "非法 decision 必须 400: {err:?}");

        // 驳回无理由仍被拒（既有纪律不回归）
        let err = executor.decide("task-guard-bad", "rejected", None, None).await;
        assert!(matches!(err, Err(ApiError::BadRequest(_))));
    }

    #[tokio::test]
    async fn phase_review_merges_into_result_without_clobbering() {
        // 阶段初审结论入库语义：result 为 NULL 时新建骨架；二次合并不冲掉前一阶段结论
        let task_repo = InMemoryTaskStore::new();
        seed_task(&task_repo, &sample_task("awaiting_approval"));

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
        let dir = std::env::temp_dir().join("ev-task-exec-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo_root = dir.clone();
        let task_repo = Arc::new(InMemoryTaskStore::new());
        let approvals = Arc::new(InMemoryApprovalStore::new());
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&repo_root)]);
        let executor = TaskExecutor::new(
     task_repo.clone(),
     approvals.clone(),
     sessions,
     maps,
     harness_stub("框架"),
     4,
     Arc::new(FixedAgentSlot::new(Arc::new("true".into()), Arc::new(vec![]))),
     None,
 );
        let mut task = sample_task("pending");
        task.repo = "ev-task-exec-test".into();
        task.trust = "manual".into();
        seed_task(task_repo.as_ref(), &task);
        executor.clone().enqueue_pending(Some("ev-task-exec-test")).await;
        // manual：不 spawn，等待计划审批
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.status, "awaiting_approval");
        assert_eq!(t.gate.as_deref(), Some("plan"));
        // 分阶段执行流（2026-10-03）：plan → 阶段1 需求矩阵 → analysis 关 →
        // 阶段2 方案 → solution 关 → 阶段3 实施 →（审查不可用：true 无输出）→ diff → report → done。
        // true 立即成功，每关都异步回写——统一等关助手。
        async fn wait_gate(task_repo: &std::sync::Arc<InMemoryTaskStore>, want: &str) -> TaskRecord {
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
        let aps = approvals_of(approvals.as_ref(), "task-t1").await;
        assert_eq!(aps.len(), 5);
        assert!(aps.iter().all(|a| a.decision == "approved"));
    }

    #[tokio::test]
    async fn review_fail_auto_rejects_task() {
        // B案：独立子agent审查 fail → 任务自动打回（rejected），理由入留痕，不进 diff 关
        let dir = std::env::temp_dir().join("ev-task-review-fail-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let task_repo = Arc::new(InMemoryTaskStore::new());
        let approvals = Arc::new(InMemoryApprovalStore::new());
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
     4,
     Arc::new(FixedAgentSlot::new(Arc::new("echo".into()), Arc::new(vec!["[EASYVIBE-REVIEW] {\"verdict\":\"fail\",\"summary\":\"存在阻断性问题\"}".into()]))),
     None,
 );
        let mut task = sample_task("pending");
        task.repo = "ev-task-review-fail-test".into();
        task.trust = "manual".into();
        seed_task(task_repo.as_ref(), &task);
        executor.clone().enqueue_pending(Some("ev-task-review-fail-test")).await;
        // 分阶段流：plan → analysis → solution 三关都过，阶段3 实施后审查会话（echo 打 fail）→ 自动打回
        async fn wait_status(task_repo: &std::sync::Arc<InMemoryTaskStore>, want_status: &str, want_gate: Option<&str>) -> TaskRecord {
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
        let aps = approvals_of(approvals.as_ref(), "task-t1").await;
        assert!(aps.iter().any(|a| a.gate == "diff" && a.decision == "rejected"), "审查打回必须留审批痕");
    }

    #[tokio::test]
    async fn review_unavailable_does_not_block_diff_gate() {
        // B案降级路径：审查会话产出无 [EASYVIBE-REVIEW] 行（如 true 命令）→ 审查不可用
        // → 不阻断，照常进 diff 关（人机审查兜底）
        let dir = std::env::temp_dir().join("ev-task-review-na-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let task_repo = Arc::new(InMemoryTaskStore::new());
        let approvals = Arc::new(InMemoryApprovalStore::new());
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        let executor = TaskExecutor::new(
     task_repo.clone(),
     approvals.clone(),
     sessions,
     maps,
     harness_stub("框架"),
     4,
     Arc::new(FixedAgentSlot::new(Arc::new("true".into()), // 无输出：审查会话拿不到结论行
            Arc::new(vec![]))),
     None,
 );
        let mut task = sample_task("pending");
        task.repo = "ev-task-review-na-test".into();
        task.trust = "manual".into();
        seed_task(task_repo.as_ref(), &task);
        executor.clone().enqueue_pending(Some("ev-task-review-na-test")).await;
        // 分阶段流三关走通；阶段3 实施后审查会话（true 无输出）拿不到结论 → 不阻断
        async fn wait_gate2(task_repo: &std::sync::Arc<InMemoryTaskStore>, want: &str) -> TaskRecord {
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
        // 2026-10-06 实弹修正：不可用必须落 result.review（verdict=error + 原因），前端灰横幅可见
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        let result: serde_json::Value =
            serde_json::from_str(&t.result.expect("审查不可用必须落 result.review（verdict=error）")).unwrap();
        assert_eq!(result["review"]["verdict"], "error", "审查不可用 verdict 必须为 error");
        assert!(
            !result["review"]["summary"].as_str().unwrap_or("").is_empty(),
            "审查不可用 summary 必须携带失败原因"
        );
    }

    #[tokio::test]
    async fn phase_review_unavailable_writes_error_verdict() {
        // 2026-10-06 实弹修正（初审静默缺失）：初审不可用不再只留一行日志——
        // 结论行 JSON 非法 → Unavailable(原因) → result.phaseReviews.analysis =
        // { verdict: "error", summary: 原因 }，且仍照常进 analysis 人工关（不阻断语义不变）。
        // echo 同时充当执行 agent 与初审 agent：打印非法 REVIEW 行——主会话成功
        // （phase 1 跳过采集），初审会话解析失败 → Unavailable。
        let dir = std::env::temp_dir().join("ev-phase-review-err-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let task_repo = Arc::new(InMemoryTaskStore::new());
        let approvals = Arc::new(InMemoryApprovalStore::new());
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        let executor = TaskExecutor::new(
     task_repo.clone(),
     approvals,
     sessions,
     maps,
     harness_stub("框架"),
     4,
     Arc::new(FixedAgentSlot::new(Arc::new("echo".into()), Arc::new(vec!["[EASYVIBE-REVIEW] {not-json".to_string()]))),
     None,
 );
        let mut task = sample_task("pending");
        task.repo = "ev-phase-review-err-test".into();
        task.trust = "manual".into();
        seed_task(task_repo.as_ref(), &task);
        executor.clone().enqueue_pending(Some("ev-phase-review-err-test")).await;
        // plan 关 → 通过 → 阶段1 执行（echo 成功）→ 初审会话解析非法 JSON → error 落库
        let t0 = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t0.gate.as_deref(), Some("plan"), "manual 任务先入 plan 关");
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // plan → 阶段1
        let mut t = task_repo.get("task-t1").await.unwrap().unwrap();
        for _ in 0..60 {
            if t.status == "awaiting_approval" && t.gate.as_deref() == Some("analysis") { break }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = task_repo.get("task-t1").await.unwrap().unwrap();
        }
        assert_eq!(t.status, "awaiting_approval", "初审不可用仍须进人工关（不阻断）");
        assert_eq!(t.gate.as_deref(), Some("analysis"));
        let result: serde_json::Value =
            serde_json::from_str(&t.result.expect("初审 error 结论必须落 result.phaseReviews")).unwrap();
        assert_eq!(
            result["phaseReviews"]["analysis"]["verdict"], "error",
            "初审不可用 verdict 必须为 error（前端灰横幅判定依据）"
        );
        let summary = result["phaseReviews"]["analysis"]["summary"].as_str().unwrap_or("");
        assert!(!summary.is_empty(), "初审不可用 summary 必须携带失败原因");
    }

    #[tokio::test]
    async fn review_unavailable_carries_specific_reason() {
        // 失败原因取证（2026-10-06 实弹：claude 拒模型只写 stderr，stdout 为空——
        // 原因必须具体到可排查）：
        //   异常终态无 stderr → 通用文案；异常终态有 stderr → 最后一条 stderr 行；
        //   成功但无结论行 → 缺失文案；run_subagent_review 同路径。
        let dir = std::env::temp_dir().join("ev-review-reason-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let task = sample_task("running");
        let custom = HarnessCustom::default();

        // ① 异常终态、无输出（false）→ "会话异常终态"
        let o1 = run_phase_doc_review(&sessions, "false", &[], "demo", &dir, &task, 1, &custom).await;
        let ReviewOutcome::Unavailable(r1) = o1 else { panic!("false 命令必须 Unavailable: {o1:?}") };
        assert_eq!(r1, "会话异常终态");

        // ② 异常终态、stderr 有料（实弹形态）→ 最后一条 stderr 行
        let o2 = run_phase_doc_review(
            &sessions,
            "sh",
            &["-c".into(), "echo unrecognized_model >&2; exit 1".into()],
            "demo",
            &dir,
            &task,
            1,
            &custom,
        )
        .await;
        let ReviewOutcome::Unavailable(r2) = o2 else { panic!("stderr 场景必须 Unavailable: {o2:?}") };
        assert!(r2.contains("unrecognized_model"), "失败原因必须取最后一条 stderr 行: {r2}");

        // ③ 成功但结论行缺失（true 无输出）→ 缺失文案
        let o3 = run_phase_doc_review(&sessions, "true", &[], "demo", &dir, &task, 1, &custom).await;
        let ReviewOutcome::Unavailable(r3) = o3 else { panic!("无结论行必须 Unavailable: {o3:?}") };
        assert!(r3.contains("[EASYVIBE-REVIEW]"), "结论行缺失须明示: {r3}");

        // ④ run_subagent_review 不可用同路径（diff 关审查的降级原料）
        let o4 = run_subagent_review(&sessions, "false", &[], "demo", &dir, &task, &custom, ("", "")).await;
        let ReviewOutcome::Unavailable(r4) = o4 else { panic!("子agent审查 false 必须 Unavailable: {o4:?}") };
        assert_eq!(r4, "会话异常终态");
    }
