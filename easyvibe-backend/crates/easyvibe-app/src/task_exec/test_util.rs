//! task_exec 测试共用夹具（仅测试编译）。

use super::*;

    pub(crate) fn sample_task(status: &str) -> TaskRow {
        TaskRow {
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
