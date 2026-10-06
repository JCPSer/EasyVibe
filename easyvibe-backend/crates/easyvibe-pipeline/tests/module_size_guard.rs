//! R10 防回胀 / 方向守卫（easyvibe-pipeline）：文件集快照 + LOC + 禁用依赖面 + harness 注入签名。

use std::path::PathBuf;

fn src() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rs_files() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(src())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "rs").unwrap_or(false))
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        .collect();
    v.sort();
    v
}

fn read(f: &str) -> String {
    std::fs::read_to_string(src().join(f)).unwrap_or_else(|e| panic!("读取 {f} 失败: {e}"))
}

const FROZEN_FILES: &[&str] = &["lib.rs"];

/// 禁止出现的依赖面：应用层状态 / HTTP 框架 / task-engine 反向引用。
const FORBIDDEN: &[&str] = &["axum", "AppState", "easyvibe_app", "crate::task_exec", "easyvibe_db"];

#[test]
fn file_set_is_frozen() {
    let mut expected: Vec<String> = FROZEN_FILES.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(rs_files(), expected, "easyvibe-pipeline 文件集漂移——须登记快照（R10）");
}

#[test]
fn files_stay_below_size_limits() {
    for f in rs_files() {
        let n = read(&f).lines().count();
        assert!(n <= 300, "easyvibe-pipeline/src/{f} 超 300 行: {n}");
    }
}

#[test]
fn no_upward_dependencies() {
    for f in rs_files() {
        let txt = read(&f);
        for bad in FORBIDDEN {
            assert!(!txt.contains(bad), "easyvibe-pipeline/src/{f} 出现禁用依赖面 `{bad}`——不得反向依赖应用层/任务引擎（R10）");
        }
    }
}

#[test]
fn harness_injection_stays_plain_string() {
    // 防 harness 回流：入口必须以已解析字符串接收 custom global 块，不得吃 Harness 类型
    let txt = read("lib.rs");
    assert!(txt.contains("auto_init_suffix: String"), "入口签名必须保留 auto_init_suffix: String（防 easyvibe-pipeline → task-engine 反向依赖，R10）");
    assert!(!txt.contains("Harness"), "入口不得感知 Harness 类型（R10）");
}
