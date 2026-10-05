//! 产物副本一致性守卫的 cargo test 薄封装（R4/R6）。
//!
//! 唯一实现在 `scripts/verify_assets.py`（跨平台 python3）；此处只调用同一入口，不重复实现。
//! 与 `module_size_guard.rs` / `contract_guard.rs` 同范式：只读、不 import 被测符号、毫秒级。
//! `--check` 对「副本不存在」（CI 全新 checkout）记 SKIP，故本测试在无产物的环境同样成立。

use std::path::PathBuf;
use std::process::Command;

/// 仓库根 = CARGO_MANIFEST_DIR/../../..（crates/easyvibe-app → crates → easyvibe-backend → root）
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[test]
fn asset_sources_and_copies_are_consistent() {
    let root = repo_root();
    let script = root.join("scripts/verify_assets.py");
    assert!(script.is_file(), "缺守卫实现: {}", script.display());
    let out = Command::new("python3")
        .arg(&script)
        .arg("--check")
        .current_dir(&root)
        .output()
        .expect("无法执行 python3（跨平台守卫要求 python3 在 PATH）");
    assert!(
        out.status.success(),
        "产物一致性守卫失败:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
