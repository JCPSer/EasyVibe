//! task 域编排（自 `service.rs` 原样搬迁，零语义改动）。

use crate::state::*;
use easyvibe_common::ApiError;

#[derive(serde::Deserialize)]
pub(crate) struct CreateTaskRequest {
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub acceptance: String,
    #[serde(default)]
    pub source: String, // module / concern / layer / manual
    #[serde(default)]
    pub context: serde_json::Value,
    #[serde(default)]
    pub trust: String, // manual / auto
    /// M4-2：任务←→会话关联（对话升级/工作台聚合）
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// R3 D2：返工来源（驳回→复制为新任务）——后端统一注入驳回理由并回填反链，
    /// 三个前端入口（评审/收件箱/任务页）不必各自拼理由
    #[serde(default)]
    pub origin_task_id: Option<String>,
}

/// 影响面合约展开：声明模块 → 其 files glob 边界（地图缺失/模块未命中时静默不约束——
/// 合约是增强护栏不是准入门槛，与"提示不拦截"哲学一致）
pub(crate) async fn task_ctx_with_contract(
    st: &AppState,
    repo: &easyvibe_map::Repo,
    modules: &[String],
    mut ctx: serde_json::Value,
) -> serde_json::Value {
    if modules.is_empty() {
        return ctx;
    }
    let Ok(snap) = st.map_service.load_map(repo).await else { return ctx };
    let mut patterns: Vec<String> = vec![];
    if let Some(mods) = snap.json["modules"].as_array() {
        for mid in modules {
            if let Some(m) = mods.iter().find(|m| m["id"].as_str() == Some(mid.as_str())) {
                if let Some(files) = m["files"].as_array() {
                    patterns.extend(files.iter().filter_map(|f| f.as_str().map(str::to_string)));
                }
            }
        }
    }
    if !patterns.is_empty() {
        ctx["contract"] = serde_json::json!({ "modules": modules, "patterns": patterns });
    }
    ctx
}

/// 建任务编排（从 `routes/task.rs::create_task` 原样搬迁，零语义改动）。
/// 返回新任务 id；HTTP 边界（201/响应包装）留在 handler。
pub(crate) async fn create_task(st: &AppState, id: &str, body: CreateTaskRequest) -> Result<String, ApiError> {
    use easyvibe_db::{ApprovalRepository as _, EventRepository as _, TaskRepository as _};
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    if body.description.trim().is_empty() {
        return Err(ApiError::BadRequest("需求描述不能为空".into()));
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0).to_string();
    let task_id = format!("task-{}", &now);
    let repo_id = repo.id.clone();
    // R3 D2：返工来源——三入口统一在后端注入驳回理由（此前只有评审页注入，记忆强度取决于入口），
    // 并回填原任务的 successor 反链（返工率聚合的命脉）。
    // 前端惯例放在 context.origin_task_id（TaskDraft.context 透传），兼容顶层 origin_task_id
    let origin_input = body.origin_task_id.or_else(|| {
        body.context
            .get("origin_task_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    });
    let mut description = body.description;
    let mut origin_task_id = None;
    if let Some(origin_id) = origin_input.as_deref() {
        if let Some(origin) = st.task_repo.get(origin_id).await? {
            origin_task_id = Some(origin.id.clone());
            let reason = st
                .approval_repo
                .list_by_task(&origin.id)
                .await
                .ok()
                .map(|aps| {
                    aps.into_iter()
                        .filter(|a| a.decision == "rejected" && a.note.as_deref().map(str::trim).is_some_and(|n| !n.is_empty()))
                        .map(|a| a.note.unwrap())
                        .collect::<Vec<_>>()
                        .join("；")
                })
                .filter(|r| !r.is_empty());
            if let Some(r) = reason {
                description = format!("{description}\n\n—— 返工自 {}（驳回理由：{}）", origin.id, r);
            } else {
                description = format!("{description}\n\n—— 返工自 {}", origin.id);
            }
        }
    }
    let contract_ctx = task_ctx_with_contract(st, &repo, &body.modules, body.context.clone()).await;
    let has_contract = contract_ctx.get("contract").is_some();
    let row = easyvibe_db::TaskRow {
        id: task_id.clone(),
        repo: repo_id.clone(),
        title: body.title,
        description,
        modules: serde_json::to_string(&body.modules).unwrap_or_else(|_| "[]".into()),
        acceptance: body.acceptance,
        source: if body.source.is_empty() { "manual".into() } else { body.source },
        // 影响面合约（战略审查第一 P0）：声明的模块在创建时展开为 glob 边界写入 context——
        // 终态采集对全部变更文件做确定性越界校验（task_exec::collect_task_result），零 LLM。
        context: serde_json::to_string(&contract_ctx).unwrap_or_else(|_| "{}".into()),
        status: "pending".into(), // M3-3：harness 执行引擎接走
        trust: match body.trust.as_str() {
            "auto" => "auto".into(),
            "supervised" => "supervised".into(),
            _ => "manual".into(),
        },
        error: None,
        session_id: None,
        gate: None,
        conversation_id: body.conversation_id,
        prompt_tokens: None,
        completion_tokens: None,
        result: None,
        base_head: None,
        created_at: now.clone(),
        updated_at: now,
        origin_task_id,
        successor_task_id: None,
    };
    st.task_repo.create(&row).await?;
    // 返工链反链回填（原任务 → 本任务）
    if let Some(origin_id) = row.origin_task_id.as_deref() {
        let _ = st.task_repo.set_successor(origin_id, &task_id).await;
    }
    // R3 D1：影响模块填写率/合约声明率——L3 门①的原始计数
    let _ = st
        .event_repo
        .record(&repo_id, "task.created", &serde_json::json!({ "trust": row.trust, "modules": body.modules.len(), "hasContract": has_contract, "rework": row.origin_task_id.is_some() }).to_string())
        .await;
    // M3-3：入队执行（harness 引擎；并发上限 4，审批门 M3-4 接入）
    st.executor.clone().enqueue_pending(Some(&repo_id)).await;
    Ok(task_id)
}
