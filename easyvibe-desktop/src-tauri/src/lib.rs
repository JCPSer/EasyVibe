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

/// 关闭确认放行位：用户在确认框点"仍要关闭"后置位，后续 close() 不再拦截（防死循环）
struct CloseConfirmed(std::sync::atomic::AtomicBool);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
            .plugin(tauri_plugin_notification::init())
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
            app.manage(CloseConfirmed(std::sync::atomic::AtomicBool::new(false)));
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
            // spawn 独立函数：壳内唯一 spawn 点（首启与崩溃自重启共用同一组 env）
            fn spawn_sidecar(
                app: &tauri::App,
                static_dir: &std::path::Path,
                prompt_dir: &std::path::Path,
            ) -> Result<
                (
                    tauri::async_runtime::Receiver<tauri_plugin_shell::process::CommandEvent>,
                    tauri_plugin_shell::process::CommandChild,
                ),
                Box<dyn std::error::Error>,
            > {
                let port_str = BACKEND_PORT.to_string();
                let (rx, child) = app
                    .shell()
                    .sidecar("easyvibe-backend")
                    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
                    .env("EASYVIBE_PORT", &port_str)
                    .env("EASYVIBE_STATIC_DIR", static_dir)
                    .env("EASYVIBE_PROMPT_PATH", prompt_dir.join("easyvibe-map-prompt-v2.2.md"))
                    .env("EASYVIBE_PATROL_PROMPT_PATH", prompt_dir.join("easyvibe-map-patrol-prompt.md"))
                    .env("EASYVIBE_SCHEMA_PATH", prompt_dir.join("easyvibe-map-schema-v1.json"))
                    .env("EASYVIBE_SUBMAP_PROMPT_PATH", prompt_dir.join("easyvibe-module-submap-prompt.md"))
                    .spawn()
                    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
                Ok((rx, child))
            }
            // spawn 返回 (事件接收端, 子进程句柄)。消费事件流：Stdout/Stderr 行转发到壳日志
            // （routa pipe_child_logs 模式——后端日志与壳日志汇流到一处，排障只看一个流）
            let (mut rx, child) = spawn_sidecar(app, &static_dir, &prompt_dir)?;
            app.manage(SidecarState(Mutex::new(Some(child))));
            // AppHandle 版：自重启路径用（壳已 setup 完，只有 handle）
            fn spawn_sidecar_handle(
                app: &tauri::AppHandle,
                static_dir: &std::path::Path,
                prompt_dir: &std::path::Path,
            ) -> Result<
                (
                    tauri::async_runtime::Receiver<tauri_plugin_shell::process::CommandEvent>,
                    tauri_plugin_shell::process::CommandChild,
                ),
                Box<dyn std::error::Error>,
            > {
                use tauri::Manager as _;
                let port_str = BACKEND_PORT.to_string();
                let (rx, child) = app
                    .shell()
                    .sidecar("easyvibe-backend")
                    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?
                    .env("EASYVIBE_PORT", &port_str)
                    .env("EASYVIBE_STATIC_DIR", static_dir)
                    .env("EASYVIBE_PROMPT_PATH", prompt_dir.join("easyvibe-map-prompt-v2.2.md"))
                    .env("EASYVIBE_PATROL_PROMPT_PATH", prompt_dir.join("easyvibe-map-patrol-prompt.md"))
                    .env("EASYVIBE_SCHEMA_PATH", prompt_dir.join("easyvibe-map-schema-v1.json"))
                    .env("EASYVIBE_SUBMAP_PROMPT_PATH", prompt_dir.join("easyvibe-module-submap-prompt.md"))
                    .spawn()
                    .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
                Ok((rx, child))
            }
            let app_handle_mon = app.handle().clone();
            let static_dir_mon = static_dir.clone();
            let prompt_dir_mon = prompt_dir.clone();
            tauri::async_runtime::spawn(async move {
                use tauri_plugin_shell::process::CommandEvent;
                loop {
                    match rx.recv().await {
                        Some(CommandEvent::Stdout(line)) => eprintln!("[sidecar][stdout] {}", String::from_utf8_lossy(&line)),
                        Some(CommandEvent::Stderr(line)) => eprintln!("[sidecar][stderr] {}", String::from_utf8_lossy(&line)),
                        Some(CommandEvent::Error(e)) => eprintln!("[sidecar][error] {e}"),
                        Some(CommandEvent::Terminated(payload)) => {
                            eprintln!("[sidecar][terminated] code={:?} signal={:?}——启动自重启", payload.code, payload.signal);
                            // 2026-10-05 白屏根治：运行期后端死亡 = webview 空白且无法自救。
                            // 自重启（3 次退避）→ 健康后通知前端 reload——用户视角只是页面刷新了一下。
                            let mut recovered = false;
                            for attempt in 1..=3u32 {
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                match spawn_sidecar_handle(&app_handle_mon, &static_dir_mon, &prompt_dir_mon) {
                                    Ok((new_rx, child)) => {
                                        rx = new_rx;
                                        if let Some(state) = app_handle_mon.try_state::<SidecarState>() {
                                            *state.0.lock().unwrap() = Some(child);
                                        }
                                        eprintln!("[sidecar] 自重启成功（第 {attempt} 次）");
                                        recovered = true;
                                        break;
                                    }
                                    Err(e) => eprintln!("[sidecar] 自重启失败（第 {attempt} 次）: {e}"),
                                }
                            }
                            if recovered {
                                // 等端口健康再通知前端（最多 15s）
                                for _ in 0..30 {
                                    if std::net::TcpStream::connect(("127.0.0.1", BACKEND_PORT)).is_ok() { break; }
                                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                }
                                use tauri::Emitter as _;
                                let _ = app_handle_mon.emit_to("main", "backend-recovered", ());
                            } else {
                                eprintln!("[sidecar] 自重启三次均失败——请查看 ~/.easyvibe/logs/ 后重启应用");
                            }
                        }
                        Some(_) => {}
                        None => break,
                    }
                }
            });
            }

            // 2026-10-05 白屏自愈补环【已停用——实弹嫌疑：启动期对正在首载的 webview eval(reload)
            // 疑似把 WKWebView 打入永不完成的加载态（Cmd+R 也无效）。先回归验证，再设计更安全的恢复。】

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
                    let builder = WebviewWindowBuilder::new(
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
                    // macOS 原生红绿灯 + 自绘标题栏（参考 multica/Electron hiddenInset 范式）：
                    // Overlay = 隐藏系统标题栏但保留原生交通灯悬浮于内容上方，
                    // 拖拽由前端透明热区负责（见 AppShell 的 startDragging）
                    .title_bar_style(tauri::TitleBarStyle::Overlay)
                    .hidden_title(true);
                    // 2026-10-05 红绿灯纵向居中：系统默认把灯居中在 28pt 幻影子标题栏
                    // （灯心 14pt），我们的自绘 header 是 40px（中线 20pt）——灯偏上。
                    // wry 语义（读源码 + 实测定标）：容器高 = 按钮帧高(28) + y，灯心 = 容器中线。
                    // 实测 y=28 → 灯心 29pt（过头压到底部分隔线）；目标 20pt → y=16（实弹迭代：y=12 仍偏高半档），
                    // 与前端失焦仿灯（x=12/32/52、header 居中）完全同位，聚焦/失焦零跳变。
                    #[cfg(target_os = "macos")]
                    let builder =
                        builder.traffic_light_position(tauri::LogicalPosition::new(12.0, 18.0));
                    builder.build()
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
        // 关闭确认（2026-10-06）：有 agent 会话在跑时关闭 = 静默杀任务（此前的僵尸"进行中"实弹）。
        // 拦截 CloseRequested 查后端活动会话数，>0 先阻止关闭、弹原生确认，确认后经
        // CloseConfirmed 放行位再 close()（否则确认后的 close 会再次触发本拦截死循环）。
        // 查询失败放行——后端不在就没有会话可丢。崩溃/强杀路径由后端启动清扫兜底。
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() != "main" {
                    return;
                }
                if let Some(st) = window.try_state::<CloseConfirmed>() {
                    if st.0.load(std::sync::atomic::Ordering::SeqCst) {
                        return; // 用户已确认，放行
                    }
                }
                let busy = active_session_count();
                if busy > 0 {
                    api.prevent_close();
                    let win = window.clone();
                    use tauri_plugin_dialog::DialogExt;
                    window
                        .dialog()
                        .message(format!(
                            "有 {busy} 个 agent 会话正在运行，关闭将中断它们（任务与进度保留，重启后可重新发起）。"
                        ))
                        .title("关闭确认")
                        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancelCustom(
                            "仍要关闭".to_string(),
                            "取消".to_string(),
                        ))
                        .show(move |ok| {
                            if ok {
                                if let Some(st) = win.try_state::<CloseConfirmed>() {
                                    st.0.store(true, std::sync::atomic::Ordering::SeqCst);
                                }
                                let _ = win.close();
                            }
                        });
                }
            }
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

/// 关闭确认原料：查后端活动会话数（阻塞 ≤1.4s；查不到/解析失败视为 0——后端不在就没有会话可丢）
fn active_session_count() -> usize {
    use std::io::{Read, Write};
    let addr: std::net::SocketAddr = match "127.0.0.1:7151".parse() {
        Ok(a) => a,
        Err(_) => return 0,
    };
    let mut stream = match std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(600)) {
        Ok(s) => s,
        Err(_) => return 0,
    };
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(800)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(800)));
    if stream
        .write_all(b"GET /api/sessions/overview HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return 0;
    }
    let mut buf = Vec::new();
    if stream.read_to_end(&mut buf).is_err() {
        return 0;
    }
    let text = String::from_utf8_lossy(&buf);
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("");
    let v: serde_json::Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return 0,
    };
    v["data"]["active"].as_array().map(|a| a.len()).unwrap_or(0)
        + v["data"]["queued"].as_array().map(|a| a.len()).unwrap_or(0)
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
