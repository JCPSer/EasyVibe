//! task_exec 测试（仅测试编译）。

use super::*;

    #[tokio::test]
    async fn dirty_files_baseline_includes_gitignored_untracked() {
        // 2026-10-03 实弹回归（hover-client 644 越界冤案）：上轮被打回的 agent 改 .gitignore
        // 把 docs/ 变忽略 → 基线快照这批文件"被消失" → 本轮恢复 .gitignore 后它们首次进
        // git 视野，全部被误判"任务新增越界"。基线必须收录"被忽略但存在"的文件。
        let dir = std::env::temp_dir().join(format!("ev-dirty-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join(".gitignore"), "docs/\n").unwrap();
        std::fs::write(dir.join("docs/asset.png"), "x").unwrap();
        std::fs::write(dir.join("tracked.txt"), "t").unwrap();
        for args in [
            ["init", "-q"].as_slice(),
            ["add", "."].as_slice(),
            ["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"].as_slice(),
        ] {
            std::process::Command::new("git").args(args).current_dir(&dir).output().unwrap();
        }
        // 提交后制造"被忽略但未跟踪"与"普通未跟踪"
        std::fs::write(dir.join("docs/asset.png"), "y").unwrap();
        std::fs::write(dir.join("new.txt"), "n").unwrap();
        let d = dirty_files(&dir).await;
        assert!(d.iter().any(|p| p.contains("docs/asset.png")), "被忽略但存在的文件必须进基线: {:?}", d);
        assert!(d.iter().any(|p| p.contains("new.txt")), "普通未跟踪文件必须在基线: {:?}", d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn git_change_summary_tracks_and_untracked() {
        // 非 git 目录 → None
        let plain = std::env::temp_dir().join("ev-not-a-repo");
        let _ = std::fs::remove_dir_all(&plain);
        std::fs::create_dir_all(&plain).unwrap();
        assert!(git_change_summary(&plain, None).await.is_none(), "非 git 仓库无摘要");

        // 真 git 仓库：提交后修改已跟踪文件 + 新增未跟踪文件
        let repo = std::env::temp_dir().join("ev-git-repo-test");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(&repo).output().expect("git 执行失败")
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(repo.join("b_new.txt"), "new\n").unwrap();
        let summary = git_change_summary(&repo, None).await.expect("git 仓库应有摘要");
        assert!(summary.contains("a.txt"), "已跟踪改动应入摘要: {summary}");
        assert!(summary.contains("b_new.txt"), "未跟踪新文件应入摘要: {summary}");
        // M4-3：完整 diff 含增行；非 git 目录返回 None
        let full = git_full_diff(&repo, None).await.expect("应有完整 diff");
        assert!(full.contains("+two"), "diff 应含新增行: {full}");
        assert!(full.contains("diff --git"), "标准 diff 格式: {full}");
        assert!(git_full_diff(&plain, None).await.is_none(), "非 git 仓库无完整 diff");
    }
