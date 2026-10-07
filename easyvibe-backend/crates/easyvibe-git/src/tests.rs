use super::*;


#[test]
fn porcelain_basic_kinds() {
    let text = " M src/a.java\nA  src/b.java\n?? new/file.ts\n D gone.sql\nR  old.yml -> new.yml\n M \"quoted path.java\"\n";
    let fs = parse_porcelain(text);
    assert_eq!(fs.len(), 6);
    assert_eq!(fs[0], GitFile { status: 'M', path: "src/a.java".into(), orig: None, adds: None, dels: None });
    assert_eq!(fs[1].status, 'A');
    assert_eq!(fs[2], GitFile { status: '?', path: "new/file.ts".into(), orig: None, adds: None, dels: None });
    assert_eq!(fs[3].status, 'D');
    assert_eq!(fs[4].orig.as_deref(), Some("old.yml"));
    assert_eq!(fs[4].path, "new.yml");
    assert_eq!(fs[5].path, "\"quoted path.java\"", "带引号路径原样保留（前端展示）");
}

#[test]
fn numstat_merge_and_rename() {
    let text = "12\t3\tsrc/a.java\n-\t-\tbin/logo.png\n1\t1\tsrc/{old => new}/c.java\n";
    let rows = parse_numstat(text);
    assert_eq!(rows[0], ("src/a.java".into(), Some(12), Some(3)));
    assert_eq!(rows[1], ("bin/logo.png".into(), None, None), "二进制无统计");
    assert_eq!(rows[2].0, "src/new/c.java", "重命名取目标路径");
}

#[test]
fn log_parse_roundtrip() {
    // git 实际格式：%x1e 记录分隔 + 字段全用 \x1f 分隔 + --name-only 文件行
    let text = "\u{1e}abcdef123456\u{1f}abcdef1\u{1f}张伟\u{1f}zw@ex.com\u{1f}1790871442\u{1f}fix: x\nsrc/a.java\nsrc/b.java\n\u{1e}fedcba654321\u{1f}fedcba6\u{1f}李雷\u{1f}ll@ex.com\u{1f}1790870000\u{1f}feat: y\n\n";
    let rows = parse_log(text);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].short, "abcdef1");
    assert_eq!(rows[0].at, 1790871442);
    assert_eq!(rows[0].subject, "fix: x");
    assert_eq!(rows[0].files, vec!["src/a.java".to_string(), "src/b.java".to_string()]);
    assert_eq!(rows[1].files.len(), 0, "空行不产生文件项");
}

#[test]
fn rel_path_guard() {
    assert!(validate_rel_path("src/a.java").is_ok());
    assert!(validate_rel_path("../etc/passwd").is_err());
    assert!(validate_rel_path("a/../../b").is_err());
    assert!(validate_rel_path("/abs/path").is_err());
    assert!(validate_rel_path("").is_err());
}

#[tokio::test]
async fn status_commit_discard_e2e() {
    let repo = std::env::temp_dir().join("ev-git-status-test");
    let _ = std::fs::remove_dir_all(&repo);
    std::fs::create_dir_all(&repo).unwrap();
    let git_sync = |args: &[&str]| {
        std::process::Command::new("git").args(args).current_dir(&repo).output().expect("git 执行失败")
    };
    git_sync(&["init", "-q"]);
    git_sync(&["config", "user.email", "t@t"]);
    git_sync(&["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git_sync(&["add", "."]);
    git_sync(&["commit", "-q", "-m", "init"]);

    // 修改已跟踪 + 新增未跟踪
    std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(repo.join("b_new.txt"), "new\n").unwrap();
    let st = status(&repo).await.unwrap();
    assert_eq!(st.branch, "main", "默认分支（init 默认 main）");
    assert!(st.upstream.is_none(), "无远程 → upstream None");
    let a = st.files.iter().find(|f| f.path == "a.txt").unwrap();
    assert_eq!(a.status, 'M');
    assert_eq!(a.adds, Some(1), "numstat 合并出增行");
    assert_eq!(a.dels, Some(0));
    let b = st.files.iter().find(|f| f.path == "b_new.txt").unwrap();
    assert_eq!(b.status, '?');
    assert_eq!(b.adds, None, "未跟踪文件无 diff 统计");

    // 提交（add -A 含未跟踪）
    let short = commit_all(&repo, "second").await.unwrap();
    assert_eq!(short.len(), 7);
    let st = status(&repo).await.unwrap();
    assert!(st.files.is_empty(), "提交后工作树干净");
    let rows = log(&repo, 10).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].subject, "second");
    assert!(rows[0].files.contains(&"a.txt".to_string()) && rows[0].files.contains(&"b_new.txt".to_string()), "--name-only 带出文件清单: {:?}", rows[0].files);

    // 撤销已跟踪修改 → 内容还原；撤销未跟踪 → 文件删除
    std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
    std::fs::write(repo.join("c_un.txt"), "x\n").unwrap();
    discard(&repo, "a.txt").await.unwrap();
    assert_eq!(std::fs::read_to_string(repo.join("a.txt")).unwrap(), "one\ntwo\n");
    discard(&repo, "c_un.txt").await.unwrap();
    assert!(!repo.join("c_un.txt").exists());

    // 空提交说明被拒
    assert!(commit_all(&repo, "   ").await.is_err());
    let _ = std::fs::remove_dir_all(&repo);
}

#[tokio::test]
async fn commit_all_excludes_bookkeeping_dirs() {
    // R3 C5 回归：把关台提交不得带入 .easyvibe/（巡检写回的 map.json——否则 freshness 立刻自反漂移）
    // 与 .claude/（agent STAR 归档）——产品账簿不进用户提交历史
    let dir = std::env::temp_dir().join("ev-commit-exclude-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".easyvibe/map")).unwrap();
    std::fs::create_dir_all(dir.join(".claude/development_docs")).unwrap();
    // 故意不写 .gitignore：排除必须来自 commit_all 的 pathspec，而非 gitignore 的副作用
    let git_sync = |args: &[&str]| {
        assert!(std::process::Command::new("git").args(args).current_dir(&dir).status().unwrap().success())
    };
    git_sync(&["init", "-q"]);
    git_sync(&["config", "user.email", "t@t"]);
    git_sync(&["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    git_sync(&["add", "."]);
    git_sync(&["commit", "-q", "-m", "init"]);

    // 用户改动 + 账簿变动（等价于巡检写回 map.json 与 STAR 归档）
    std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.join(".easyvibe/map/map.json"), "{}").unwrap();
    std::fs::write(dir.join(".claude/development_docs/t1.json"), "{}").unwrap();

    let short = commit_all(&dir, "feature").await.unwrap();
    assert_eq!(short.len(), 7);
    let rows = log(&dir, 5).await.unwrap();
    assert!(rows[0].files.contains(&"a.txt".to_string()), "用户改动必须提交: {:?}", rows[0].files);
    assert!(
        !rows[0].files.iter().any(|f| f.starts_with(".easyvibe") || f.starts_with(".claude")),
        "产品账簿不得进用户提交历史: {:?}",
        rows[0].files
    );
    assert!(dir.join(".easyvibe/map/map.json").exists() && dir.join(".claude/development_docs/t1.json").exists(), "账簿文件必须留在磁盘（不被提交也不被吞掉）");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn commit_all_works_when_bookkeeping_dirs_are_gitignored() {
    // 回归：.claude 被 .gitignore 忽略时，commit_all 不得因 "The following paths are
    // ignored by one of your .gitignore files" 失败；已跟踪的 .claude 文件改动也不进提交
    let dir = std::env::temp_dir().join("ev-commit-gitignored-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".claude/development_docs")).unwrap();
    std::fs::write(dir.join(".gitignore"), ".claude/\n").unwrap();
    let git_sync = |args: &[&str]| {
        assert!(std::process::Command::new("git").args(args).current_dir(&dir).status().unwrap().success())
    };
    git_sync(&["init", "-q"]);
    git_sync(&["config", "user.email", "t@t"]);
    git_sync(&["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "one\n").unwrap();
    std::fs::write(dir.join(".claude/development_docs/tracked.txt"), "v1\n").unwrap();
    // 被忽略的目录需 -f 才能进历史（模拟"历史上提交过、后来被 ignore"的仓库）
    git_sync(&["add", "a.txt"]);
    git_sync(&["add", "-f", ".claude/development_docs/tracked.txt"]);
    git_sync(&["commit", "-q", "-m", "init"]);

    // 用户改动 + 被忽略目录的新文件 + 已跟踪账簿文件的改动
    std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    std::fs::write(dir.join(".claude/development_docs/t1.json"), "{}\n").unwrap();
    std::fs::write(dir.join(".claude/development_docs/tracked.txt"), "v2\n").unwrap();

    let short = commit_all(&dir, "fix").await.unwrap();
    assert_eq!(short.len(), 7);
    let rows = log(&dir, 5).await.unwrap();
    assert!(rows[0].files.contains(&"a.txt".to_string()), "用户改动必须提交: {:?}", rows[0].files);
    assert!(
        !rows[0].files.iter().any(|f| f.starts_with(".claude")),
        "被忽略/已跟踪的账簿都不得进提交: {:?}",
        rows[0].files
    );
    // 磁盘现状：新文件与未提交的改动都完好
    assert!(dir.join(".claude/development_docs/t1.json").exists());
    assert_eq!(std::fs::read_to_string(dir.join(".claude/development_docs/tracked.txt")).unwrap(), "v2\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn show_commit_parses_header_and_numstat() {
    // 历史记录可点开（用户反馈）：提交详情 = 主题 + 正文 + 逐文件增删
    let dir = std::env::temp_dir().join("ev-show-commit-test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git_sync = |args: &[&str]| {
        assert!(std::process::Command::new("git").args(args).current_dir(&dir).status().unwrap().success())
    };
    git_sync(&["init", "-q"]);
    git_sync(&["config", "user.email", "t@t"]);
    git_sync(&["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "1\n").unwrap();
    git_sync(&["add", "."]);
    git_sync(&["commit", "-q", "-m", "标题行", "-m", "正文段落一\n正文段落二"]);
    let head = std::process::Command::new("git").args(["rev-parse", "HEAD"]).current_dir(&dir).output().unwrap();
    let hash = String::from_utf8_lossy(&head.stdout).trim().to_string();

    let d = show_commit(&dir, &hash).await.unwrap();
    assert_eq!(d.hash, hash);
    assert_eq!(d.subject, "标题行");
    assert!(d.body.contains("正文段落一"), "正文必须带出: {:?}", d.body);
    assert_eq!(d.files.len(), 1);
    assert_eq!(d.files[0].path, "a.txt");
    assert_eq!(d.files[0].adds, 1);

    // 非法 hash 必须 400 而不是进 git
    assert!(show_commit(&dir, "zzzz!!").await.is_err());
    assert!(show_commit(&dir, "abc").await.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// diff 抽屉 e2e：修改 / 未跟踪 / 二进制 / 暂存 / 截断 / 不存在 / 注入防护
#[tokio::test]
async fn file_diff_covers_untracked_binary_staged_truncation() {
    let dir = std::env::temp_dir().join(format!("ev-file-diff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git_sync = |args: &[&str]| {
        assert!(std::process::Command::new("git").args(args).current_dir(&dir).status().unwrap().success())
    };
    git_sync(&["init", "-q"]);
    git_sync(&["config", "user.email", "t@t"]);
    git_sync(&["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "one\ntwo\n").unwrap();
    git_sync(&["add", "."]);
    git_sync(&["commit", "-qm", "init"]);

    // ① 修改已跟踪文件 → unstaged 统一 diff
    std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    let d = diff(&dir, "a.txt", false).await.unwrap();
    assert!(!d.untracked && !d.binary && !d.empty && !d.truncated);
    assert!(d.text.contains("--- a/a.txt"), "文件头: {:?}", d.text);
    assert!(d.text.contains("+++ b/a.txt"));
    assert!(d.text.contains("+TWO"));
    assert!(d.text.contains("-two"));
    assert!(d.text.contains("@@ "), "hunk 头: {:?}", d.text);
    assert_eq!(d.total_lines, d.text.lines().count() as i64);

    // ② staged diff：未暂存时为空；--cached 后带出 index vs HEAD 差异
    let empty = diff(&dir, "a.txt", true).await.unwrap();
    assert!(empty.empty, "未暂存文件的 staged diff 应为空: {:?}", empty.text);
    git_sync(&["add", "a.txt"]);
    let staged = diff(&dir, "a.txt", true).await.unwrap();
    assert!(!staged.empty && staged.text.contains("+three"));
    let unstaged_after_add = diff(&dir, "a.txt", false).await.unwrap();
    assert!(unstaged_after_add.empty, "全部暂存后 unstaged 为空");
    git_sync(&["reset", "-q", "--", "a.txt"]);

    // ③ 未跟踪新文件 → 合成整文件新增 diff
    std::fs::write(dir.join("new.txt"), "n1\nn2\n").unwrap();
    let u = diff(&dir, "new.txt", false).await.unwrap();
    assert!(u.untracked && !u.binary && !u.empty);
    assert!(u.text.contains("--- /dev/null"));
    assert!(u.text.contains("+++ b/new.txt"));
    assert!(u.text.contains("+n1") && u.text.contains("+n2"));
    assert_eq!(u.total_lines, 5, "3 头 + 2 内容行: {:?}", u.text);

    // ④ 二进制文件 → binary 标记，无文本
    std::fs::write(dir.join("bin.dat"), [0u8, 159, 146, 150, 0, 1, 2]).unwrap();
    git_sync(&["add", "-f", "bin.dat"]);
    git_sync(&["commit", "-qm", "add bin"]);
    std::fs::write(dir.join("bin.dat"), [0u8, 159, 146, 150, 9, 9, 9]).unwrap();
    let b = diff(&dir, "bin.dat", false).await.unwrap();
    assert!(b.binary && b.text.is_empty());

    // ⑤ 大 diff 截断保护
    let big: String = (0..DIFF_MAX_LINES as i64 + 500).map(|i| format!("line{i}\n")).collect();
    std::fs::write(dir.join("a.txt"), big).unwrap();
    let t = diff(&dir, "a.txt", false).await.unwrap();
    assert!(t.truncated, "超过 {DIFF_MAX_LINES} 行必须截断");
    assert!(t.total_lines > DIFF_MAX_LINES as i64, "完整行数应超过上限: {}", t.total_lines);
    assert_eq!(t.text.lines().count(), DIFF_MAX_LINES);

    // ⑥ 文件不存在 → NotFound；路径注入 → BadRequest
    assert!(matches!(diff(&dir, "gone.txt", false).await, Err(e) if e.to_string().contains("不存在")));
    assert!(diff(&dir, "../etc/passwd", false).await.is_err());
    assert!(diff(&dir, "-c", false).await.is_err(), "`-` 开头路径必须拒绝");
    assert!(diff(&dir, "", false).await.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn non_git_repo_errs() {
    let dir = std::env::temp_dir().join("ev-not-git-status");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(status(&dir).await.is_err());
    assert!(log(&dir, 10).await.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn discard_all_restores_tracked_and_removes_untracked() {
    // 重审 P2：全部撤销——tracked 恢复 + untracked（含目录）删除
    let dir = std::env::temp_dir().join(format!("ev-discard-all-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"]).await.unwrap();
    std::fs::write(dir.join("tracked.txt"), "v1").unwrap();
    git(&dir, &["add", "."]).await.unwrap();
    git(&dir, &["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]).await.unwrap();
    // 造改动：改 tracked + 新增 untracked 文件与目录
    std::fs::write(dir.join("tracked.txt"), "v2-changed").unwrap();
    std::fs::create_dir_all(dir.join("untracked-dir")).unwrap();
    std::fs::write(dir.join("untracked-dir/new.txt"), "new").unwrap();
    std::fs::write(dir.join("untracked.txt"), "new").unwrap();

    discard_all(&dir).await.unwrap();

    assert_eq!(std::fs::read_to_string(dir.join("tracked.txt")).unwrap(), "v1", "tracked 必须恢复");
    assert!(!dir.join("untracked.txt").exists(), "untracked 文件必须删除");
    assert!(!dir.join("untracked-dir").exists(), "untracked 目录必须删除");
    let st = status(&dir).await.unwrap();
    assert!(st.files.is_empty(), "清理后工作树必须干净: {:?}", st.files);
    let _ = std::fs::remove_dir_all(&dir);
}
