//! task 域编排（自 `service.rs` 原样搬迁，零语义改动）。
//!
//! c-arch-7 R1：`routes/task.rs` 与 `routes/dev_docs.rs` 的 handler 编排（含仓储调用与
//! 事件广播）下沉到本域——routes 只保留「解析入参 → 调 service → 映射响应」。
//! 语义逐条等价（分页/limit clamp、`unwrap_or_default` 兜底、错误映射与状态码不变）。

use crate::db_ports::{ApprovalPort as _, EventPort as _, TaskPort as _};
use crate::state::*;
use easyvibe_api_types::TaskActionResult;
use easyvibe_common::ApiError;
use easyvibe_event_bus::{publish, BusEvent};

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
    let row = crate::db_ports::TaskDraft {
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

// ============================================================================
// c-arch-7 R1：routes/task.rs 与 routes/dev_docs.rs 的下沉编排（零语义改动）
// ============================================================================

/// 任务列表（`?conv=` 会话级过滤优先；`limit` 缺省 50、clamp(1,500)）——响应体原样组装。
pub(crate) async fn list_tasks(st: &AppState, id: &str, conv: Option<&str>, limit: i64) -> Result<serde_json::Value, ApiError> {
    let limit = limit.clamp(1, 500);
    let tasks = match conv {
        Some(cid) => st.task_repo.list_by_conversation(cid).await?,
        None => st.task_repo.list(id, limit).await?,
    };
    let items: Vec<serde_json::Value> = tasks
        .into_iter()
        .map(|t| serde_json::json!({
            "id": t.id, "title": t.title, "description": t.description,
            "modules": serde_json::from_str::<serde_json::Value>(&t.modules).unwrap_or_default(),
            "acceptance": t.acceptance, "source": t.source,
            "status": t.status, "trust": t.trust, "error": t.error,
            "gate": t.gate, "sessionId": t.session_id,
            "conversationId": t.conversation_id,
            "originTaskId": t.origin_task_id, "successorTaskId": t.successor_task_id,
            "result": t.result.as_deref().and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok()),
            "createdAt": t.created_at, "updatedAt": t.updated_at,
        }))
        .collect();
    Ok(serde_json::json!({ "success": true, "data": items }))
}

/// 审批留痕列表（diff/审批卡数据源）。
pub(crate) async fn list_task_approvals(st: &AppState, tid: &str) -> Result<serde_json::Value, ApiError> {
    let aps = st.approval_repo.list_by_task(tid).await?;
    Ok(serde_json::json!({ "success": true, "data": aps }))
}

/// 按任务终止：解析任务 → 会话 → kill（任务卡「终止」按钮）。
pub(crate) async fn kill_task(st: &AppState, id: &str, tid: &str) -> Result<(), ApiError> {
    st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.task_repo.get(tid).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {tid} 不存在")))?;
    let sid = task.session_id.ok_or_else(|| ApiError::BadRequest(format!("任务 {tid} 无关联会话（未开始执行）")))?;
    st.session_manager.kill(&sid).await?;
    Ok(())
}

/// 删除任务（管理闭环 P0）：活动会话先 best-effort 终止 → 级联删（库内）→
/// 任务自有归档 development_docs/{tid}.json 一并删除（共享 MEMORY/INDEX 不动）→
/// 内存基线清残留 → 广播 deleted。返工链反链（origin/successor）保留不动。
pub(crate) async fn delete_task(st: &AppState, id: &str, tid: &str) -> Result<(), ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.task_repo.get(tid).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {tid} 不存在")))?;
    if matches!(task.status.as_str(), "running" | "awaiting_approval") {
        if let Some(sid) = &task.session_id {
            // best-effort：会话可能已自然终态（kill 返回 409/404），删除不该被拦
            let _ = st.session_manager.kill(sid).await;
        }
    }
    st.task_repo.delete(tid).await?;
    // 任务自有归档（diff 全文）随任务删除；读端对缺文件本就返回 diff=null
    let archive = repo.root.join(".easyvibe/development_docs").join(format!("{tid}.json"));
    if archive.is_file() {
        let _ = std::fs::remove_file(&archive);
    }
    // 内存基线清残留（删除后采集永远不会来，留着只是泄漏）
    if let Ok(mut m) = st.executor.baselines.lock() {
        m.remove(tid);
    }
    publish(&st.event_bus, BusEvent::TaskStatus { repo: id.into(), task_id: tid.into(), status: "deleted".into(), gate: None });
    Ok(())
}

/// 就地重试：failed/interrupted → pending 重新入队（语义见 TaskExecutor::retry）。
pub(crate) async fn retry_task(st: &AppState, id: &str, tid: &str) -> Result<TaskActionResult, ApiError> {
    st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.executor.retry(tid).await?;
    publish(&st.event_bus, BusEvent::TaskStatus { repo: id.into(), task_id: tid.into(), status: task.status.clone(), gate: task.gate.clone() });
    Ok(TaskActionResult { status: task.status.clone(), gate: task.gate.clone() })
}

/// 修改并复审：rejected → 注入审查意见 → 直达实施阶段重跑（语义见 TaskExecutor::remediate）。
pub(crate) async fn remediate_task(st: &AppState, id: &str, tid: &str) -> Result<TaskActionResult, ApiError> {
    st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.executor.remediate(tid).await?;
    publish(&st.event_bus, BusEvent::TaskStatus { repo: id.into(), task_id: tid.into(), status: task.status.clone(), gate: task.gate.clone() });
    Ok(TaskActionResult { status: task.status.clone(), gate: task.gate.clone() })
}

/// 管道回看·节点重开：放回目标评审关（rewind 自身已广播，此处不再双发）。
pub(crate) async fn rewind_task(st: &AppState, id: &str, tid: &str, target: &str) -> Result<TaskActionResult, ApiError> {
    st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.executor.rewind(tid, target).await?;
    Ok(TaskActionResult { status: task.status.clone(), gate: task.gate.clone() })
}

/// 人工触发子 agent 复审（异步执行；结论经 result.review + 留痕 + 事件送达）。
pub(crate) async fn review_task(st: &AppState, id: &str, tid: &str) -> Result<TaskActionResult, ApiError> {
    st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.executor.review_now(tid).await?;
    Ok(TaskActionResult { status: task.status.clone(), gate: task.gate.clone() })
}

/// 审批决策（approved/rejected 按当前关卡推进或终止；发射 task.statusChanged）。
pub(crate) async fn decide_task(
    st: &AppState,
    id: &str,
    tid: &str,
    decision: &str,
    note: Option<&str>,
    expected_gate: Option<&str>,
) -> Result<TaskActionResult, ApiError> {
    let task = st.executor.decide(tid, decision, note, expected_gate).await?;
    publish(&st.event_bus, BusEvent::TaskStatus {
        repo: id.into(),
        task_id: tid.into(),
        status: task.status.clone(),
        gate: task.gate.clone(),
    });
    Ok(TaskActionResult { status: task.status.clone(), gate: task.gate.clone() })
}

/// 任务完整 diff（M4-3）：development_docs 归档中的 diffFull；无归档/无 diff 返回 null。
pub(crate) async fn task_diff(st: &AppState, id: &str, tid: &str) -> Result<serde_json::Value, ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let path = repo.root.join(".easyvibe/development_docs").join(format!("{tid}.json"));
    if !path.exists() {
        return Ok(serde_json::json!({ "success": true, "data": { "diff": null, "diffStat": null } }));
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| ApiError::Internal(format!("归档读取失败: {e}")))?;
    let v: serde_json::Value = serde_json::from_str(&raw).map_err(|e| ApiError::Internal(format!("归档解析失败: {e}")))?;
    Ok(serde_json::json!({
        "success": true,
        "data": { "diff": v["diffFull"], "diffStat": v["diffStat"] }
    }))
}

/// 智能优化建议（Stub=确定性派生；LLM=地图注入生成）。
pub(crate) async fn suggest_tasks(st: &AppState, id: &str) -> Result<serde_json::Value, ApiError> {
    use easyvibe_ai_agent::SuggestClient as _;
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let snap = st.map_service.load_map(&repo).await?;
    let suggestions: Vec<easyvibe_ai_agent::Suggestion> = match *st.llm_mode {
        LlmMode::Stub => easyvibe_ai_agent::StubSuggestClient.suggest(&snap.json).await?,
        LlmMode::Anthropic => {
            let cfg = resolve_llm(st, id, "chat").await;
            if cfg.api_key.is_empty() {
                return Err(ApiError::BadRequest("未配置 LLM API key（设置面板或 EASYVIBE_LLM_API_KEY）".into()));
            }
            let llm = easyvibe_ai_agent::AnthropicClient::new(&cfg.base_url, &cfg.api_key, &cfg.model);
            easyvibe_ai_agent::LlmSuggestClient::new(llm).suggest(&snap.json).await?
        }
    };
    let items: Vec<serde_json::Value> = suggestions
        .into_iter()
        .map(|sg| serde_json::json!({ "title": sg.title, "description": sg.description, "modules": sg.modules, "priority": sg.priority, "rationale": sg.rationale }))
        .collect();
    Ok(serde_json::json!({ "success": true, "data": items }))
}

/// 任务产物文档（方案 v3 §4.4）：按任务时间窗过滤 development_docs/**。
/// 右端规则：running/pending 用 now()（updatedAt 执行期不刷新），终态用 updatedAt+10min；
/// excerpt 按 char 边界截 200 字；INDEX-/MEMORY-/operation- 单独归类。
pub(crate) async fn list_task_dev_docs(st: &AppState, id: &str, tid: &str) -> Result<serde_json::Value, ApiError> {
    let repo = st.map_service.find_repo(id).await.ok_or_else(|| ApiError::NotFound(format!("仓库 {id} 未注册")))?;
    let task = st.task_repo.get(tid).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {tid} 不存在")))?;
    let docs_root = repo.root.join(".easyvibe/development_docs");
    if !docs_root.is_dir() {
        return Ok(serde_json::json!({ "success": true, "data": { "docs": [], "indices": [] } }));
    }
    // 任务时间戳是 epoch 毫秒串（与 toMs/try_advance_gate 同一口径）
    let parse_ms = |s: &str| s.parse::<i64>().ok().unwrap_or(0);
    let left = parse_ms(&task.created_at) - 10 * 60_000;
    let right = if task.status == "running" || task.status == "pending" {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(i64::MAX)
    } else {
        parse_ms(&task.updated_at) + 10 * 60_000
    };
    let mut docs: Vec<serde_json::Value> = vec![];
    let mut indices: Vec<serde_json::Value> = vec![];
    let mut stack: Vec<std::path::PathBuf> = vec![docs_root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            if !name.ends_with(".md") {
                continue; // *.json 任务归档与 harness 文档混处（task_exec.rs collect），不进文档卡
            }
            let mtime_ms = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            if mtime_ms < left || mtime_ms > right {
                continue;
            }
            let rel = p.strip_prefix(&repo.root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            let is_index = name.starts_with("INDEX-") || name.starts_with("MEMORY-") || name.starts_with("operation-");
            let item = serde_json::json!({ "name": name, "path": rel, "mtime": mtime_ms });
            if is_index {
                indices.push(item);
                continue;
            }
            let excerpt = std::fs::read_to_string(&p)
                .map(|s| s.chars().take(200).collect::<String>())
                .unwrap_or_default();
            docs.push(serde_json::json!({ "name": name, "path": rel, "mtime": mtime_ms, "excerpt": excerpt }));
        }
    }
    docs.sort_by(|a, b| b["mtime"].as_i64().unwrap_or(0).cmp(&a["mtime"].as_i64().unwrap_or(0)));
    Ok(serde_json::json!({ "success": true, "data": { "docs": docs, "indices": indices } }))
}
