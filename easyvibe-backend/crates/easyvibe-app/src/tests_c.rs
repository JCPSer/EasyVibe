//! main.rs 内联测试（迁移自 god file，仅测试编译）——分片 3/4。

use crate::router::*;
use crate::state::*;
use crate::test_support::*;
use tower::ServiceExt;

    #[tokio::test]
    async fn view_rename_prune_and_repo_wipe() {
        // 重审 P1：视图改名 / 巡检历史清理 / 注销仓库数据清除
        use easyvibe_db::{ConversationRepository as _, HealthRepository as _, TaskRepository as _};
        // 隔离持久化文件：remove_repo 会重写 desktop-repos（data_dir 受 EASYVIBE_DATA_DIR 支配）。
        // 指到真实 ~/.easyvibe 时，跑一遍测试 = 把开发者的仓库清单清空（2026-10-03 实弹踩坑）——
        // 本测试进程内所有 data_dir 调用都落到临时目录（并行测试同进程共享，无副作用）。
        let data_dir = std::env::temp_dir().join(format!("ev-test-data-{}", std::process::id()));
        std::fs::create_dir_all(&data_dir).unwrap();
        std::env::set_var("EASYVIBE_DATA_DIR", &data_dir);
        let (state, repo) = chat_state("p1-admin").await;
        let app = build_router(state.clone());

        // ── 视图：保存 → 改名 → 列表反映新 slug；冲突名 409 ──
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/views"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::json!({ "name": "支付链路", "nodes": ["module:a"] }).to_string()))
                .unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
        let resp = app.clone().oneshot(
            axum::http::Request::put(format!("/api/repos/{repo}/views/{}", urlencoding_encode("支付链路")))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::json!({ "name": "支付链路v2" }).to_string()))
                .unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "改名应成功");
        let resp = app.clone().oneshot(
            axum::http::Request::get(format!("/api/repos/{repo}/views")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let views = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"].clone();
        assert_eq!(views.as_array().unwrap().len(), 1, "改名是移动不是复制");
        assert_eq!(views[0]["slug"].as_str().unwrap(), "支付链路v2");
        assert_eq!(views[0]["name"].as_str().unwrap(), "支付链路v2");

        // ── 巡检历史：4 次终态 + 1 次 running，keep=2 → 删 2 条，running 不动 ──
        for (i, status) in ["succeeded", "succeeded", "failed", "succeeded"].iter().enumerate() {
            let rid = format!("prune-run-{i}");
            state.health_repo.create_run(&easyvibe_db::NewPatrolRun {
                id: rid.clone(), repo: repo.clone(), started_at: format!("2026100{i}0000"), model: "stub".into(),
            }).await.unwrap();
            state.health_repo.finish_run(&easyvibe_db::FinishPatrolRun {
                id: rid, finished_at: "1".into(), status: status.to_string(), arch_score: Some(60),
                error: None, prompt_tokens: None, completion_tokens: None, concerns_diff: None,
            }).await.unwrap();
            state.health_repo.insert_module_health(&easyvibe_db::ModuleHealthRow {
                run_id: format!("prune-run-{i}"), module_id: "m1".into(), name: Some("m1".into()), score: 60,
                coupling: None, complexity: None, churn: None,
                decay_flags: "[]".into(), review_note: None, concerns: "[]".into(),
            }).await.unwrap();
        }
        state.health_repo.create_run(&easyvibe_db::NewPatrolRun {
            id: "prune-run-live".into(), repo: repo.clone(), started_at: "202610090000".into(), model: "stub".into(),
        }).await.unwrap();
        let resp = app.clone().oneshot(
            axum::http::Request::delete(format!("/api/repos/{repo}/patrol-runs?keep=2")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let runs = state.health_repo.list_runs(&repo, 20).await.unwrap();
        assert_eq!(runs.len(), 3, "保留最近 2 次终态 + running 1 次");
        assert!(runs.iter().any(|r| r.id == "prune-run-live"), "running 永不进删除集");
        let hist = state.health_repo.list_module_history(&repo, "m1", 20).await.unwrap();
        assert_eq!(hist.len(), 2, "明细随主表级联清除");

        // ── 注销仓库：wipe=true 抹掉任务/会话/巡检，仓库从列表消失 ──
        state.task_repo.create(&easyvibe_db::TaskRow {
            id: "wipe-task".into(), repo: repo.clone(), title: "t".into(), description: "d".into(),
            modules: "[]".into(), acceptance: String::new(), source: "manual".into(), context: "{}".into(),
            status: "done".into(), trust: "manual".into(), error: None, session_id: None, gate: None,
            prompt_tokens: None, completion_tokens: None, result: None, base_head: None,
            created_at: "1".into(), updated_at: "1".into(), conversation_id: None,
            origin_task_id: None, successor_task_id: None,
        }).await.unwrap();
        let conv = state.conversation_repo.get_or_create(&repo).await.unwrap();
        state.conversation_repo.append_message(&conv.id, "user", "hi", 0).await.unwrap();
        let resp = app.clone().oneshot(
            axum::http::Request::delete(format!("/api/repos/{repo}?wipe=true")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        assert!(state.task_repo.get("wipe-task").await.unwrap().is_none(), "任务必须被清除");
        assert!(state.conversation_repo.list_by_repo(&repo).await.unwrap().is_empty(), "会话必须被清除");
        assert!(state.health_repo.list_runs(&repo, 20).await.unwrap().is_empty(), "巡检历史必须被清除");
        let resp = app.clone().oneshot(axum::http::Request::get("/api/repos").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let repos = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"].clone();
        assert!(repos.as_array().unwrap().iter().all(|r| r["id"] != repo), "仓库必须注销");
    }

    #[tokio::test]
    async fn dev_doc_accepts_repo_relative_path() {
        // 2026-10-03 实弹 bug 回归：dev-docs 返回仓库相对路径，/dev-doc 此前只收
        // development_docs 相对路径——双前缀 404，评审卡全文永远空白。两种形态都必须可读。
        let (state, repo) = chat_state("dev-doc-path").await;
        let dir = std::env::temp_dir().join("ev-chat-test-dev-doc-path/.easyvibe/development_docs/liyuhang/1_requirements_matrix");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("matrix.md"), "# 需求矩阵\n\nR1 xxx").unwrap();
        let app = build_router(state);
        for p in ["liyuhang/1_requirements_matrix/matrix.md", ".easyvibe/development_docs/liyuhang/1_requirements_matrix/matrix.md"] {
            let resp = app
                .clone()
                .oneshot(
                    axum::http::Request::get(format!(
                        "/api/repos/{repo}/dev-doc?path={}",
                        urlencoding_encode(p)
                    ))
                    .body(axum::body::Body::empty())
                    .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), axum::http::StatusCode::OK, "形态应可读: {p}");
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            let v = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
            assert!(v["data"]["content"].as_str().unwrap().contains("R1 xxx"), "内容必须完整: {p}");
        }
        // 穿越防线不动：目录外路径仍 404
        let resp = app
            .oneshot(
                axum::http::Request::get(format!(
                    "/api/repos/{repo}/dev-doc?path={}",
                    urlencoding_encode("../../etc/passwd")
                ))
                .body(axum::body::Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND, "穿越必须 404");
    }

    #[tokio::test]
    async fn task_rewind_back_to_review_gate() {
        // 管道回看·节点重开（方案 §3.1 + 评审修订）：走到 diff 关后 rewind 回 solution 关——
        // 状态归位、rewind 留痕、且 decide(approved) 零改动推进（重跑实施）。
        use easyvibe_db::{ApprovalRepository as _, TaskRepository as _};
        let (state, repo) = chat_state("task-rewind").await;
        let app = build_router(state.clone());
        let create = |title: &str| {
            let app = app.clone();
            let repo = repo.clone();
            let title = title.to_string();
            async move {
                let resp = app.oneshot(
                    axum::http::Request::post(format!("/api/repos/{repo}/tasks"))
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(serde_json::json!({ "title": title, "description": "回看回归", "trust": "manual" }).to_string()))
                        .unwrap(),
                ).await.unwrap();
                let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
                serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["id"].as_str().unwrap().to_string()
            }
        };
        let tid = create("回看重开").await;
        let decide = |gate: &str, decision: &str, note: &str| {
            let app = app.clone();
            let repo = repo.clone();
            let tid = tid.clone();
            let body = serde_json::json!({ "decision": decision, "note": note, "gate": gate }).to_string();
            async move {
                app.oneshot(
                    axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/decide"))
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body))
                        .unwrap(),
                ).await.unwrap()
            }
        };
        // 推到 diff 关：plan→(等 analysis)→analysis→(等 solution)→solution→(等 diff)
        assert_eq!(decide("plan", "approved", "开工").await.status(), axum::http::StatusCode::OK);
        async fn wait_gate(state: &AppState, tid: &str, gate: &str) -> easyvibe_db::TaskRow {
            let mut t = state.task_repo.get(tid).await.unwrap().unwrap();
            for _ in 0..40 {
                if t.status == "awaiting_approval" && t.gate.as_deref() == Some(gate) { break; }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                t = state.task_repo.get(tid).await.unwrap().unwrap();
            }
            t
        }
        wait_gate(&state, &tid, "analysis").await;
        assert_eq!(decide("analysis", "approved", "矩阵过").await.status(), axum::http::StatusCode::OK);
        wait_gate(&state, &tid, "solution").await;
        assert_eq!(decide("solution", "approved", "方案过").await.status(), axum::http::StatusCode::OK);
        let t = wait_gate(&state, &tid, "diff").await;
        assert_eq!(t.gate.as_deref(), Some("diff"), "前置：任务在 diff 关");

        // rewind 回 solution 关
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/rewind"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::json!({ "gate": "solution" }).to_string()))
                .unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "diff→solution rewind 应成功");
        let t = state.task_repo.get(&tid).await.unwrap().unwrap();
        assert_eq!(t.status, "awaiting_approval");
        assert_eq!(t.gate.as_deref(), Some("solution"));
        let aps = state.approval_repo.list_by_task(&tid).await.unwrap();
        assert!(
            aps.iter().any(|a| a.gate == "solution" && a.decision == "rewind"),
            "rewind 必须留痕（评审轮回可见），实际: {:?}",
            aps.iter().map(|a| (a.gate.clone(), a.decision.clone())).collect::<Vec<_>>()
        );

        // 零改动推进链：rewind 后再通过 → 重跑实施（running + p:implement）
        assert_eq!(decide("solution", "approved", "再看一遍没问题").await.status(), axum::http::StatusCode::OK);
        let t = state.task_repo.get(&tid).await.unwrap().unwrap();
        assert_eq!(t.status, "running");
        assert_eq!(t.gate.as_deref(), Some("p:implement"));
    }

    #[tokio::test]
    async fn task_manual_review_at_diff_gate() {
        // 2026-10-05 用户裁定：代码审查节点 = 审查-修复闭环——人工可在 diff 关
        // 发起子 agent 复审（实施后的自动审查之外），结论落 result.review + 留痕。
        use easyvibe_db::{ApprovalRepository as _, TaskRepository as _};
        let (state, repo) = chat_state("task-manual-review").await;
        let app = build_router(state.clone());
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::json!({ "title": "人工复审", "description": "代码审查闭环", "trust": "manual" }).to_string()))
                .unwrap(),
        ).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let tid = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["id"].as_str().unwrap().to_string();
        let decide = |gate: &str| {
            let app = app.clone();
            let repo = repo.clone();
            let tid = tid.clone();
            let gate = gate.to_string();
            async move {
                app.oneshot(
                    axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/decide"))
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(serde_json::json!({ "decision": "approved", "note": "过", "gate": gate }).to_string()))
                        .unwrap(),
                ).await.unwrap()
            }
        };
        // 非 diff 关发起复审 → 409
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/review")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT, "非 diff 关不得发起复审");

        // 推进到 diff 关
        assert_eq!(decide("plan").await.status(), axum::http::StatusCode::OK);
        async fn wait_gate(state: &AppState, tid: &str, gate: &str) -> easyvibe_db::TaskRow {
            let mut t = state.task_repo.get(tid).await.unwrap().unwrap();
            for _ in 0..40 {
                if t.status == "awaiting_approval" && t.gate.as_deref() == Some(gate) { break; }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                t = state.task_repo.get(tid).await.unwrap().unwrap();
            }
            t
        }
        wait_gate(&state, &tid, "analysis").await;
        decide("analysis").await;
        wait_gate(&state, &tid, "solution").await;
        decide("solution").await;
        let t = wait_gate(&state, &tid, "diff").await;
        assert_eq!(t.gate.as_deref(), Some("diff"));

        // diff 关发起人工复审 → 202；会话（"true" 无结论）→ 不可用留痕，不阻断
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/review")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::ACCEPTED, "diff 关发起复审应 202");
        let mut ok = false;
        for _ in 0..40 {
            let aps = state.approval_repo.list_by_task(&tid).await.unwrap();
            if aps.iter().any(|a| a.gate == "diff" && a.decision == "flagged" && a.note.as_deref().unwrap_or("").contains("人工复审")) {
                ok = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        assert!(ok, "复审结束必须留痕（人工复审：…），flagged 决策");
        let t = state.task_repo.get(&tid).await.unwrap().unwrap();
        assert_eq!(t.status, "awaiting_approval", "人工复审不迁移状态（裁决仍由人做）");
    }

    #[tokio::test]
    async fn task_rewind_source_states_and_guards() {
        // 评审#B1/B2/S2：done/failed 可 rewind（kill→failed 的回头路）；
        // auto 信任与 running 态拒绝；目标关白名单校验。
        use easyvibe_db::TaskRepository as _;
        let (state, repo) = chat_state("task-rewind-src").await;
        let app = build_router(state.clone());
        let mk = |title: &str, trust: &str| {
            let app = app.clone();
            let repo = repo.clone();
            let title = title.to_string();
            let trust = trust.to_string();
            async move {
                let resp = app.oneshot(
                    axum::http::Request::post(format!("/api/repos/{repo}/tasks"))
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(serde_json::json!({ "title": title, "description": "d", "trust": trust }).to_string()))
                        .unwrap(),
                ).await.unwrap();
                let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
                serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["id"].as_str().unwrap().to_string()
            }
        };
        let rewind = |app: axum::Router, repo: String, tid: String, gate: String| async move {
            app.oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/rewind"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::json!({ "gate": gate }).to_string()))
                    .unwrap(),
            ).await.unwrap()
        };

        // done → rewind（B2：专用原子 UPDATE，不能复用 try_advance_gate）
        let tid_done = mk("归档返工", "manual").await;
        state.task_repo.update_status(&tid_done, "done", None).await.unwrap();
        state.task_repo.set_gate(&tid_done, Some("done")).await.unwrap();
        let resp = rewind(app.clone(), repo.clone(), tid_done.clone(), "analysis".into()).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "done 任务应可 rewind 返工");
        let t = state.task_repo.get(&tid_done).await.unwrap().unwrap();
        assert_eq!((t.status.as_str(), t.gate.as_deref()), ("awaiting_approval", Some("analysis")));

        // failed（kill 会话后的典型态）→ rewind（B1 回头路）
        let tid_failed = mk("终止回头", "manual").await;
        state.task_repo.update_status(&tid_failed, "failed", Some("被终止")).await.unwrap();
        state.task_repo.set_gate(&tid_failed, Some("p:implement")).await.unwrap();
        let resp = rewind(app.clone(), repo.clone(), tid_failed.clone(), "solution".into()).await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "failed 任务应可 rewind 回上游关");
        let t = state.task_repo.get(&tid_failed).await.unwrap().unwrap();
        assert_eq!((t.status.as_str(), t.gate.as_deref()), ("awaiting_approval", Some("solution")));

        // auto 信任 → 409（S2）
        let tid_auto = mk("自动任务", "auto").await;
        state.task_repo.update_status(&tid_auto, "done", None).await.unwrap();
        let resp = rewind(app.clone(), repo.clone(), tid_auto.clone(), "analysis".into()).await;
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT, "auto 任务不支持 rewind");

        // running → 409
        let tid_run = mk("运行中", "manual").await;
        state.task_repo.update_status(&tid_run, "running", None).await.unwrap();
        let resp = rewind(app.clone(), repo.clone(), tid_run.clone(), "analysis".into()).await;
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT, "running 任务须先终止");

        // 目标关白名单 → 400
        let resp = rewind(app.clone(), repo.clone(), tid_done.clone(), "diff".into()).await;
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST, "rewind 仅支持 analysis/solution");
    }

    #[tokio::test]
    async fn task_retry_preserves_phase_gate() {
        // 2026-10-05 实弹（评审#B1 同源）：失败在 p: 阶段的 manual 任务 retry 时
        // gate 不得被清空——否则 execute() 够不着阶段感知分支，被打回 plan 关重来（进度清零）。
        use easyvibe_db::TaskRepository as _;
        let (state, repo) = chat_state("task-retry-gate").await;
        let app = build_router(state.clone());
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::json!({ "title": "中途失败", "description": "p:implement 失败", "trust": "manual" }).to_string()))
                .unwrap(),
        ).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let tid = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["id"].as_str().unwrap().to_string();
        state.task_repo.update_status(&tid, "failed", Some("被终止")).await.unwrap();
        state.task_repo.set_gate(&tid, Some("p:implement")).await.unwrap();
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/retry")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let t = state.task_repo.get(&tid).await.unwrap().unwrap();
        assert_eq!(t.gate.as_deref(), Some("p:implement"), "retry 必须保留 p: 阶段标记");
        // execute 阶段感知分支接管：等它跑完（"true" 秒退 → 收集 → 子agent审查不可用 → diff 关）
        let mut t = state.task_repo.get(&tid).await.unwrap().unwrap();
        for _ in 0..40 {
            if t.status == "awaiting_approval" || t.status == "done" { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = state.task_repo.get(&tid).await.unwrap().unwrap();
        }
        assert_eq!(t.status, "awaiting_approval", "应跑完实施回到审批流，而非停回 plan 关");
        assert_eq!(t.gate.as_deref(), Some("diff"));
    }
