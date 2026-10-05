//! main.rs 内联测试的共享夹具（仅测试编译）。

use easyvibe_api_types::SessionStatusChanged;
use easyvibe_map::{repo_from_root, MapService};
use easyvibe_session::SessionManager;
use std::sync::Arc;
use tokio::sync::broadcast;
use crate::state::*;
use crate::task_exec;
use tower::ServiceExt;

    /// 样例地图（与 ai-agent 测试同构：validate_minimum 可通过，stub 问答可命中）
    pub(crate) const SAMPLE_MAP: &str = r#"{
      "version": "1.0",
      "meta": {"repo": "demo", "generated_at": "t", "generator": "g/test"},
      "layers": [{"id": "application", "name": "应用服务层", "order": 0, "description": "d"}],
      "modules": [{
        "id": "exam-core", "name": "考试与评测核心", "layer": "application",
        "responsibility": "考试会话编排、答题流程与评测提交管线",
        "files": ["lib/**"], "key_entries": [], "dependencies": [],
        "health": {"score": 64, "coupling": "high", "complexity": "high", "churn": "medium",
                   "decay_flags": [], "review_note": "n", "concerns": []}
      }],
      "edges": [],
      "health": {"score": 58, "coupling": "high", "complexity": "high", "churn": "high",
                 "decay_flags": [], "review_note": "r", "concerns": []}
    }"#;

    pub(crate) async fn test_state() -> AppState {
        test_state_with(MapService::new(vec![])).await
    }

    pub(crate) async fn test_state_with(svc: std::sync::Arc<MapService>) -> AppState {
        let (bus, _) = broadcast::channel(8);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let session_manager = SessionManager::new(tx);
        // 巡检槽位用内存库（测试不落盘）
        let db = easyvibe_db::Database::connect_memory().await.unwrap();
        let health_repo = Arc::new(easyvibe_db::SqliteHealthRepository::new(db.pool().clone()));
        let patrol_service = Arc::new(easyvibe_ai_agent::PatrolService::new(health_repo.clone()));
        let settings_repo = Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone()));
        let task_repo = Arc::new(easyvibe_db::SqliteTaskRepository::new(db.pool().clone()));
        let approval_repo = Arc::new(easyvibe_db::SqliteApprovalRepository::new(db.pool().clone()));
        let _agent_session_repo = Arc::new(easyvibe_db::AgentSessionRepo::new(db.pool().clone()));
        let conversation_repo = Arc::new(easyvibe_db::SqliteConversationRepository::new(db.pool().clone()));
        let harness = Arc::new(tokio::sync::RwLock::new(task_exec::Harness {
            dir: std::env::temp_dir(),
            manifest: task_exec::HarnessManifest {
                id: "stub".into(), version: "0".into(), builtin: false,
                route_rules: vec![], skills: task_exec::HarnessSkills::default(), transparent_neutralize: vec![],
            },
            framework_transparent: "框架".into(),
            user_entry_skills: vec![],
            custom: task_exec::HarnessCustom::default(),
            // 测试夹具必填：`Harness.custom_neutral` 由 HEAD fd080d5 引入（harness.rs），
            // 此处不补齐则 easyvibe-app 测试目标 E0063 无法编译（既有缺陷，非本任务运行时改动）。
            custom_neutral: task_exec::HarnessCustom::default(),
            rule_development: String::new(),
            rule_bugfix: String::new(),
        }));
        let executor = task_exec::TaskExecutor::new(
            task_repo.clone(),
            approval_repo.clone(),
            session_manager.clone(),
            svc.clone(),
            harness.clone(),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            settings_repo.clone(),
            None,
        );
        let cipher = easyvibe_common::SecretCipher::from_hex_key(&"ab".repeat(32)).unwrap();
        AppState {
            map_service: svc,
            session_manager,
            prompt_template: Arc::new("test".into()),
            agent_command: Arc::new("true".into()),
            agent_args: Arc::new(vec![]),
            patrol_service,
            health_repo,
            agent_session_repo: Arc::new(easyvibe_db::AgentSessionRepo::new(db.pool().clone())),
            session_output_repo: Arc::new(easyvibe_db::SessionOutputRepo::new(db.pool().clone())),
            settings_repo,
            cipher: Arc::new(cipher),
            task_repo,
            approval_repo,
            conversation_repo,
            event_repo: Arc::new(easyvibe_db::SqliteEventRepository::new(db.pool().clone())),
            chat_lock: Arc::new(tokio::sync::Mutex::new(())),
            executor,
            harness,
            llm_mode: Arc::new(LlmMode::Stub),
            patrol_prompt: Arc::new("test".into()),
            submap_prompt: Arc::new("test".into()),
            incremental_prompt: Arc::new("test".into()),
            schema_path: Arc::new("schema.json".into()),
            event_bus: bus,
            pool: db.pool().clone(),
            agent_detected: Default::default(),
            agent_test: Default::default(),
            agent_test_lock: Default::default(),
            session_queue: Default::default(),
        }
    }

    /// 带样例地图的测试仓库（chat E2E 用）
    pub(crate) async fn chat_state(tag: &str) -> (AppState, String) {
        let dir = std::env::temp_dir().join(format!("ev-chat-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".easyvibe/map")).unwrap();
        std::fs::write(dir.join(".easyvibe/map/map.json"), SAMPLE_MAP).unwrap();
        let repo = repo_from_root(&dir);
        let repo_id = repo.id.clone();
        (test_state_with(MapService::new(vec![repo])).await, repo_id)
    }

    pub(crate) async fn post_chat(app: &axum::Router, repo: &str, message: &str) -> serde_json::Value {
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/chat"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::json!({ "message": message }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK, "chat POST 应成功");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice::<serde_json::Value>(&body).unwrap()
    }

    pub(crate) fn urlencoding_encode(s: &str) -> String {
        s.bytes().map(|b| format!("%{:02X}", b)).collect()
    }

    pub(crate) async fn queue_state(tag: &str) -> (AppState, String) {
        chat_state(tag).await
    }

    pub(crate) async fn post_queue(app: &axum::Router, repo: &str, body: &str) -> (axum::http::StatusCode, serde_json::Value) {
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::post(format!("/api/repos/{repo}/session-queue"))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let v = if status == axum::http::StatusCode::NO_CONTENT {
            serde_json::Value::Null
        } else {
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        (status, v)
    }

    pub(crate) async fn get_queue(app: &axum::Router, repo: &str) -> serde_json::Value {
        let resp = app
            .clone()
            .oneshot(axum::http::Request::get(format!("/api/repos/{repo}/session-queue")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["data"].clone()
    }

    pub(crate) async fn delete_queue(app: &axum::Router, repo: &str) -> axum::http::StatusCode {
        let resp = app
            .clone()
            .oneshot(axum::http::Request::delete(format!("/api/repos/{repo}/session-queue")).body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        resp.status()
    }

    pub(crate) fn running(sid: &str) -> SessionStatusChanged {
        SessionStatusChanged { repo: String::new(), session_id: sid.into(), status: easyvibe_api_types::SessionStatus::Running }
    }
