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
    fn deploy_never_touches_existing_factory_files() {
        // 17:58 裁定：任何方式不动原 harness——deploy 只在缺失时写参考副本；
        // 已存在的文件（含被用户手改的）一律不碰；装配事实源是内嵌底账，磁盘被改不影响装配。
        let dir = std::env::temp_dir().join("ev-harness-deploy-test");
        let _ = std::fs::remove_dir_all(&dir);
        deploy_sealed(&dir).unwrap();
        // 手改出厂文件（模拟用户在磁盘上编辑）
        std::fs::write(dir.join("rule_development.md"), "用户手改的出厂规则").unwrap();
        deploy_sealed(&dir).unwrap();
        let disk = std::fs::read_to_string(dir.join("rule_development.md")).unwrap();
        assert_eq!(disk, "用户手改的出厂规则", "已存在的出厂文件必须原样保留（运行时零写入）");
        // 内嵌装配不受磁盘污染：产物仍等于出厂换姓版
        let custom_dir = dir.with_file_name("harness-custom-deploy-test");
        let _ = std::fs::remove_dir_all(&custom_dir);
        let h = assemble_from_builtin(&dir, &custom_dir).unwrap();
        assert!(h.rule_development.contains(".easyvibe/"), "内嵌装配 = 出厂换姓版");
        assert!(!h.rule_development.contains("用户手改"), "磁盘手改不得进入装配产物");
        // 缺失文件仍会被补齐（首次部署语义）
        std::fs::remove_file(dir.join("manifest.json")).unwrap();
        deploy_sealed(&dir).unwrap();
        assert!(dir.join("manifest.json").exists(), "缺失文件应补齐");
    }

    #[test]
    fn migration_seeds_custom_without_touching_factory() {
        // 一次性迁移（只读播种，17:58 修订）：出厂层含用户修改 → 播进 custom；
        // 出厂目录原样不动（不 rename、不重铺）；槽已存在不覆盖（崩溃重入安全）；二次调用幂等
        let base = std::env::temp_dir().join("ev-harness-mig-suite");
        let _ = std::fs::remove_dir_all(&base);
        let factory = base.join("harness");
        let custom = base.join("harness-custom");
        std::fs::create_dir_all(&factory).unwrap();
        deploy_sealed(&factory).unwrap();
        let ip = std::fs::read_to_string(factory.join("inject-prompt.md")).unwrap();
        std::fs::write(factory.join("inject-prompt.md"), format!("{ip}\n\n用户自定义补充")).unwrap();
        let rd = std::fs::read_to_string(factory.join("rule_development.md")).unwrap();
        std::fs::write(factory.join("rule_development.md"), format!("{rd}\n\n团队规则X")).unwrap();

        migrate_builtin_to_custom(&factory, &custom);

        // 出厂层原样不动（用户修改保留在原地）
        let ip_after = std::fs::read_to_string(factory.join("inject-prompt.md")).unwrap();
        assert!(ip_after.contains("用户自定义补充"), "迁移不得动出厂层");
        assert!(custom.join(".migrated-v2").exists(), "哨兵应写入");
        // 播种：global.md 拿到旧 inject-prompt 全文（含用户行），development 槽拿到旧规则全文
        let g = std::fs::read_to_string(custom.join("global.md")).unwrap();
        assert!(g.contains("用户自定义补充"), "旧 inject-prompt 修改应播进 global.md");
        let d = std::fs::read_to_string(custom.join("rule_development.md")).unwrap();
        assert!(d.contains("团队规则X"), "旧 rule_development 修改应播进同名槽");
        // 二次调用幂等
        let g_before = g.clone();
        migrate_builtin_to_custom(&factory, &custom);
        assert_eq!(g_before, std::fs::read_to_string(custom.join("global.md")).unwrap(), "哨兵短路：不得重复播种");

        // 崩溃重入语义：删哨兵 + 预置半完成手稿 → 重跑不得覆盖既有槽、且仍不动出厂层
        std::fs::remove_file(custom.join(".migrated-v2")).unwrap();
        std::fs::write(custom.join("global.md"), "半完成的手稿，不得被覆盖").unwrap();
        migrate_builtin_to_custom(&factory, &custom);
        assert_eq!(
            "半完成的手稿，不得被覆盖",
            std::fs::read_to_string(custom.join("global.md")).unwrap(),
            "槽文件已存在时播种必须跳过（崩溃重入安全）"
        );
        assert!(std::fs::read_to_string(factory.join("inject-prompt.md")).unwrap().contains("用户自定义补充"), "重入仍不得动出厂层");
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
        assert!(factory.exists(), "出厂层保持原样");
    }

    #[test]
    fn custom_neutral_copy_blocks_interrogation_leaks() {
        // 2026-10-05 实弹防线：迁移播种的旧框架文本含 grill-me 拷问指令，巡检 agent 被带跑。
        // custom 层必须为透明注入点提供"中和副本"——与出厂框架同一 neutralize 防线；
        // 原始副本保留给入口对话（user_entry 场景拷问/澄清是合法行为）
        let base = std::env::temp_dir().join("ev-harness-neutral-test");
        let _ = std::fs::remove_dir_all(&base);
        let factory = base.join("harness");
        let custom = base.join("harness-custom");
        deploy_sealed(&factory).unwrap();
        write_manifest(&factory, &["grill-me", "拷问"], &[]);
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(custom.join("global.md"), "用户的表达一定是片面的，必须使用grill-me技能拷问用户。\n正常补充条款").unwrap();

        let h = load_harness_from(&factory).unwrap();
        let raw = h.custom.global.expect("raw 副本应有内容");
        assert!(raw.contains("拷问用户"), "原始副本保留原文（供入口对话使用）");
        let neutral = h.custom_neutral.global.expect("中和副本应有内容");
        assert!(!neutral.contains("拷问用户") && !neutral.to_lowercase().contains("grill-me"), "中和副本不得残留拷问指令");
        assert!(neutral.contains("透明执行模式"), "命中行必须替换为透明执行指令");
        assert!(neutral.contains("正常补充条款"), "未命中行保持原样");
    }

    #[test]
    fn custom_layer_loading_state_toggle_and_type_fallback() {
        // 自定义层装载（§3.4）：缺失=无补充；停用=不注入；路径换姓；64KB 超限拒载；
        // 五个 harness 类型各归其槽；development 仅作兜底（用户裁定 18:10）
        let base = std::env::temp_dir().join("ev-harness-custom-test");
        let _ = std::fs::remove_dir_all(&base);
        let factory = base.join("harness");
        let custom = base.join("harness-custom");
        deploy_sealed(&factory).unwrap();
        write_manifest(&factory, &[], &[]);

        // 无 custom 目录：全部 None
        let h = load_harness_from(&factory).unwrap();
        assert!(h.custom.global.is_none() && h.custom.analysis.is_none() && h.custom.review.is_none(), "无 custom 目录 = 无补充");

        // 写入五个类型槽 + 停用 review
        std::fs::create_dir_all(&custom).unwrap();
        std::fs::write(custom.join("global.md"), "团队规则：.claude/development_docs 里的产物要复查").unwrap();
        std::fs::write(custom.join("rule_analysis.md"), "需求分析补充").unwrap();
        std::fs::write(custom.join("rule_design.md"), "方案设计补充").unwrap();
        std::fs::write(custom.join("rule_implement.md"), "代码开发补充").unwrap();
        std::fs::write(custom.join("rule_review.md"), "代码审查补充").unwrap();
        std::fs::write(custom.join("state.json"), r#"{"review": false}"#).unwrap();
        let h2 = load_harness_from(&factory).unwrap();
        let g = h2.custom.global.expect("global 应装载");
        assert!(g.contains(".easyvibe/development_docs"), "custom 内容须路径换姓");
        assert!(!g.contains(".claude/development_docs"), "旧路径不得残留");
        assert_eq!(h2.custom.analysis.as_deref(), Some("需求分析补充"));
        assert_eq!(h2.custom.design.as_deref(), Some("方案设计补充"));
        assert_eq!(h2.custom.implement.as_deref(), Some("代码开发补充"));
        assert!(h2.custom.review.is_none(), "停用槽不得注入");
        assert!(h2.custom.development.is_none(), "未创建 development 槽");

        // 重新启用 review + 写 development 兜底槽
        std::fs::write(custom.join("state.json"), r#"{"review": true}"#).unwrap();
        std::fs::write(custom.join("rule_development.md"), "旧版四阶段共用补充").unwrap();
        let h3 = load_harness_from(&factory).unwrap();
        assert_eq!(h3.custom.review.as_deref(), Some("代码审查补充"), "启用后应装载");
        assert_eq!(h3.custom.development.as_deref(), Some("旧版四阶段共用补充"));

        // 64KB 超限拒载（不 panic，槽 = None）
        std::fs::write(custom.join("rule_analysis.md"), "x".repeat(70 * 1024)).unwrap();
        let h4 = load_harness_from(&factory).unwrap();
        assert!(h4.custom.analysis.is_none(), "超限槽应拒载");
        assert!(h4.custom.global.is_some(), "超限不影响其他槽");
    }
