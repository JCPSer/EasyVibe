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

        // 场景 3：出厂底账部署——清空目录后 load_harness 从内嵌底账补齐全部文件
        let dir3 = std::env::temp_dir().join("ev-harness-builtin-test");
        let _ = std::fs::remove_dir_all(&dir3);
        let h3 = load_harness_from(&dir3).unwrap();
        assert_eq!(h3.manifest.id, "builtin-default", "底账 manifest 应就位");
        assert!(!h3.framework_transparent.contains("拷问"), "出厂底账透明装配仍须中和");
        assert_eq!(h3.user_entry_skills.len(), 1, "出厂底账 user_entry=grill-me");
        assert!(dir3.join("rule_development.md").exists(), "规则正文应补齐");
    }

    #[test]
    fn deploy_migrates_unmodified_rules_but_keeps_user_edits() {
        let dir = std::env::temp_dir().join("ev-harness-migrate-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 预置"旧版部署"：manifest 1.1.0 + 三份规则正文 = 出厂原稿（含 .claude）
        let builtin_ver = |s: &str| {
            BUILTIN_HARNESS.iter().find(|(r, _)| *r == "manifest.json").map(|(_, c)| {
                serde_json::from_str::<serde_json::Value>(c).unwrap()["version"].as_str().unwrap().to_string()
            }).unwrap_or_else(|| s.into())
        };
        let builtin_version = builtin_ver("");
        // 模拟磁盘上的旧 manifest：内置版本降一段，其余字段照抄
        let old_manifest = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../reference/manifest.json")).unwrap()
            .replace(&format!("\"version\": \"{builtin_version}\""), "\"version\": \"1.0.0\"");
        std::fs::write(dir.join("manifest.json"), &old_manifest).unwrap();
        let rd = BUILTIN_HARNESS.iter().find(|(r, _)| *r == "rule_development.md").unwrap().1;
        let rb = BUILTIN_HARNESS.iter().find(|(r, _)| *r == "rule_bugfix.md").unwrap().1;
        std::fs::write(dir.join("rule_development.md"), rd).unwrap();
        std::fs::write(dir.join("rule_bugfix.md"), rb).unwrap();
        // 用户编辑过的文件：加一行批注（内容不等于出厂原稿）
        std::fs::write(dir.join("inject-prompt.md"), format!("{}\n\n用户自定义补充", BUILTIN_HARNESS.iter().find(|(r, _)| *r == "inject-prompt.md").unwrap().1)).unwrap();

        deploy_builtin(&dir).unwrap();

        // 未改动的规则正文：重写为换姓版
        let new_rd = std::fs::read_to_string(dir.join("rule_development.md")).unwrap();
        assert!(!new_rd.contains(".claude/"), "未改动文件必须完成换姓");
        assert!(new_rd.contains(".easyvibe/development_docs"), "换姓目标路径");
        // 用户编辑过的文件：原样保留
        let kept = std::fs::read_to_string(dir.join("inject-prompt.md")).unwrap();
        assert!(kept.contains("用户自定义补充"), "用户编辑不得被迁移覆盖");
        // manifest：版本被抬到内置版，其余字段保留
        let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m["version"].as_str().unwrap(), builtin_version, "manifest 版本抬升");
        assert!(m["routeRules"].is_array(), "manifest 其余字段保留");
    }

    #[test]
    fn deploy_no_migrate_when_versions_equal() {
        let dir = std::env::temp_dir().join("ev-harness-nomigrate-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 磁盘版本 = 内置版本 → 即使文件还是旧内容也不重写（防无限迁移循环）
        deploy_builtin(&dir).unwrap(); // 首次：缺失补齐（已是换姓版）
        let rd_before = std::fs::read_to_string(dir.join("rule_development.md")).unwrap();
        deploy_builtin(&dir).unwrap(); // 第二次：版本相等，不动
        let rd_after = std::fs::read_to_string(dir.join("rule_development.md")).unwrap();
        assert_eq!(rd_before, rd_after, "版本相等时不得重写");
    }
