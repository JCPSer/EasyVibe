//! task_exec 测试（仅测试编译）。

use super::*;
use super::test_util::*;

    #[test]
    fn harness_slots_assembly_and_neutralize_from_manifest() {
        // §12c 插槽内核回归：①透明装配的中和模式来自 manifest 声明（退役硬编码 grep）
        // ②user_entry 插槽正文装载（grill-me 以 skill 形态挂插槽，§9 #4）
        let dir = std::env::temp_dir().join("ev-harness-grill-test2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("skills/grill-me")).unwrap();
        std::fs::write(
            dir.join("inject-prompt.md"),
            "用户的表达一定是片面的，正式开始动手前，必须使用grill-me技能拷问用户。\ncat ~/.claude/hooks/rule_development.md",
        )
        .unwrap();
        std::fs::write(dir.join("skills/grill-me/SKILL.md"), "# grill-me\n访谈方法论正文").unwrap();

        // 场景 1：manifest 声明中和模式 → 透明框架不含拷问指令
        write_manifest(&dir, &["grill-me", "拷问"], &["skills/grill-me/SKILL.md"]);
        let h = load_harness_from(&dir).unwrap();
        assert!(!h.framework_transparent.to_lowercase().contains("grill-me"), "透明 agent 不得收到 grill-me 指令");
        assert!(!h.framework_transparent.contains("拷问"), "拷问指令必须被替换");
        assert!(h.framework_transparent.contains("透明执行模式"));
        assert_eq!(h.user_entry_skills.len(), 1, "user_entry 插槽正文应装载");
        assert!(h.user_entry_skills[0].contains("grill-me"), "插槽内容应是 skill 本体");

        // 场景 2：manifest 不声明中和 → 不过滤（证明驱动者是 manifest 而非硬编码）
        write_manifest(&dir, &[], &[]);
        let h2 = load_harness_from(&dir).unwrap();
        assert!(h2.framework_transparent.contains("grill-me"), "中和由 manifest 声明驱动");

        // 场景 3：出厂底账部署——v2 拆分后 deploy_sealed 负责补齐，load_harness_from 纯装载
        let dir3 = std::env::temp_dir().join("ev-harness-builtin-test");
        let _ = std::fs::remove_dir_all(&dir3);
        deploy_sealed(&dir3).unwrap();
        let h3 = load_harness_from(&dir3).unwrap();
        assert_eq!(h3.manifest.id, "builtin-default", "底账 manifest 应就位");
        assert!(!h3.framework_transparent.contains("拷问"), "出厂底账透明装配仍须中和");
        assert_eq!(h3.user_entry_skills.len(), 1, "出厂底账 user_entry=grill-me");
        assert!(dir3.join("rule_development.md").exists(), "规则正文应补齐");
    }

    #[test]
    fn deploy_sealed_restores_factory_content() {
        // 密封语义（方案 v2 §3.1）：磁盘被手工改（含写入未换姓的底账原文）→ deploy_sealed 一律覆写回出厂内容
        let dir = std::env::temp_dir().join("ev-harness-sealed-test");
        let _ = std::fs::remove_dir_all(&dir);
        deploy_sealed(&dir).unwrap();
        // 手工改动：换姓形态被破坏 + 塞入未换姓原文（两种"不等于出厂"形态都要被抓回）
        std::fs::write(dir.join("rule_development.md"), "被用户改坏的规则正文").unwrap();
        let raw = BUILTIN_HARNESS.iter().find(|(r, _)| *r == "inject-prompt.md").unwrap().1;
        std::fs::write(dir.join("inject-prompt.md"), raw).unwrap();
        deploy_sealed(&dir).unwrap();
        let rd = std::fs::read_to_string(dir.join("rule_development.md")).unwrap();
        assert!(!rd.contains("被用户改坏"), "手工改动必须被覆写");
        assert!(rd.contains(".easyvibe/"), "重铺结果必须是换姓形态");
        let ip = std::fs::read_to_string(dir.join("inject-prompt.md")).unwrap();
        assert_ne!(ip, raw, "未换姓的底账原文必须被识别为偏离并覆写");
        // 幂等：稳态再跑一遍，内容不变
        let before = std::fs::read_to_string(dir.join("rule_development.md")).unwrap();
        deploy_sealed(&dir).unwrap();
        assert_eq!(before, std::fs::read_to_string(dir.join("rule_development.md")).unwrap(), "稳态幂等");
    }

    #[test]
    fn migration_seeds_custom_and_is_crash_safe() {
        // 一次性迁移（§3.3）三态：有改动（归档+重铺+播种+哨兵）、槽已存在不覆盖（崩溃重入）、二次调用幂等
        let base = std::env::temp_dir().join("ev-harness-mig-suite");
        let _ = std::fs::remove_dir_all(&base);
        let factory = base.join("harness");
        let custom = base.join("harness-custom");
        std::fs::create_dir_all(&factory).unwrap();
        // 预置"老用户"状态：出厂文件被 10-04 的 UI 编辑过（换姓形态 + 用户行）
        deploy_sealed(&factory).unwrap();
        let ip = std::fs::read_to_string(factory.join("inject-prompt.md")).unwrap();
        std::fs::write(factory.join("inject-prompt.md"), format!("{ip}\n\n用户自定义补充")).unwrap();
        let rd = std::fs::read_to_string(factory.join("rule_development.md")).unwrap();
        std::fs::write(factory.join("rule_development.md"), format!("{rd}\n\n团队规则X")).unwrap();

        migrate_builtin_to_custom(&factory, &custom);

        // 出厂层被重铺为密封内容（用户行消失）
        let ip_after = std::fs::read_to_string(factory.join("inject-prompt.md")).unwrap();
        assert!(!ip_after.contains("用户自定义补充"), "出厂层必须恢复密封");
        // 旧层归档为 harness.backup-*
        let backup = base.join(
            std::fs::read_dir(&base).unwrap().flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .find(|n| n.starts_with("harness.backup-"))
                .expect("应产生备份目录"),
        );
        // 播种：global.md 拿到旧 inject-prompt 全文（含用户行），development 槽拿到旧规则全文
        let g = std::fs::read_to_string(custom.join("global.md")).unwrap();
        assert!(g.contains("用户自定义补充"), "旧 inject-prompt 修改应播进 global.md");
        let d = std::fs::read_to_string(custom.join("rule_development.md")).unwrap();
        assert!(d.contains("团队规则X"), "旧 rule_development 修改应播进同名槽");
        // 哨兵就位 → 二次调用幂等
        assert!(custom.join(".migrated-v2").exists(), "哨兵应写入");
        let g_before = g.clone();
        migrate_builtin_to_custom(&factory, &custom);
        assert_eq!(g_before, std::fs::read_to_string(custom.join("global.md")).unwrap(), "哨兵短路：不得重复播种");

        // 崩溃重入语义：删掉哨兵 + 预置半完成手稿 → 重跑不得覆盖既有槽
        std::fs::remove_file(custom.join(".migrated-v2")).unwrap();
        std::fs::write(custom.join("global.md"), "半完成的手稿，不得被覆盖").unwrap();
        migrate_builtin_to_custom(&factory, &custom);
        assert_eq!(
            "半完成的手稿，不得被覆盖",
            std::fs::read_to_string(custom.join("global.md")).unwrap(),
            "槽文件已存在时播种必须跳过（崩溃重入安全）"
        );
        let _ = backup;
    }

    #[test]
    fn migration_noop_when_factory_clean() {
        // 无改动的老用户：不播种、只写哨兵（启动路径不得误触发归档）
        let base = std::env::temp_dir().join("ev-harness-mig-clean");
        let _ = std::fs::remove_dir_all(&base);
        let factory = base.join("harness");
        let custom = base.join("harness-custom");
        deploy_sealed(&factory).unwrap();
        migrate_builtin_to_custom(&factory, &custom);
        assert!(custom.join(".migrated-v2").exists());
        assert!(!custom.join("global.md").exists(), "无修改不得播种");
        assert!(!custom.join("rule_development.md").exists(), "无修改不得播种");
        assert!(factory.exists(), "无修改不得归档出厂层");
    }

    #[test]
    fn custom_layer_loading_and_state_toggle() {
        // 自定义层装载（§3.4）：缺失=无补充；停用=不注入；路径换姓；64KB 超限拒载
        let base = std::env::temp_dir().join("ev-harness-custom-test");
        let _ = std::fs::remove_dir_all(&base);
        let factory = base.join("harness");
        let custom = base.join("harness-custom");
        deploy_sealed(&factory).unwrap();
        write_manifest(&factory, &[], &[]);

        // 无 custom 目录：全部 None
        let h = load_harness_from(&factory).unwrap();
        assert!(h.custom.global.is_none() && h.custom.development.is_none(), "无 custom 目录 = 无补充");

        // 写入两槽 + 停用 development
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(custom.join("global.md"), "团队规则：.claude/development_docs 里的产物要复查").unwrap();
        std::fs::write(custom.join("rule_development.md"), "开发规则补充").unwrap();
        std::fs::write(custom.join("state.json"), r#"{"development": false}"#).unwrap();
        let h2 = load_harness_from(&factory).unwrap();
        let g = h2.custom.global.expect("global 应装载");
        assert!(g.contains(".easyvibe/development_docs"), "custom 内容须路径换姓");
        assert!(!g.contains(".claude/development_docs"), "旧路径不得残留");
        assert!(h2.custom.development.is_none(), "停用槽不得注入");

        // 重新启用
        std::fs::write(custom.join("state.json"), r#"{"development": true}"#).unwrap();
        let h3 = load_harness_from(&factory).unwrap();
        assert_eq!(h3.custom.development.as_deref(), Some("开发规则补充"), "启用后应装载");

        // 64KB 超限拒载（不 panic，槽 = None）
        std::fs::write(custom.join("rule_development.md"), "x".repeat(70 * 1024)).unwrap();
        let h4 = load_harness_from(&factory).unwrap();
        assert!(h4.custom.development.is_none(), "超限槽应拒载");
        assert!(h4.custom.global.is_some(), "超限不影响其他槽");
    }
