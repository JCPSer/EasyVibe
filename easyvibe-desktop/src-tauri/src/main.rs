// 防直接 `cargo run` 时误用：入口统一走 lib（tauri 宏要求 main 位于 bin）
fn main() {
    easyvibe_desktop_lib::run();
}
