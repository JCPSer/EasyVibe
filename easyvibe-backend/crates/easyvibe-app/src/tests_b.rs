//! main.rs 内联测试（迁移自 god file，仅测试编译）——分片 2/4。

use axum::{
    extract::State,
    response::IntoResponse,
};
use easyvibe_map::{repo_from_root, MapService};
use crate::router::*;
use crate::routes::map::*;
use crate::test_support::*;
use tower::ServiceExt;

    #[tokio::test]
    async fn views_roundtrip_create_list_delete() {
        let (state, repo) = chat_state("views-crud").await;
        let dir = std::env::temp_dir().join("ev-chat-test-views-crud/.easyvibe/views");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("我的视图.json"),
            r#"{"version":"1.0","name":"我的视图","created_at":"2026-09-30","nodes":[{"ref":"module:m1"}],"edges":[],"annotations":[]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("second.json"),
            r#"{"version":"1.0","name":"second","created_at":"2026-09-29","nodes":[],"edges":[],"annotations":[]}"#,
        )
        .unwrap();
        let app = build_router(state);
        let get = |app: axum::Router, path: &str| {
            let path = path.to_string();
            async move {
                let resp = app.oneshot(axum::http::Request::get(path).body(axum::body::Body::empty()).unwrap()).await.unwrap();
                let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
                serde_json::from_slice::<serde_json::Value>(&body).unwrap()
            }
        };
        // 列表：按 createdAt 倒序，中文 slug 可读
        let d = get(app.clone(), &format!("/api/repos/{repo}/views")).await;
        let items = d["data"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["slug"], "我的视图", "倒序：新者在前");
        assert_eq!(items[0]["nodes"], 1);
        // 删除：确认语义——删后列表减一
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::delete(format!("/api/repos/{repo}/views/{}", urlencoding_encode("second")))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let d = get(app.clone(), &format!("/api/repos/{repo}/views")).await;
        assert_eq!(d["data"].as_array().unwrap().len(), 1);
        // 非法 slug（路径遍历企图）→ 400
        let resp = app
            .oneshot(
                axum::http::Request::delete(format!("/api/repos/{repo}/views/..%2F..%2Fetc"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn task_diff_endpoint_reads_archive_on_demand() {
        let (state, repo) = chat_state("diff-endpoint").await;
        // 手工造归档（正常路径由终态采集写入）：diff 全文只进归档，tasks.result 不带
        let dir = std::env::temp_dir().join("ev-chat-test-diff-endpoint/.easyvibe/development_docs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("task-x.json"),
            r#"{"taskId":"task-x","diffFull":"+added line","diffStat":" a.txt | 1 +","archivedPath":null}"#,
        )
        .unwrap();
        let app = build_router(state);
        let get = |path: String| {
            let app = app.clone();
            async move {
                let resp = app.oneshot(axum::http::Request::get(path).body(axum::body::Body::empty()).unwrap()).await.unwrap();
                let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
                serde_json::from_slice::<serde_json::Value>(&body).unwrap()
            }
        };
        let d = get(format!("/api/repos/{repo}/tasks/task-x/diff")).await;
        assert_eq!(d["data"]["diff"], "+added line");
        assert_eq!(d["data"]["diffStat"], " a.txt | 1 +");
        // 无归档 → diff null（不 404：调用方区分"无变更"）
        let d = get(format!("/api/repos/{repo}/tasks/task-nope/diff")).await;
        assert!(d["data"]["diff"].is_null());
    }

    #[tokio::test]
    async fn chat_reset_starts_fresh_conversation() {
        let (state, repo) = chat_state("reset").await;
        let app = build_router(state);
        post_chat(&app, &repo, "一个问题").await;
        let resp = app
            .clone()
            .oneshot(axum::http::Request::post(format!("/api/repos/{repo}/chat/reset")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let resp = app
            .oneshot(axum::http::Request::get(format!("/api/repos/{repo}/chat")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let data = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"].clone();
        assert!(data["messages"].as_array().unwrap().is_empty());
        assert!(data["summary"].is_null());
    }

    #[tokio::test]
    async fn reinduce_collection_scoped_to_own_session() {
        // R3 P0-2 回归：归纳收尸循环必须按「自己 spawn 的会话」判定终态。
        // 旧代码用仓库级 status_of：归纳（agent "true" 秒退）终态后 2s 窗内新注册的会话会被
        // grace 收尸误判 Succeeded。本测试把 progress.json 预置为 phase=done 且 mtime 超 90s，
        // 诱导 grace 分支；若循环越界处置，patrol-second 会被改成 Succeeded。
        let dir = std::env::temp_dir().join("ev-reinduce-scope-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".easyvibe/map")).unwrap();
        let progress = dir.join(".easyvibe/map/progress.json");
        std::fs::write(&progress, r#"{"phase":"done"}"#).unwrap();
        // mtime 拨到 90s 之前（grace 阈值）
        let _ = std::process::Command::new("touch")
            .args(["-t", "202610010000", progress.to_str().unwrap()])
            .status();
        let repo = repo_from_root(&dir);
        let repo_id = repo.id.clone();
        let state = test_state_with(MapService::new(vec![repo])).await;

        // 发起重归纳：S1 秒级终态
        let resp = start_reinduce(State(state.clone()), axum::extract::Path(repo_id.clone()))
            .await
            .map_err(|e| e.0.to_string())
            .unwrap();
        let body = axum::body::to_bytes(resp.into_response().into_body(), usize::MAX).await.unwrap();
        let s1 = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["sessionId"].as_str().unwrap().to_string();

        // 等 S1 先终态（真实场景：用户在归纳结束后的瞬间紧接着发起巡检），
        // 然后抢在收尸循环的下一个 2s 轮询之前注册第二个会话
        let mut s1_terminal = false;
        for _ in 0..100 {
            if let Some(s) = state.session_manager.status_of_session(&s1).await {
                if !matches!(s.status, easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running) {
                    s1_terminal = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(s1_terminal, "S1（agent true）应在 5s 内终态");
        state
            .session_manager
            .try_register(easyvibe_api_types::SessionStatusChanged {
                repo: repo_id.clone(),
                session_id: "patrol-second".into(),
                status: easyvibe_api_types::SessionStatus::Running,
            })
            .await
            .unwrap();

        // 等过至少两个收尸循环节拍（2s/轮）+ grace 判定余量
        tokio::time::sleep(std::time::Duration::from_secs(6)).await;

        // 旧代码的误判会经 note_status 改写仓库级会话状态——以它为观测点：
        // patrol-second 必须保持 Running（probe 用 status_of_session 探不到 try_register 的会话，
        // 它在 by_id 无条目；误判的唯一痕迹是仓库级状态被改成 Succeeded）
        let repo_status = state
            .session_manager
            .status_of(&repo_id)
            .await
            .expect("注册后仓库应有活动会话");
        assert_eq!(repo_status.session_id, "patrol-second");
        assert_eq!(
            repo_status.status,
            easyvibe_api_types::SessionStatus::Running,
            "收尸循环只许处置自己 spawn 的会话，不得把后续会话误判终态: {:?}",
            repo_status.status
        );
        // 锚定：S1 确已终态——本测试真实跨过了「归纳结束 → 新会话注册」的竞态窗口
        let s1_status = state.session_manager.status_of_session(&s1).await.map(|s| s.status);
        assert!(
            matches!(
                s1_status,
                Some(easyvibe_api_types::SessionStatus::Succeeded) | Some(easyvibe_api_types::SessionStatus::Failed)
            ),
            "S1 应已终态（否则本测试未覆盖竞态窗口）: {s1_status:?}"
        );
    }

    #[tokio::test]
    async fn rework_lineage_links_origin_and_injects_reason() {
        // R3 D2 回归：复制为新任务（返工）三入口统一——
        // ① originTaskId 建立血缘并回填原任务 successor 反链（返工率聚合的命脉）
        // ② 驳回理由由后端从原任务审批留痕自动注入描述（此前只有评审页入口注入）
        use easyvibe_db::TaskRepository as _;
        let (state, repo) = chat_state("lineage").await;
        let app = build_router(state.clone());

        // 原任务：manual 停在计划关 → 驳回（带理由）
        let post = |body: serde_json::Value| {
            let app = app.clone();
            let repo = repo.clone();
            async move {
                app.oneshot(
                    axum::http::Request::post(format!("/api/repos/{repo}/tasks"))
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };
        let resp = post(serde_json::json!({ "title": "原任务", "description": "第一次尝试", "trust": "manual" })).await;
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/tasks"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::json!({ "title": "原任务", "description": "第一次尝试", "trust": "manual" }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let origin_id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["id"].as_str().unwrap().to_string();

        // 驳回（计划关）带理由——decide 需要 expected_gate（当前 gate=plan）
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/tasks/{origin_id}/decide"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::json!({ "decision": "rejected", "note": "方案风险过大", "gate": "plan" }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "驳回应成功");

        // 返工：复制为新任务（此前端行为是带 originTaskId 创建）
        let resp = post(serde_json::json!({
            "title": "返工任务", "description": "按驳回意见调整", "trust": "manual",
            "context": { "origin_task_id": origin_id },
        }))
        .await;
        assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let new_id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["id"].as_str().unwrap().to_string();

        let origin = state.task_repo.get(&origin_id).await.unwrap().unwrap();
        assert_eq!(origin.successor_task_id.as_deref(), Some(new_id.as_str()), "原任务必须回填 successor 反链");
        let task = state.task_repo.get(&new_id).await.unwrap().unwrap();
        assert_eq!(task.origin_task_id.as_deref(), Some(origin_id.as_str()), "新任务必须记录返工来源");
        assert!(
            task.description.contains("方案风险过大"),
            "驳回理由必须由后端自动注入描述（三入口统一），实际: {}",
            task.description
        );

        // 埋点：task.created 事件带合约/模块计数
        use easyvibe_db::EventRepository as _;
        let summary = state.event_repo.summary(&repo).await.unwrap();
        let created = summary.iter().find(|r| r.name == "task.created");
        assert!(created.is_some(), "创建事件必须落库: {:?}", summary);
        assert!(created.unwrap().count >= 2);
    }

    #[tokio::test]
    async fn sessions_overview_route_registered() {
        // 2026-10-05 实弹回归（巡检后无运行状态显示的根因）：d928389 实现了 handler
        // 却漏注册路由，前端 404 静默兜底使状态丸/运行页永远空白——路由必须在版。
        let (state, _repo) = chat_state("sessions-overview-route").await;
        let app = build_router(state);
        let resp = app
            .oneshot(axum::http::Request::get("/api/sessions/overview").body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "/api/sessions/overview 必须已注册");
    }

    #[tokio::test]
    async fn event_ingest_validates_name_and_summarizes() {
        // R3 D1：前端埋点入库（dot.case 校验）+ 门控读数端点
        use easyvibe_db::EventRepository as _;
        let (state, repo) = chat_state("events").await;
        let app = build_router(state.clone());
        let post = |name: &str| {
            let app = app.clone();
            let repo = repo.clone();
            let name = name.to_string();
            async move {
                app.oneshot(
                    axum::http::Request::post(format!("/api/repos/{repo}/events"))
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(serde_json::json!({ "name": name, "payload": { "a": 1 } }).to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };
        let resp = post("ui.contractAlert.click").await;
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let resp = post("非法 名字!").await;
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST, "非法事件名必须 400");

        let resp = app
            .oneshot(axum::http::Request::get(format!("/api/repos/{repo}/events/summary")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let data = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"].clone();
        assert!(data.as_array().unwrap().iter().any(|r| r["name"] == "ui.contractAlert.click" && r["count"] == 1));
        // 仓储直读兜底
        let summary = state.event_repo.summary(&repo).await.unwrap();
        assert_eq!(summary.iter().find(|r| r.name == "ui.contractAlert.click").unwrap().count, 1);
    }

    #[tokio::test]
    async fn task_delete_cascades_and_retry_requeues() {
        // 管理闭环（2026-10-03 现状重审 P0）：删除（级联 approvals）+ 就地重试（failed → 重新入队）
        use easyvibe_db::{ApprovalRepository as _, TaskRepository as _};
        let (state, repo) = chat_state("task-admin").await;
        let app = build_router(state.clone());
        let post = |body: serde_json::Value| {
            let app = app.clone();
            let repo = repo.clone();
            async move {
                app.oneshot(
                    axum::http::Request::post(format!("/api/repos/{repo}/tasks"))
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };
        let create = |title: &str| {
            let post = post(serde_json::json!({ "title": title, "description": "管理闭环测试", "trust": "manual" }));
            async move {
                let resp = post.await;
                assert_eq!(resp.status(), axum::http::StatusCode::CREATED);
                let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
                serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["id"].as_str().unwrap().to_string()
            }
        };

        // ── 删除：造一条带审批留痕的任务，删后行与留痕都不存在 ──
        let id1 = create("将被删除").await;
        state.approval_repo.record(&easyvibe_db::ApprovalRow {
            id: format!("ap-{id1}-plan-x"), task_id: id1.clone(), gate: "plan".into(),
            decision: "approved".into(), note: Some("测试留痕".into()), decided_at: "0".into(),
        }).await.unwrap();
        let resp = app.clone().oneshot(
            axum::http::Request::delete(format!("/api/repos/{repo}/tasks/{id1}")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "删除已终态任务应成功");
        assert!(state.task_repo.get(&id1).await.unwrap().is_none(), "任务行必须删除");
        assert!(state.approval_repo.list_by_task(&id1).await.unwrap().is_empty(), "审批留痕必须级联清空");

        // 删除不存在 → 404
        let resp = app.clone().oneshot(
            axum::http::Request::delete(format!("/api/repos/{repo}/tasks/task-nope")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);

        // ── 重试：failed → pending →（manual  trust）重新停到 plan 关 ──
        let id2 = create("将重试").await;
        state.task_repo.update_status(&id2, "failed", Some("模拟失败")).await.unwrap();
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks/{id2}/retry")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "failed 任务应可重试");
        let t = state.task_repo.get(&id2).await.unwrap().unwrap();
        assert_eq!(t.status, "awaiting_approval", "manual 重试后重新停计划关");
        assert_eq!(t.gate.as_deref(), Some("plan"));
        assert!(t.error.is_none(), "重试必须清 error 残留");
        assert!(t.session_id.is_none(), "重试必须清会话残留");

        // 非 failed/interrupted 重试 → 409
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks/{id2}/retry")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT, "awaiting_approval 不可重试");
    }
