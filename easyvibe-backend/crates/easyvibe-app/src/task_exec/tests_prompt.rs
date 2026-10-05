//! task_exec 测试（仅测试编译）。

use super::*;
use super::test_util::*;

    #[test]
    fn prompt_assembles_all_parts() {
        let dir = std::env::temp_dir().join("ev-harness-test2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("inject-prompt.md"), "框架内容 cat ~/.claude/hooks/rule_development.md").unwrap();
        write_manifest(&dir, &[], &[]);
        let harness = load_harness_from(&dir).unwrap();
        assert!(
            harness.framework_transparent.contains(&format!("{}/rule_development.md", dir.display())),
            "路径换姓生效"
        );

        let prompt = assemble_task_prompt(&harness.framework_transparent, &sample_task("pending"));
        assert!(prompt.contains("框架内容"));
        assert!(prompt.contains("把双向依赖改为单向"));
        assert!(prompt.contains("m1"));
        assert!(prompt.contains("无新增逆向"));
        assert!(prompt.contains("[EASYVIBE-RESULT]"));
        assert!(prompt.contains("\"inject\""));
    }

    #[test]
    fn phase_review_prompt_targets_right_dir_and_readonly() {
        // 初审 prompt：阶段 1/2 指向各自产物目录；只审不改纪律与结论协议行必备
        let t = sample_task("running");
        let p1 = assemble_phase_review_prompt(&t, 1, "liyuhang");
        assert!(p1.contains("1_requirements_matrix/"));
        assert!(p1.contains("只审不改"));
        assert!(p1.contains("[EASYVIBE-REVIEW]"));
        assert!(p1.contains("把双向依赖改为单向"), "任务书必须注入作对照基准");
        let p2 = assemble_phase_review_prompt(&t, 2, "liyuhang");
        assert!(p2.contains("2_requirements_solutions/"));
    }

    #[test]
    fn adapt_renames_all_three_claude_forms() {
        let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
        // 形态 1：inject-prompt.md 的 task.json 路径（user_name 在 development_docs 之前）
        let s1 = adapt_builtin_content(".claude/<user_name>/development_docs/task_260928_x.json");
        assert_eq!(s1, format!(".easyvibe/development_docs/{user}/task_260928_x.json"), "形态1换姓");
        // 形态 2：规则正文的产物路径（20+ 处）
        let s2 = adapt_builtin_content(".claude/development_docs/<user_name>/1_requirements_matrix/x.md");
        assert_eq!(s2, format!(".easyvibe/development_docs/{user}/1_requirements_matrix/x.md"), "形态2换姓");
        // 形态 3：附录目录树的裸 .claude/ 根行（复审残留）
        let s3 = adapt_builtin_content(".claude/\n├── development_docs/");
        assert_eq!(s3, ".easyvibe/\n├── development_docs/", "形态3换姓");
        // hooks 锚点保护：load 时替换链（load_harness_from）依赖 ~/.claude/hooks/ 原样存在
        let s4 = adapt_builtin_content("cat ~/.claude/hooks/rule_development.md");
        assert_eq!(s4, "cat ~/.claude/hooks/rule_development.md", "hooks 锚点不得换姓");
    }

    #[test]
    fn version_gt_semantics() {
        assert!(version_gt("1.2.0", "1.1.0"));
        assert!(version_gt("2.0.0", "1.10.0"), "数值段比较，非字典序");
        assert!(!version_gt("1.2.0", "1.2.0"));
        assert!(!version_gt("1.1.0", "1.2.0"));
        assert!(!version_gt("1.2", "1.2.0"), "缺段按 0，相等");
    }
