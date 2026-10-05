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

        let prompt = assemble_task_prompt(&harness, &sample_task("pending"));
        assert!(prompt.contains("框架内容"));
        assert!(prompt.contains("把双向依赖改为单向"));
        assert!(prompt.contains("m1"));
        assert!(prompt.contains("无新增逆向"));
        assert!(prompt.contains("[EASYVIBE-RESULT]"));
        assert!(prompt.contains("\"inject\""));
        // 注入点 #1 回归：无 custom 时 framework 后紧跟任务分隔（global 块为空串）
        let sep = prompt.find("\n\n---\n\n## 本次任务").expect("任务分隔结构必须保留");
        assert!(
            prompt[..sep].ends_with(&format!("{}/rule_development.md", dir.display())),
            "无 custom 时框架与任务书之间不得有多余内容（framework 尾部是换姓后的规则路径）"
        );
    }

    #[test]
    fn task_prompt_injects_global_custom_block() {
        // 注入点 #1：global 块拼在 framework 后、任务书前，带引导句
        let harness = harness_stub("框架正文");
        {
            let mut h = harness.blocking_write();
            h.custom.global = Some("团队规则A：所有查询参数化".into());
            h.custom.development = Some("开发规则B".into());
        }
        let prompt = {
            let h = harness.blocking_read();
            assemble_task_prompt(&h, &sample_task("pending"))
        };
        let fw_end = prompt.find("框架正文").unwrap() + "框架正文".len();
        let sep = prompt.find("\n\n---\n\n## 本次任务").unwrap();
        let between = &prompt[fw_end..sep];
        assert!(between.contains(CUSTOM_BLOCK_HEADER), "global 块必须带引导句");
        assert!(between.contains("团队规则A：所有查询参数化"), "global 内容必须在任务书之前");
        assert!(!between.contains("开发规则B"), "实施 prompt 只注 global，不注 development 槽");
        drop(harness);
    }

    #[test]
    fn phase_prompts_inject_global_and_development_blocks() {
        // 注入点 #2/#9：阶段 prompt 与初审 prompt 注 global + development 两槽
        let custom = HarnessCustom {
            global: Some("团队规则A".into()),
            development: Some("开发规则B".into()),
        };
        let t = sample_task("running");
        let p1 = assemble_phase1_prompt(&t, "liyuhang", &custom);
        assert!(p1.contains(CUSTOM_BLOCK_HEADER));
        assert!(p1.contains("团队规则A") && p1.contains("开发规则B"), "阶段1 prompt 应含两槽");
        assert!(p1.contains("[EASYVIBE-RESULT]"), "输出契约不得被破坏");
        let p2 = assemble_phase2_prompt(&t, "liyuhang", &custom);
        assert!(p2.contains("团队规则A") && p2.contains("开发规则B"));
        let pr = assemble_phase_review_prompt(&t, 1, "liyuhang", &custom);
        assert!(pr.contains(CUSTOM_BLOCK_HEADER) && pr.contains("团队规则A") && pr.contains("开发规则B"), "初审 prompt 应含两槽");
        assert!(pr.contains("[EASYVIBE-REVIEW]"));
        // 无 custom：产物与现状一致（块为空串）
        let empty = HarnessCustom::default();
        let bare = assemble_phase1_prompt(&t, "liyuhang", &empty);
        assert!(!bare.contains(CUSTOM_BLOCK_HEADER), "空槽不得注入引导句");
    }

    #[test]
    fn review_prompt_injects_custom_blocks() {
        // 注入点 #3：审查 prompt 注 global + development（v1 唯一规则槽）
        let custom = HarnessCustom { global: Some("团队规则A".into()), development: Some("开发规则B".into()) };
        let t = sample_task("running");
        let p = assemble_review_prompt(&t, "liyuhang", &custom);
        assert!(p.contains("团队规则A") && p.contains("开发规则B"));
        assert!(p.contains("[EASYVIBE-REVIEW]"));
    }

    #[test]
    fn custom_block_renames_claude_paths() {
        // custom 内容的路径换姓发生在装载层（load_custom 过 adapt）——此处验证空槽/空白串不产生块
        assert_eq!(custom_block(&None), "");
        assert_eq!(custom_block(&Some("   ".into())), "");
        let b = custom_block(&Some("规则正文".into()));
        assert!(b.starts_with("\n\n【用户补充规则"));
        assert!(b.contains("规则正文"));
    }

    #[test]
    fn phase_review_prompt_targets_right_dir_and_readonly() {
        // 初审 prompt：阶段 1/2 指向各自产物目录；只审不改纪律与结论协议行必备
        let t = sample_task("running");
        let empty = HarnessCustom::default();
        let p1 = assemble_phase_review_prompt(&t, 1, "liyuhang", &empty);
        assert!(p1.contains("1_requirements_matrix/"));
        assert!(p1.contains("只审不改"));
        assert!(p1.contains("[EASYVIBE-REVIEW]"));
        assert!(p1.contains("把双向依赖改为单向"), "任务书必须注入作对照基准");
        let p2 = assemble_phase_review_prompt(&t, 2, "liyuhang", &empty);
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
