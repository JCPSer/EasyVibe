//! task_exec 测试（仅测试编译）。

use super::*;
use super::test_util::*;

    #[test]
    fn contract_matcher_matches_frontend_semantics() {
        // 影响面合约：与前端 moduleOfFile 同口径——** 前缀匹配 + 路径段包含
        let pats = vec!["src/core/**".to_string(), "src/ui".to_string()];
        assert!(path_within_contract("src/core/a/b.ts", &pats));
        assert!(path_within_contract("lib/src/core/x.ts", &pats), "路径段包含命中");
        assert!(path_within_contract("src/ui/Button.tsx", &pats));
        assert!(!path_within_contract("src/other/c.ts", &pats));
        assert!(!path_within_contract("README.md", &pats));
        assert!(!path_within_contract("src/coreography/data.ts", &pats), "R2 实锤：前缀必须有段边界");
        assert!(!path_within_contract("src/core_plus/x.ts", &pats), "下划线前缀同样不得误配");
        // 精确文件型 glob（自托管实弹回归：path == base 必须命中，否则边界文件永被误报越界）
        let file_pats = vec!["src/main.rs".to_string()];
        assert!(path_within_contract("src/main.rs", &file_pats));
        assert!(!path_within_contract("src/main.rs.bak", &file_pats));
        // 空合约 = 不约束（未声明模块的任务不校验）
        assert!(!path_within_contract("anything", &[]));
        // context 提取
        let ctx = r#"{"contract":{"patterns":["a/**","b"]}}"#;
        assert_eq!(contract_patterns_from_context(ctx), vec!["a/**".to_string(), "b".to_string()]);
        assert!(contract_patterns_from_context("{}").is_empty());
    }

    #[tokio::test]
    async fn contract_violations_detected_at_collection() {
        // 影响面合约实弹：声明 allowed/**，agent 改了界内文件 + 越界新文件（未跟踪）——
        // 未跟踪必须被 porcelain 捕获（diff --stat 会漏，新建文件恰是最常见越界形态）
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let sessions = SessionManager::new(tx);
        let db = easyvibe_db::Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(easyvibe_db::SqliteTaskRepository::new(db.pool().clone()));
        let repo = std::env::temp_dir().join("ev-contract-test");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("allowed")).unwrap();
        std::fs::create_dir_all(repo.join("stray")).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("allowed/base.txt"), "1\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);

        let mut task = sample_task("running");
        task.id = "task-contract".into();
        task.context = r#"{"contract":{"patterns":["allowed/**"]}}"#.into();
        db_create(task_repo.as_ref(), &task).await;

        // 界内修改 + 越界未跟踪新文件
        std::fs::write(repo.join("allowed/base.txt"), "1\n2\n").unwrap();
        std::fs::write(repo.join("stray/out.txt"), "oops\n").unwrap();
        let json = collect_task_result(&sessions, "no-such-session", &repo, "task-contract", task_repo.as_ref(), &[]).await.expect("有 diff 即应采集");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let viol = v["contractViolations"].as_array().expect("必须有越界列表");
        assert_eq!(viol.len(), 1, "allowed/base.txt 界内、stray/out.txt 越界: {viol:?}");
        assert_eq!(viol[0].as_str().unwrap(), "stray/out.txt");
        assert!(
            v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("影响面合约")),
            "warnings 必须亮红线: {v}"
        );
        // 界内任务零误报
        std::fs::remove_file(repo.join("stray/out.txt")).unwrap();
        let mut clean = sample_task("running");
        clean.id = "task-contract-clean".into();
        clean.context = r#"{"contract":{"patterns":["allowed/**"]}}"#.into();
        db_create(task_repo.as_ref(), &clean).await;
        let json = collect_task_result(&sessions, "no-such-session", &repo, "task-contract-clean", task_repo.as_ref(), &[]).await.expect("有 diff 即应采集");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["contractViolations"].as_array().unwrap().is_empty(), "界内改动零误报: {v}");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[tokio::test]
    async fn contract_baseline_excludes_preexisting_dirt() {
        // R2 裂缝#1 回归：任务启动前就存在的脏文件（baseline 快照）不得算越界——
        // 否则红线出冤案：陈年脏仓库里每个任务都被误报
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let sessions = SessionManager::new(tx);
        let db = easyvibe_db::Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(easyvibe_db::SqliteTaskRepository::new(db.pool().clone()));
        let repo = std::env::temp_dir().join("ev-contract-baseline-test");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("allowed")).unwrap();
        std::fs::create_dir_all(repo.join("legacy")).unwrap();
        std::fs::create_dir_all(repo.join("stray")).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("allowed/base.txt"), "1\n").unwrap();
        std::fs::write(repo.join("legacy/old.txt"), "old\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);

        // 任务启动前 legacy/old.txt 已被改脏（baseline 快照会收录它）
        std::fs::write(repo.join("legacy/old.txt"), "old\ndirty\n").unwrap();
        // 任务执行：界内改动 + 全新越界文件
        std::fs::write(repo.join("allowed/base.txt"), "1\n2\n").unwrap();
        std::fs::write(repo.join("stray/new.txt"), "agent\n").unwrap();

        let mut task = sample_task("running");
        task.id = "task-baseline".into();
        task.context = r#"{"contract":{"patterns":["allowed/**"]}}"#.into();
        db_create(task_repo.as_ref(), &task).await;
        // 基线 = 启动时脏文件（legacy/old.txt）——spawn 路径由 dirty_files 提供，测试直接给等价快照
        let baseline = vec!["legacy/old.txt".to_string()];
        let json = collect_task_result(&sessions, "no-such-session", &repo, "task-baseline", task_repo.as_ref(), &baseline).await.expect("有 diff 即应采集");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let viol = v["contractViolations"].as_array().unwrap();
        assert_eq!(viol.len(), 1, "启动前的脏文件 legacy/old.txt 不得算越界: {viol:?}");
        assert_eq!(viol[0].as_str().unwrap(), "stray/new.txt");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[tokio::test]
    async fn collect_warns_when_result_line_missing() {
        // 实弹#3 防线：会话成功但无 RESULT 行 + 有 git 改动 → 采集带警告（审批人警惕空执行/归因错位）
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let sessions = SessionManager::new(tx); // 无此会话 → 无输出 → 解析不到 RESULT
        let db = easyvibe_db::Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(easyvibe_db::SqliteTaskRepository::new(db.pool().clone()));
        let repo = std::env::temp_dir().join("ev-git-repo-warn-test");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("x.txt"), "1\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        std::fs::write(repo.join("x.txt"), "1\n2\n").unwrap();
        let json = collect_task_result(&sessions, "no-such-session", &repo, "task-warn", task_repo.as_ref(), &[]).await.expect("有 diff 即应采集");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["result"].is_null());
        assert!(
            v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("EASYVIBE-RESULT")),
            "缺 RESULT 行必须亮警告: {v}"
        );
    }

    #[test]
    fn sentry_violators_are_incremental_and_idempotent() {
        // L2 哨兵：只报新越界文件，重复巡检同一文件不重复预警（幂等）
        let mut reported = std::collections::HashSet::new();
        let c1 = vec!["stray/a.ts".to_string(), "stray/b.ts".to_string()];
        assert_eq!(new_violators(&c1, &mut reported), c1, "首报全量");
        assert!(new_violators(&c1, &mut reported).is_empty(), "重复巡检不重复报");
        let c2 = vec!["stray/a.ts".to_string(), "stray/c.ts".to_string()];
        assert_eq!(new_violators(&c2, &mut reported), vec!["stray/c".to_string() + ".ts"], "只报新增");
        assert_eq!(reported.len(), 3);
    }

    #[test]
    fn parse_result_line_tolerant() {
        let good = "前置输出若干行\n[EASYVIBE-RESULT] {\"summary\":\"修复完成\",\"changed_modules\":[\"m1\"]}";
        let v = parse_result_line(good).unwrap();
        assert_eq!(v["summary"], "修复完成");
        assert_eq!(v["changed_modules"][0], "m1");
        // 取最后一行（agent 可能多次提及）
        let multi = "[EASYVIBE-RESULT] {\"summary\":\"旧\"}\nnoise\n[EASYVIBE-RESULT] {\"summary\":\"新\"}";
        assert_eq!(parse_result_line(multi).unwrap()["summary"], "新");
        assert!(parse_result_line("没有任何归档行").is_none());
        assert!(parse_result_line("[EASYVIBE-RESULT] 不是json").is_none());
    }
