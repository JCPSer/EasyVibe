// 桌面壳主逻辑（D5 第一步：壳内嵌 + 手工更新）：
// 1) 启动时 spawn easyvibe-backend sidecar（EASYVIBE_PORT=7151，静态托管打包进 resources 的 dist，
//    提示词同样走 resources，仓库列表读 ~/.easyvibe/desktop-repos）
// 2) 轮询 127.0.0.1:7151 就绪后创建主窗口加载它——前后端同源，CORS/跨站问题整体不存在
// 3) 应用退出时 kill sidecar，避免孤儿后端占端口
use std::sync::Mutex;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_shell::ShellExt;

/// 桌面壳内后端固定端口（开发后端用 7101，互不冲突）
const BACKEND_PORT: u16 = 7151;

struct SidecarState(Mutex<Option<tauri_plugin_shell::process::CommandChild>>);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(|app| {
            let res_dir = app
                .path()
                .resource_dir()
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let static_dir = res_dir.join("resources/dist");
            let prompt_dir = res_dir.join("resources/prompts");

            // 仓库列表：~/.easyvibe/desktop-repos（每行一个仓库根路径；空文件=零仓库起步）
            let repos = std::env::var("HOME")
                .map(|h| {
                    std::fs::read_to_string(format!("{h}/.easyvibe/desktop-repos"))
                        .unwrap_or_default()
                        .lines()
                        .map(|l| l.trim())
                        .filter(|l| !l.is_empty() && !l.starts_with('#'))
                        .collect::<Vec<_>>()
                        .join(",")
                })
                .unwrap_or_default();

            // 端口复用守卫：7151 已有健康后端（如壳异常退出留下的孤儿）则直接复用，
            // 不再 spawn 第二个（第二个 bind 失败即死，还会让"杀 sidecar"误杀别人）
            let already_up = std::net::TcpStream::connect(("127.0.0.1", BACKEND_PORT)).is_ok();
            if already_up {
                app.manage(SidecarState(Mutex::new(None)));
            } else {
            let port_str = BACKEND_PORT.to_string();
            // spawn 返回 (事件接收端, 子进程句柄)；事件流本壳不消费，直接丢弃保留句柄
            let (_rx, child) = app
                .shell()
                .sidecar("easyvibe-backend")
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
                .env("EASYVIBE_PORT", &port_str)
                .env("EASYVIBE_STATIC_DIR", &static_dir)
                .env("EASYVIBE_REPO", &repos)
                .env("EASYVIBE_PROMPT_PATH", prompt_dir.join("easyvibe-map-prompt-v2.2.md"))
                .env("EASYVIBE_PATROL_PROMPT_PATH", prompt_dir.join("easyvibe-map-patrol-prompt.md"))
                .env("EASYVIBE_SCHEMA_PATH", prompt_dir.join("easyvibe-map-schema-v1.json"))
                .env("EASYVIBE_SUBMAP_PROMPT_PATH", prompt_dir.join("easyvibe-module-submap-prompt.md"))
                .spawn()
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            app.manage(SidecarState(Mutex::new(Some(child))));
            }

            // 等后端就绪再开窗（最多 30s；超时也开窗，UI 有自己的健康检查与重连提示）
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                for _ in 0..60 {
                    if std::net::TcpStream::connect(("127.0.0.1", BACKEND_PORT)).is_ok() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                if let Err(e) = WebviewWindowBuilder::new(
                    &app_handle,
                    "main",
                    WebviewUrl::External(
                        format!("http://127.0.0.1:{BACKEND_PORT}")
                            .parse()
                            .expect("合法 URL"),
                    ),
                )
                .title("EasyVibe")
                .inner_size(1500.0, 940.0)
                .min_inner_size(1100.0, 700.0)
                .build()
                {
                    eprintln!("EasyVibe 主窗口创建失败: {e}");
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building EasyVibe desktop");

    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            if let Some(state) = app.try_state::<SidecarState>() {
                if let Ok(mut guard) = state.0.lock() {
                    if let Some(child) = guard.take() {
                        let _ = child.kill();
                    }
                }
            }
        }
    });
}
