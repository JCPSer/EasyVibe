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
        // D5-2：单实例锁——第二个实例直接把已有窗口拉到前台（routa 反面教材：双开抢端口）
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.unminimize();
                let _ = win.show();
                let _ = win.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        // 启动错误页走自定义协议（v2 无 WebviewUrl::Html；everr:// 始终可服务，不依赖后端）
        .register_uri_scheme_protocol("everr", |_app, _req| {
            let log_hint = std::env::var("HOME")
                .map(|h| format!("{h}/.easyvibe/logs/"))
                .unwrap_or_else(|_| "~/.easyvibe/logs/".into());
            let html = STARTUP_ERROR_HTML
                .replace("__API__", &format!("http://127.0.0.1:{BACKEND_PORT}"))
                .replace("__LOG__", &log_hint);
            tauri::http::Response::builder()
                .header("Content-Type", "text/html; charset=utf-8")
                .body(html.as_bytes().to_vec())
                .expect("错误页响应构建")
        })
        .setup(|app| {
            let res_dir = app
                .path()
                .resource_dir()
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            let static_dir = res_dir.join("resources/dist");
            let prompt_dir = res_dir.join("resources/prompts");

            // 仓库列表不再由壳代传：后端启动时自己读 ~/.easyvibe/desktop-repos（后端独占该文件，
            // 应用内添加/注销由后端改写，壳不感知）

            // 端口复用守卫：7151 已有健康后端（如壳异常退出留下的孤儿）则直接复用，
            // 不再 spawn 第二个（第二个 bind 失败即死，还会让"杀 sidecar"误杀别人）
            let already_up = std::net::TcpStream::connect(("127.0.0.1", BACKEND_PORT)).is_ok();
            if already_up {
                app.manage(SidecarState(Mutex::new(None)));
            } else {
            let port_str = BACKEND_PORT.to_string();
            // spawn 返回 (事件接收端, 子进程句柄)。消费事件流：Stdout/Stderr 行转发到壳日志
            // （routa pipe_child_logs 模式——后端日志与壳日志汇流到一处，排障只看一个流）
            let (rx, child) = app
                .shell()
                .sidecar("easyvibe-backend")
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
                .env("EASYVIBE_PORT", &port_str)
                .env("EASYVIBE_STATIC_DIR", &static_dir)
                .env("EASYVIBE_PROMPT_PATH", prompt_dir.join("easyvibe-map-prompt-v2.2.md"))
                .env("EASYVIBE_PATROL_PROMPT_PATH", prompt_dir.join("easyvibe-map-patrol-prompt.md"))
                .env("EASYVIBE_SCHEMA_PATH", prompt_dir.join("easyvibe-map-schema-v1.json"))
                .env("EASYVIBE_SUBMAP_PROMPT_PATH", prompt_dir.join("easyvibe-module-submap-prompt.md"))
                .spawn()
                .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            app.manage(SidecarState(Mutex::new(Some(child))));
            tauri::async_runtime::spawn(async move {
                use tauri_plugin_shell::process::CommandEvent;
                let mut rx = rx;
                while let Some(event) = rx.recv().await {
                    match event {
                        CommandEvent::Stdout(line) => eprintln!("[sidecar][stdout] {}", String::from_utf8_lossy(&line)),
                        CommandEvent::Stderr(line) => eprintln!("[sidecar][stderr] {}", String::from_utf8_lossy(&line)),
                        CommandEvent::Error(e) => eprintln!("[sidecar][error] {e}"),
                        CommandEvent::Terminated(payload) => {
                            eprintln!("[sidecar][terminated] code={:?} signal={:?}", payload.code, payload.signal)
                        }
                        _ => {}
                    }
                }
            });
            }

            // 等后端就绪再开窗（最多 30s）；超时渲染启动错误页而非白屏（routa 模式：
            // 给出 API 地址 / 日志位置 / 重试按钮——企业级产品不展示裸错误）
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut ready = false;
                for _ in 0..60 {
                    if std::net::TcpStream::connect(("127.0.0.1", BACKEND_PORT)).is_ok() {
                        ready = true;
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                let result = if ready {
                    WebviewWindowBuilder::new(
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
                    // 自绘标题栏（VSCode 范式）：前端渲染红绿灯与拖拽区，系统横条退役
                    .decorations(false)
                    .build()
                } else {
                    WebviewWindowBuilder::new(
                        &app_handle,
                        "main",
                        WebviewUrl::CustomProtocol("everr://localhost/error".parse().expect("合法 URL")),
                    )
                    .title("EasyVibe — 启动失败")
                    .inner_size(680.0, 420.0)
                    .resizable(false)
                    .build()
                };
                if let Err(e) = result {
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

/// 后端 30s 未就绪时的启动错误页（内嵌 HTML，无外部依赖）
const STARTUP_ERROR_HTML: &str = r#"<!doctype html>
<html lang="zh"><head><meta charset="utf-8"><title>EasyVibe 启动失败</title>
<style>
  body { font-family: -apple-system, system-ui, sans-serif; background: #f8fafc; color: #334155;
         display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }
  .card { background: #fff; border: 1px solid #e2e8f0; border-radius: 12px; padding: 28px 32px; max-width: 520px; }
  h1 { font-size: 17px; margin: 0 0 10px; color: #b91c1c; }
  p { font-size: 13px; line-height: 1.8; margin: 6px 0; }
  code { background: #f1f5f9; padding: 1px 6px; border-radius: 4px; font-size: 12px; }
  button { margin-top: 14px; background: #2563eb; color: #fff; border: 0; border-radius: 8px;
           padding: 8px 20px; font-size: 13px; cursor: pointer; }
</style></head><body><div class="card">
  <h1>EasyVibe 后端未能启动</h1>
  <p>预期 API 地址：<code>__API__</code></p>
  <p>后端日志目录：<code>__LOG__</code>（按日期滚动，诊断请贴最新一份）</p>
  <p>常见原因：sidecar 二进制缺失/损坏、端口被占用、提示词资源缺失。</p>
  <button onclick="location.reload()">重试</button>
</div></body></html>"#;
