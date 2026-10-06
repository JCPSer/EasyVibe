//! task_exec 测试共用夹具（仅测试编译）。

use super::*;

    pub(crate) fn sample_task(status: &str) -> TaskRecord {
        TaskRecord {
            id: "task-t1".into(),
            repo: "demo".into(),
            title: "修复耦合".into(),
            description: "把双向依赖改为单向".into(),
            modules: "[\"m1\"]".into(),
            acceptance: "无新增逆向".into(),
            source: "concern".into(),
            context: "{\"inject\":{\"module\":{\"id\":\"m1\"}}}".into(),
            status: status.into(),
            trust: "manual".into(),
            error: None,
            session_id: None,
            gate: None,
            conversation_id: None,
            prompt_tokens: None,
            completion_tokens: None,
            result: None,
            base_head: None,
            created_at: "1".into(),
            updated_at: "1".into(),
            origin_task_id: None,
            successor_task_id: None,
        }
    }

    /// TaskRecord（切片内 DTO）→ 持久层任务行：测试夹具用真实 SQLite 仓储落库时使用。
    pub(crate) fn as_row(t: &TaskRecord) -> easyvibe_db::TaskRow {
        easyvibe_db::TaskRow {
            id: t.id.clone(), repo: t.repo.clone(), title: t.title.clone(), description: t.description.clone(),
            modules: t.modules.clone(), acceptance: t.acceptance.clone(), source: t.source.clone(), context: t.context.clone(),
            status: t.status.clone(), trust: t.trust.clone(), error: t.error.clone(), session_id: t.session_id.clone(),
            gate: t.gate.clone(), prompt_tokens: t.prompt_tokens, completion_tokens: t.completion_tokens,
            result: t.result.clone(), base_head: t.base_head.clone(), created_at: t.created_at.clone(),
            updated_at: t.updated_at.clone(), conversation_id: t.conversation_id.clone(),
            origin_task_id: t.origin_task_id.clone(), successor_task_id: t.successor_task_id.clone(),
        }
    }

    /// 夹具：把切片内 TaskRecord 落库（隔离两个同名 trait 的方法解析歧义）。
    pub(crate) async fn db_create(r: &easyvibe_db::SqliteTaskRepository, t: &TaskRecord) {
        easyvibe_db::TaskRepository::create(r, &as_row(t)).await.unwrap();
    }

    /// 夹具：按 id 读持久层任务行（返回持久层行，测试断言字段用）。
    pub(crate) async fn db_get(r: &easyvibe_db::SqliteTaskRepository, id: &str) -> Option<easyvibe_db::TaskRow> {
        easyvibe_db::TaskRepository::get(r, id).await.unwrap()
    }

    /// 夹具：按任务读审批留痕（持久层行）。
    pub(crate) async fn db_approvals(r: &easyvibe_db::SqliteApprovalRepository, task_id: &str) -> Vec<easyvibe_db::ApprovalRow> {
        easyvibe_db::ApprovalRepository::list_by_task(r, task_id).await.unwrap()
    }

    /// 测试桩：最小 harness（只有框架正文，无 skill/中和）
    pub(crate) fn harness_stub(framework: &str) -> Arc<tokio::sync::RwLock<Harness>> {
        Arc::new(tokio::sync::RwLock::new(Harness {
            dir: std::env::temp_dir(),
            manifest: HarnessManifest {
                id: "stub".into(),
                version: "0".into(),
                builtin: false,
                route_rules: vec![],
                skills: HarnessSkills::default(),
                transparent_neutralize: vec![],
            },
            framework_transparent: framework.into(),
            user_entry_skills: vec![],
            custom: HarnessCustom::default(),
            custom_neutral: HarnessCustom::default(),
            rule_development: String::new(),
            rule_bugfix: String::new(),
        }))
    }

    pub(crate) fn write_manifest(dir: &std::path::Path, neutralize: &[&str], user_entry: &[&str]) {
        let m = serde_json::json!({
            "id": "test-harness", "version": "1.0.0",
            "routeRules": [], "resultProtocol": "[EASYVIBE-RESULT]",
            "skills": { "userEntry": user_entry, "transparent": [] },
            "transparentNeutralize": neutralize,
        });
        std::fs::write(dir.join("manifest.json"), serde_json::to_string(&m).unwrap()).unwrap();
    }
