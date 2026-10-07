//! 子图分析流程编排（c-arch-13 R2：自 `service/map.rs` 原样搬迁，零语义改动）。
//!
//! 透明 agent 扫描模块文件产出子图，与归纳共用写互斥/会话机制。
//! 共享前置 `global_custom_block` 留驻 `service::map`（唯一规范路径）。

use crate::assets::{self, resolve_text_asset};
use crate::service::map::global_custom_block;
use crate::state::*;
use easyvibe_ai_agent::agent_conf;
use easyvibe_common::ApiError;
use tracing::info;

/// ④ 子图深入分析（试用反馈"子图加载失败"根因修复——v2.2 归纳不产子图，文件无人生产；
/// 此处把缺口变为能力：透明 agent 扫描模块文件产出子图，落盘 .easyvibe/modules/<id>.json，
/// 与归纳共用写互斥/会话机制。读时拉取无需 watcher，产出后重新展开即见）
pub(crate) async fn analyze_submap(st: &AppState, id: &str, module_id: &str) -> Result<serde_json::Value, ApiError> {
    if !easyvibe_map::is_valid_id(module_id) {
        return Err(ApiError::BadRequest(format!("非法模块 id: {module_id}")));
    }
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    let module = snap.json["modules"]
        .as_array()
        .and_then(|ms| ms.iter().find(|m| m["id"].as_str() == Some(module_id)).cloned())
        .ok_or_else(|| ApiError::NotFound(format!("模块 {module_id} 不在主地图中")))?;
    // 子图提示词每次请求重读（产品内置协议迭代快——避免"改了提示词要重启后端"的叠加
    // （本次实弹：路径修正后的提示词因后端未重启而仍用旧版，DeskWar 两次分析空跑）。
    // 读取失败回退启动时装载的副本，绝不阻断
    let submap_spec = assets::spec("submap");
    let (template, _) = resolve_text_asset(submap_spec.env, submap_spec.name, &st.submap_prompt, submap_spec.persist);
    ensure_agent_available(st)?;
    let prompt = template
        .replace("<REPO_ROOT>", &repo.root.to_string_lossy())
        .replace("<MODULE_ID>", module_id)
        .replace("<MODULE_JSON>", &serde_json::to_string(&module).unwrap_or_default());
    // 注入点 #6：自定义 global 块追加到子图分析 prompt 尾部
    let prompt = format!("{prompt}{}", global_custom_block(&st.harness).await);
    let resolved = agent_conf::resolve_agent(&st.settings_repo, None, &st.agent_command, &st.agent_args).await;
    let session = st
        .session_manager
        .start_induction(&repo.id, &repo.root, &prompt, &resolved.command, &resolved.args, None)
        .await?;
    // I1：气泡标签「分析模块 {name}」（name 缺失降级为 id）
    let module_name = module["name"].as_str().unwrap_or(module_id).to_string();
    st.session_manager.note_label(&session.session_id, format!("分析模块 {module_name}")).await;
    // L1 归因：子图会话直接归属模块（任务会话在 task_exec 经 tasks.modules 反查）
    if let Err(err) = st.agent_session_repo.set_module_id(&session.session_id, module_id).await {
        tracing::warn!("[agent_sessions] 子图归因失败 {}: {err}", session.session_id);
    }
    info!("[submap] 模块 {} 子图分析会话 {} 已启动", module_id, session.session_id);
    Ok(serde_json::json!(session))
}
