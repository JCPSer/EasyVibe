//! main.rs 内联测试（迁移自 god file，仅测试编译）——分片 4/4。

use easyvibe_api_types::{MapChanged, MapInvalid, SessionStatusChanged};
use easyvibe_common::ApiError;
use easyvibe_event_bus::queue::{JobKind, QueuedJob};
use easyvibe_event_bus::BusEvent;
use crate::router::*;
use crate::state::*;
use crate::test_support::*;
use crate::ws::translate;
use tower::ServiceExt;

    #[tokio::test]
    async fn task_remediate_injects_feedback_and_respawns() {
        // 修改并复审闭环：rejected（子 agent 审查打回）→ 注入审查意见 → 直达实施阶段重跑
        use easyvibe_db::TaskRepository as _;
        let (state, repo) = chat_state("task-remediate").await;
        let app = build_router(state.clone());
        let create = async {
            let resp = app.clone().oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/tasks"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::json!({ "title": "被审查打回", "description": "实施后有越界", "trust": "manual" }).to_string()))
                    .unwrap(),
            ).await.unwrap();
            let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["id"].as_str().unwrap().to_string()
        };
        let tid = create.await;
        // 计划关打回（留驳回意见——复审反馈的来源）
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/decide"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::json!({ "decision": "rejected", "note": "根 .gitignore 越界改动须剥离", "gate": "plan" }).to_string()))
                .unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        // 修改并复审
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/remediate")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "rejected 任务应可修改复审");
        let t = state.task_repo.get(&tid).await.unwrap().unwrap();
        assert_eq!(t.status, "running", "复审直达实施阶段");
        assert_eq!(t.gate.as_deref(), Some("p:implement"));
        let ctx: serde_json::Value = serde_json::from_str(&t.context).unwrap();
        assert_eq!(ctx["remediation"]["round"], 1, "复审轮次记录");
        assert!(ctx["remediation"]["review_feedback"].as_str().unwrap().contains("越界"), "审查意见必须注入 context");
        // 非 rejected 状态再审 → 409
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/remediate")).body(axum::body::Body::empty()).unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT, "running 任务不可再审");
        // 复审执行（agent "true" 秒退）→ 走完实施链回到审批流
        let mut t = state.task_repo.get(&tid).await.unwrap().unwrap();
        for _ in 0..40 {
            if matches!(t.status.as_str(), "awaiting_approval" | "rejected" | "done" | "failed") && t.gate.as_deref() != Some("p:implement") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = state.task_repo.get(&tid).await.unwrap().unwrap();
        }
        assert_ne!(t.gate.as_deref(), Some("p:implement"), "复审应跑完实施阶段");
    }

    #[tokio::test]
    async fn task_reject_at_analysis_gate_reworks_phase_with_feedback() {
        // 2026-10-05 打回闭环（用户裁定：评审关不通过=带意见原地重跑本阶段，
        // 与需求分析/方案设计同一机制，覆盖代码审查 diff/report 关）：
        // analysis 关打回 → running+p:analysis + remediation 注入 → 重跑完成 → 回到 analysis 关可再审。
        use easyvibe_db::{ApprovalRepository as _, TaskRepository as _};
        let (state, repo) = chat_state("task-rework").await;
        let app = build_router(state.clone());
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::json!({ "title": "矩阵打回", "description": "做一个东西", "trust": "manual" }).to_string()))
                .unwrap(),
        ).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let tid = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["data"]["id"].as_str().unwrap().to_string();

        // 批准计划关 → 阶段1 跑完（agent "true" 秒退，初审不可用不阻断）→ 停 analysis 关
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/decide"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::json!({ "decision": "approved", "note": "开工", "gate": "plan" }).to_string()))
                .unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let mut t = state.task_repo.get(&tid).await.unwrap().unwrap();
        for _ in 0..40 {
            if t.status == "awaiting_approval" && t.gate.as_deref() == Some("analysis") { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = state.task_repo.get(&tid).await.unwrap().unwrap();
        }
        assert_eq!(t.status, "awaiting_approval", "阶段1 应停在 analysis 评审关");
        assert_eq!(t.gate.as_deref(), Some("analysis"));

        // 打回（带意见）→ 不是终态，而是带意见重跑阶段1
        let resp = app.clone().oneshot(
            axum::http::Request::post(format!("/api/repos/{repo}/tasks/{tid}/decide"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(serde_json::json!({ "decision": "rejected", "note": "验收标准缺边界用例", "gate": "analysis" }).to_string()))
                .unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "评审关打回应成功");
        let t = state.task_repo.get(&tid).await.unwrap().unwrap();
        assert_eq!(t.status, "running", "打回=原地重跑，不落终态 rejected");
        assert_eq!(t.gate.as_deref(), Some("p:analysis"), "重跑阶段1");
        let ctx: serde_json::Value = serde_json::from_str(&t.context).unwrap();
        assert_eq!(ctx["remediation"]["round"], 1, "打回轮次记录");
        assert!(ctx["remediation"]["review_feedback"].as_str().unwrap().contains("边界用例"), "打回意见必须注入 context");

        // 重跑完成 → 回到 analysis 关等再审（闭环可循环）
        let mut t = state.task_repo.get(&tid).await.unwrap().unwrap();
        for _ in 0..40 {
            if t.status == "awaiting_approval" && t.gate.as_deref() == Some("analysis") { break; }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = state.task_repo.get(&tid).await.unwrap().unwrap();
        }
        assert_eq!(t.status, "awaiting_approval", "重跑后应回到 analysis 评审关");
        assert_eq!(t.gate.as_deref(), Some("analysis"));
        // 留痕：审批轨迹里可看到打回记录（评审轮回的证据）
        let aps = state.approval_repo.list_by_task(&tid).await.unwrap();
        assert!(
            aps.iter().any(|a| a.gate == "analysis" && a.decision == "rejected" && a.note.as_deref().unwrap_or("").contains("边界用例")),
            "打回必须留痕，实际: {:?}",
            aps.iter().map(|a| (a.gate.clone(), a.decision.clone())).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn agent_status_detect_test_endpoints() {
        // M1 端点：status 形状 / test 协议判定（echo 兼容 / false 非 0 退出不兼容）
        use easyvibe_db::SettingsRepository as _;
        let (state, _repo) = chat_state("agent-endpoints").await;
        let app = build_router(state.clone());
        // status：预设目录由后端提供（前端不硬编码）
        let resp = app.clone().oneshot(axum::http::Request::get("/api/agent/status").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        let presets = v["data"]["presets"].as_array().unwrap();
        assert_eq!(presets.len(), 3, "本期预设三家");
        assert!(presets.iter().any(|p| p["id"] == "claude" && p["stability"] == "stable"));
        assert!(presets.iter().any(|p| p["id"] == "codex" && p["stability"] == "experimental"));
        assert!(v["data"]["effective"]["found"].is_boolean());

        // test：/bin/echo hello → 退出 0 有输出 → compatible
        state.settings_repo.set(&easyvibe_db::SettingRow {
            scope: "global".into(), key: "agent.command".into(), value: "/bin/echo".into(),
            encrypted: false, updated_at: "t".into(),
        }).await.unwrap();
        state.settings_repo.set(&easyvibe_db::SettingRow {
            scope: "global".into(), key: "agent.args.global".into(), value: r#"["hello"]"#.into(),
            encrypted: false, updated_at: "t".into(),
        }).await.unwrap();
        let resp = app.clone().oneshot(axum::http::Request::post("/api/agent/test").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        assert_eq!(v["data"]["protocol"], "compatible", "echo 应兼容: {:?}", v);
        assert!(v["data"]["ok"].as_bool().unwrap());

        // test：/bin/false → 非 0 退出 → incompatible（附退出码语义）
        state.settings_repo.set(&easyvibe_db::SettingRow {
            scope: "global".into(), key: "agent.command".into(), value: "/bin/false".into(),
            encrypted: false, updated_at: "t".into(),
        }).await.unwrap();
        state.settings_repo.set(&easyvibe_db::SettingRow {
            scope: "global".into(), key: "agent.args.global".into(), value: "[]".into(),
            encrypted: false, updated_at: "t".into(),
        }).await.unwrap();
        let resp = app.clone().oneshot(axum::http::Request::post("/api/agent/test").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        assert!(!v["data"]["ok"].as_bool().unwrap());
        assert!(v["data"]["protocol"].as_str().unwrap().starts_with("incompatible"), "false 应不兼容: {:?}", v);

        // status 反映最近测试结果
        let resp = app.oneshot(axum::http::Request::get("/api/agent/status").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v = serde_json::from_slice::<serde_json::Value>(&body).unwrap();
        assert_eq!(v["data"]["protocolOk"], false, "status 应带出最近测试结论");
    }

    #[test]
    fn agent_command_resolves_to_absolute_path() {
        // 桌面壳实弹回归（hover-client 三任务"spawn claude 失败"）：GUI 薄 PATH 下必须解析出绝对路径
        let p = resolve_agent_command("claude");
        assert!(std::path::Path::new(&p).is_file(), "claude 必须解析为真实存在的绝对路径，实际: {p}");
        assert_eq!(resolve_agent_command("/bin/echo"), "/bin/echo", "已含路径且存在的命令原样返回");
    }

    // ---------- 运行会话排队（需求 v1 §5/§8 + 评审裁决 B1/B2/B4） ----------

    /// B4：POST 时无活动会话 → 后端直接代执行（started:true），队列保持空
    #[tokio::test]
    async fn session_queue_post_executes_directly_without_active() {
        let (state, repo) = queue_state("q-b4").await;
        let app = build_router(state.clone());
        let (status, v) = post_queue(&app, &repo, r#"{"kind":"patrol"}"#).await;
        assert_eq!(status, axum::http::StatusCode::ACCEPTED);
        assert_eq!(v["data"]["started"], true, "无活动会话必须直接执行: {v}");
        assert!(v["data"]["queued"].is_null());
        assert!(state.session_queue.is_empty().await, "直接执行不得留队列项");
        // 代执行真实发生了：状态被新会话接管（inner 在响应返回前已同步完成注册；
        // stub 巡检毫秒级收尾，故只断言「会话已接续」这一稳定事实，label 契约由 GET 用例覆盖）
        let mut saw = false;
        for _ in 0..40 {
            if let Some(s) = state.session_manager.status_of(&repo).await {
                if s.session_id != "ind-1" {
                    saw = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(saw, "直接执行后应出现接续的巡检会话");
    }

    /// 入队/替换（响应带旧 label）/取消；DELETE 404；GET 契约；非法 body 400
    #[tokio::test]
    async fn session_queue_enqueue_replace_cancel_contract() {
        let (state, repo) = queue_state("q-crud").await;
        let app = build_router(state.clone());
        // 占一个活动会话（带标签与 startedAt 戳）
        let mut s1 = running("ind-1");
        s1.repo = repo.clone();
        state.session_manager.try_register(s1).await.unwrap();
        state.session_manager.note_label("ind-1", "归纳".into()).await;

        // 初始 GET：active 齐、queued 空
        let d = get_queue(&app, &repo).await;
        assert_eq!(d["active"]["sessionId"], "ind-1");
        assert_eq!(d["active"]["label"], "归纳");
        assert!(d["active"]["startedAt"].is_string());
        assert!(d["queued"].is_null());

        // 入队 → 202 queued:true replaced:null
        let (status, v) = post_queue(&app, &repo, r#"{"kind":"patrol"}"#).await;
        assert_eq!(status, axum::http::StatusCode::ACCEPTED);
        assert_eq!(v["data"]["queued"], true);
        assert!(v["data"]["replaced"].is_null(), "首次入队 replaced 必须 null: {v}");
        let d = get_queue(&app, &repo).await;
        assert_eq!(d["queued"]["kind"], "patrol");
        assert_eq!(d["queued"]["label"], "巡检");

        // 替换 → 响应带被替换项 label（S3）
        let (status, v) = post_queue(&app, &repo, r#"{"kind":"submap","moduleId":"exam-core"}"#).await;
        assert_eq!(status, axum::http::StatusCode::ACCEPTED);
        assert_eq!(v["data"]["replaced"]["label"], "巡检", "替换必须携带旧 label: {v}");
        let d = get_queue(&app, &repo).await;
        assert_eq!(d["queued"]["kind"], "submap");
        assert_eq!(d["queued"]["moduleId"], "exam-core");
        assert!(d["queued"]["enqueuedAt"].is_string());

        // 非法 body → 400
        let (status, _) = post_queue(&app, &repo, r#"{"kind":"task"}"#).await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "未知类型必须 400");
        let (status, _) = post_queue(&app, &repo, r#"{"kind":"submap"}"#).await;
        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST, "submap 缺 moduleId 必须 400");

        // 取消 → 204；再取消 → 404（带错误文案）
        assert_eq!(delete_queue(&app, &repo).await, axum::http::StatusCode::NO_CONTENT);
        let resp = app
            .clone()
            .oneshot(axum::http::Request::delete(format!("/api/repos/{repo}/session-queue")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND, "无排队必须 404");
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap();
        assert!(v["error"].as_str().unwrap().contains("无排队任务"), "404 文案: {}", v["error"]);
    }

    /// drain 正常路径：活动会话终态 → drain pop + 代执行，队列清空、巡检接续启动
    #[tokio::test]
    async fn session_queue_drain_starts_queued_job_after_terminal() {
        let (state, repo) = queue_state("q-drain").await;
        let app = build_router(state.clone());
        let mut s1 = running("ind-1");
        s1.repo = repo.clone();
        state.session_manager.try_register(s1).await.unwrap();
        state.session_manager.note_label("ind-1", "归纳".into()).await;
        let (_status, v) = post_queue(&app, &repo, r#"{"kind":"patrol"}"#).await;
        assert_eq!(v["data"]["queued"], true);

        // 活动会话终态 → drain（事件循环/清扫器触发的就是这个函数）
        state
            .session_manager
            .note_status(SessionStatusChanged { repo: repo.clone(), session_id: "ind-1".into(), status: easyvibe_api_types::SessionStatus::Succeeded })
            .await;
        assert!(state.session_queue.drain(&state, &repo).await, "终态后必须 drain");
        assert!(state.session_queue.is_empty().await, "drain 必须 pop 队列项");

        // 接续执行真实发生：仓库状态被新会话接管（stub 巡检毫秒级收尾，
        // 故只断言「会话已接续」这一稳定事实）
        let mut saw = false;
        for _ in 0..40 {
            if let Some(s) = state.session_manager.status_of(&repo).await {
                if s.session_id != "ind-1" {
                    saw = true;
                    break;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(saw, "drain 后巡检应接续启动");
    }

    /// B2：drain 撞 Conflict → 放回原槽位且不覆盖期间用户新排的队；确定性失败 → 丢弃 + 广播 failed
    #[tokio::test]
    async fn session_queue_drain_failure_grading() {
        let (state, repo) = queue_state("q-b2").await;
        let app = build_router(state.clone());
        let mut rx = state.event_bus.subscribe();
        let mut s1 = running("ind-1");
        s1.repo = repo.clone();
        state.session_manager.try_register(s1).await.unwrap();
        // 入队「巡检」→ 再替换为「归纳」（用户期间改主意）
        let (_s, v) = post_queue(&app, &repo, r#"{"kind":"patrol"}"#).await;
        assert_eq!(v["data"]["queued"], true);
        let (_s, v) = post_queue(&app, &repo, r#"{"kind":"reinduce"}"#).await;
        assert_eq!(v["data"]["replaced"]["label"], "巡检");

        // drain 的 patrol 项撞 Conflict（TOCTOU：任务槽抢注）→ 不得覆盖用户新排的「归纳」
        let patrol = QueuedJob { kind: JobKind::Patrol, module_id: None, label: "巡检".into(), enqueued_at: chrono::Utc::now(), force_full: false };
        state.session_queue.handle_failure(&state, &repo, patrol, ApiError::Conflict("仓库有活动会话".into())).await;
        let kept = state.session_queue.peek(&repo).await.expect("Conflict 不得丢项");
        assert_eq!(kept.kind, JobKind::Reinduce, "不得覆盖期间用户新排的队: {:?}", kept.kind);
        // 槽位空时 Conflict → 放回原项（不丢）
        let _ = state.session_queue.cancel(&state, &repo).await;
        let patrol2 = QueuedJob { kind: JobKind::Patrol, module_id: None, label: "巡检".into(), enqueued_at: chrono::Utc::now(), force_full: false };
        state.session_queue.handle_failure(&state, &repo, patrol2, ApiError::Conflict("抢注".into())).await;
        assert_eq!(state.session_queue.peek(&repo).await.unwrap().label, "巡检");

        // 确定性失败 → 丢弃 + 广播 queue.changed{type:"failed", error}
        // （先清槽模拟 drain 已 pop 出该项——handle 只处置被弹出的项，不动槽内其他排队）
        let _ = state.session_queue.cancel(&state, &repo).await;
        let doomed = QueuedJob { kind: JobKind::Submap, module_id: Some("exam-core".into()), label: "分析模块 exam-core".into(), enqueued_at: chrono::Utc::now(), force_full: false };
        state.session_queue.handle_failure(&state, &repo, doomed, ApiError::BadRequest("agent 缺失".into())).await;
        assert!(state.session_queue.is_empty().await, "确定性失败必须丢弃（不留死信）");
        // 广播核验：从事件流里捞 queue.changed（enqueued/replaced/requeued/failed 都应出现过）
        let mut saw_failed = false;
        while let Ok(e) = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv()).await {
            let Ok(BusEvent::QueueChanged { change, .. }) = e else { continue };
            if change.type_str() == "failed" {
                saw_failed = true;
                break;
            }
        }
        assert!(saw_failed, "确定性失败必须广播 queue.changed failed");
    }

    /// B1 app 级：grace 迟到 Succeeded 不得顶掉新活动会话；drain 复查活动会话，不 pop 不误接续
    #[tokio::test]
    async fn grace_late_terminal_keeps_new_session_and_skips_drain() {
        let (state, repo) = queue_state("q-b1").await;
        let app = build_router(state.clone());
        let mgr = &state.session_manager;
        // S1 走完终态（归纳失败/成功皆可）→ S2 抢注为新活动会话
        let mut s1 = running("ind-a");
        s1.repo = repo.clone();
        mgr.try_register(s1).await.unwrap();
        mgr.note_label("ind-a", "归纳".into()).await;
        mgr.note_status(SessionStatusChanged { repo: repo.clone(), session_id: "ind-a".into(), status: easyvibe_api_types::SessionStatus::Failed }).await;
        let mut s2 = running("ind-b");
        s2.repo = repo.clone();
        mgr.try_register(s2).await.unwrap();
        // grace 收尸迟到：ind-a 的 Succeeded 不得覆写 ind-b
        mgr.note_status(SessionStatusChanged { repo: repo.clone(), session_id: "ind-a".into(), status: easyvibe_api_types::SessionStatus::Succeeded }).await;
        let active = mgr.status_of(&repo).await.expect("仓库应有活动会话");
        assert_eq!(active.session_id, "ind-b", "迟到 Succeeded 不得顶掉新活动会话");
        assert_eq!(active.status, easyvibe_api_types::SessionStatus::Running);
        // B1 drain 收紧：事件 session_id（ind-a）经 status_of_session 确认为终态（护栏丢弃后 by_id 仍 Failed）
        let by_id = mgr.status_of_session("ind-a").await.expect("by_id 必须有 ind-a 的终态归属");
        assert!(matches!(by_id.status, easyvibe_api_types::SessionStatus::Failed | easyvibe_api_types::SessionStatus::Succeeded));

        // 事件循环的条件分支成立 → 调 drain；drain 复查发现 ind-b 活动 → 不 pop、不接续
        let (_s, v) = post_queue(&app, &repo, r#"{"kind":"patrol"}"#).await;
        assert_eq!(v["data"]["queued"], true, "ind-b 活动期间 POST 必须入队");
        assert!(!state.session_queue.drain(&state, &repo).await, "有活动会话不得 drain");
        assert!(state.session_queue.peek(&repo).await.is_some(), "队项必须保留等下一终态");
        let active = mgr.status_of(&repo).await.unwrap();
        assert_eq!(active.session_id, "ind-b", "drain 不得误接续");
    }

    /// R7：WS 契约快照——BusEvent → WsMessage 的事件名与关键载荷字段必须稳定，
    /// 前端 App.tsx 按 msg.name 匹配，漂移即静默失效（丢弃无提示）。
    #[test]
    fn ws_translate_event_name_snapshot() {
        let cases: Vec<(BusEvent, &str)> = vec![
            (BusEvent::MapChanged(MapChanged { repo: "r".into(), version: "v".into() }), "map.changed"),
            (BusEvent::MapInvalid(MapInvalid { repo: "r".into(), error: "e".into() }), "map.invalid"),
            (BusEvent::Growth { repo: "r".into(), event: serde_json::json!({}) }, "growth.event"),
            (BusEvent::Progress { repo: "r".into(), progress: serde_json::json!({}) }, "progress.updated"),
            (BusEvent::SessionStatus(SessionStatusChanged { repo: "r".into(), session_id: "s".into(), status: easyvibe_api_types::SessionStatus::Running }), "session.statusChanged"),
            (BusEvent::TaskStatus { repo: "r".into(), task_id: "t".into(), status: "running".into(), gate: None }, "task.statusChanged"),
            (BusEvent::TaskContractViolated { repo: "r".into(), task_id: "t".into(), files: vec!["a".into()] }, "task.contractViolated"),
            (BusEvent::TaskContractAlert { repo: "r".into(), task_id: "t".into(), files: vec![] }, "task.contractAlert"),
            (BusEvent::Freshness { repo: "r".into(), status: "stale".into(), latest_commit_at: Some(1), commits_since_map: Some(2) }, "freshness.changed"),
            (BusEvent::SessionOutput { session_id: "s".into(), seq: 1, stream: "stdout".into(), line: "x".into() }, "session.output"),
            (BusEvent::PatrolFinished { repo: "r".into(), run_id: "run".into(), status: "ok".into() }, "patrol.finished"),
            (BusEvent::QueueChanged { repo: "r".into(), change: easyvibe_event_bus::queue::QueueChange::Enqueued {
                job: QueuedJob { kind: JobKind::Patrol, module_id: None, label: "巡检".into(), enqueued_at: chrono::Utc::now(), force_full: false } } }, "queue.changed"),
        ];
        for (ev, want) in cases {
            assert_eq!(translate(ev).name, want);
        }
        // 关键载荷字段（task.* / queue.* 前端直接消费）
        let m = translate(BusEvent::TaskStatus { repo: "r".into(), task_id: "t".into(), status: "running".into(), gate: Some("p:implement".into()) });
        assert_eq!(m.data["taskId"], "t");
        assert_eq!(m.data["status"], "running");
        assert_eq!(m.data["gate"], "p:implement");
        let q = translate(BusEvent::QueueChanged { repo: "r".into(), change: easyvibe_event_bus::queue::QueueChange::Failed {
            job: QueuedJob { kind: JobKind::Submap, module_id: Some("m".into()), label: "分析模块 m".into(), enqueued_at: chrono::Utc::now(), force_full: false },
            error: "boom".into() } });
        assert_eq!(q.data["type"], "failed");
        assert_eq!(q.data["repo"], "r");
        assert_eq!(q.data["job"]["moduleId"], "m");
        assert_eq!(q.data["error"], "boom");
    }
