//! 任务执行引擎（M3-3）：pending 任务 → 组装 harness 上下文 → spawn agent 执行 → 状态回写。
//! 分工：路由/拷问/豁免由 LLM 决断（注入框架+规则正文，§9 定稿）；
//! 并行上限 4（§11 🟡7）；透明 agent 不注入 grill-me（§9 #4）。
use easyvibe_common::ApiError;
use easyvibe_db::{TaskRepository as _, TaskRow};
use easyvibe_map::MapService;
use easyvibe_session::SessionManager;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{info, warn};

mod changes;
mod contract;
mod harness;
mod prompt;
mod review;
pub use changes::*;
pub use contract::*;
pub use harness::*;
pub use prompt::*;
pub use review::*;

pub struct TaskExecutor {
    pub task_repo: Arc<easyvibe_db::SqliteTaskRepository>,
    pub approval_repo: Arc<easyvibe_db::SqliteApprovalRepository>,
    /// L1 归因：任务会话归属影响模块（tasks.modules 首个模块 id）。
    /// Option 的原因：12 个测试构造点没有 DB 池——new() 保持原 10 参签名，
    /// 生产路径走 new_with_sessions() 注入
    pub agent_session_repo: Option<Arc<easyvibe_db::AgentSessionRepo>>,
    pub session_manager: Arc<SessionManager>,
    pub map_service: Arc<MapService>,
    /// harness（单一事实源，恢复默认后热换——spawn 时现读，不缓存快照）
    pub harness: Arc<tokio::sync::RwLock<Harness>>,
    /// Y5：设置仓储——spawn 时解析槽位级 agent 参数（任务槽可单独收紧权限）
    pub settings_repo: Arc<easyvibe_db::SqliteSettingsRepository>,
    pub agent_command: Arc<String>,
    pub agent_args: Arc<Vec<String>>,
    /// 并行执行上限（§11 🟡7 = 4）
    pub permits: Arc<Semaphore>,
    /// 影响面合约·启动基线：task_id → 启动时工作区脏文件快照（采集时扣减，防冤案）。
    /// 内存态即可：spawn 与采集同进程；重启把 running 任务标 interrupted，基线随之作废。
    pub baselines: Arc<std::sync::Mutex<std::collections::HashMap<String, Vec<String>>>>,
    /// R2 裂缝#3：越界事件出口——auto/supervised 任务无审批关，越界必须主动送达（WS→通知）
    pub events: Option<tokio::sync::broadcast::Sender<easyvibe_event_bus::BusEvent>>,
}

impl TaskExecutor {
    pub fn new(
        task_repo: Arc<easyvibe_db::SqliteTaskRepository>,
        approval_repo: Arc<easyvibe_db::SqliteApprovalRepository>,
        session_manager: Arc<SessionManager>,
        map_service: Arc<MapService>,
        harness: Arc<tokio::sync::RwLock<Harness>>,
        agent_command: Arc<String>,
        agent_args: Arc<Vec<String>>,
        max_parallel: usize,
        settings_repo: Arc<easyvibe_db::SqliteSettingsRepository>,
        events: Option<tokio::sync::broadcast::Sender<easyvibe_event_bus::BusEvent>>,
    ) -> Arc<Self> {
        Self::assemble(task_repo, approval_repo, None, session_manager, map_service, harness, agent_command, agent_args, max_parallel, settings_repo, events)
    }

    /// 生产路径：注入 agent 会话仓储（L1 归因写库）
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_sessions(
        task_repo: Arc<easyvibe_db::SqliteTaskRepository>,
        approval_repo: Arc<easyvibe_db::SqliteApprovalRepository>,
        agent_session_repo: Arc<easyvibe_db::AgentSessionRepo>,
        session_manager: Arc<SessionManager>,
        map_service: Arc<MapService>,
        harness: Arc<tokio::sync::RwLock<Harness>>,
        agent_command: Arc<String>,
        agent_args: Arc<Vec<String>>,
        max_parallel: usize,
        settings_repo: Arc<easyvibe_db::SqliteSettingsRepository>,
        events: Option<tokio::sync::broadcast::Sender<easyvibe_event_bus::BusEvent>>,
    ) -> Arc<Self> {
        Self::assemble(task_repo, approval_repo, Some(agent_session_repo), session_manager, map_service, harness, agent_command, agent_args, max_parallel, settings_repo, events)
    }

    #[allow(clippy::too_many_arguments)]
    fn assemble(
        task_repo: Arc<easyvibe_db::SqliteTaskRepository>,
        approval_repo: Arc<easyvibe_db::SqliteApprovalRepository>,
        agent_session_repo: Option<Arc<easyvibe_db::AgentSessionRepo>>,
        session_manager: Arc<SessionManager>,
        map_service: Arc<MapService>,
        harness: Arc<tokio::sync::RwLock<Harness>>,
        agent_command: Arc<String>,
        agent_args: Arc<Vec<String>>,
        max_parallel: usize,
        settings_repo: Arc<easyvibe_db::SqliteSettingsRepository>,
        events: Option<tokio::sync::broadcast::Sender<easyvibe_event_bus::BusEvent>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            task_repo,
            approval_repo,
            agent_session_repo,
            session_manager,
            map_service,
            harness,
            agent_command,
            agent_args,
            settings_repo,
            // Y2 清债：并发上限可配（env/启动配置传入），默认 4 不再焊死
            permits: Arc::new(Semaphore::new(max_parallel.max(1))),
            baselines: Default::default(),
            events,
        })
    }

    /// 扫描 pending 任务并执行（启动恢复 + 创建后触发共用）
    pub fn enqueue_pending<'a>(self: &'a Arc<Self>, repo_filter: Option<&'a str>) -> impl std::future::Future<Output = ()> + Send + 'a {
        async move {
        let repos: Vec<String> = match repo_filter {
            Some(r) => vec![r.to_string()],
            None => self.map_service.repos().await.into_iter().map(|r| r.id).collect(),
        };
        for repo_id in repos {
            let Ok(tasks) = self.task_repo.list(&repo_id, 50).await else { continue };
            for t in tasks.into_iter().filter(|t| t.status == "pending") {
                self.clone().execute(t).await;
            }
        }
        }
    }

    /// 执行单个任务（F5 信任分流，改进#7 三档）：
    /// - manual：停在计划审批关（不 spawn）
    /// - supervised：计划时风险预评估（v1 确定性规则；LLM 评估为记档增强）——
    ///   低危直通（plan 留痕带理由），高危停在 plan 关并 flagged 留痕（审批人可见风险理由）
    /// - auto：直通（三道关 skipped 留痕）
    /// Conflict/并发满载的排队语义在 spawn_and_watch。
    pub fn execute(self: Arc<Self>, task: TaskRow) -> impl std::future::Future<Output = ()> + Send {
        async move {
            // 2026-10-05 实弹 bug（治理任务进度清零）：写互斥/并发满退回 pending 的任务
            // gate 带着 p: 阶段标记——必须原地重跑该阶段。此前无脑走 trust 分流，
            // manual 分支把 gate 重置回 plan = 已评审通过的矩阵/方案全部作废、
            // 文档卡消失，用户被迫从头再走一遍（且每 5s 循环一次直至互斥释放）。
            if let Some(phase_gate) = task.gate.as_deref().and_then(|g| g.strip_prefix("p:")) {
                let phase = match phase_gate {
                    "analysis" => 1u8,
                    "solution" => 2,
                    _ => 3,
                };
                info!("[task-exec] 任务 {} 写互斥退回复跑：p:{} 阶段原地重跑（不回计划关）", task.id, phase_gate);
                let _ = self.task_repo.update_status(&task.id, "running", None).await;
                // 不在这里广播 running——spawn 未必成功（permit/槽位仍满会退回 pending），
                // 早产广播 = 前端每 5s 看到一次 running→pending 闪跳（2026-10-06 实弹）。
                // running 由 spawn 成功后的广播统一宣布；失败路径广播 pending。
                self.spawn_and_watch(task, phase).await;
                return;
            }
            match task.trust.as_str() {
                "manual" => {
                    let _ = self.task_repo.update_status(&task.id, "awaiting_approval", None).await;
                    let _ = self.task_repo.set_gate(&task.id, Some("plan")).await;
                    // P0（ui-test-2026-10-03）：停审批关必须广播——前端列表/徽标/注意力条
                    // 全靠 task.statusChanged 刷新；不发 = 任务在前端"凭空消失"，审批闭环断裂
                    self.publish_status(&task.repo, &task.id, "awaiting_approval", Some("plan")).await;
                    info!("[task-exec] 任务 {} 等待计划审批（manual）", task.id);
                    return;
                }
                "supervised" => {
                    let (high, reason) = risk_assess(&task);
                    if high {
                        let _ = self.task_repo.update_status(&task.id, "awaiting_approval", None).await;
                        let _ = self.task_repo.set_gate(&task.id, Some("plan")).await;
                        self.record_approval(&task.id, "plan", "flagged", Some(&format!("监督模式风险预评估：{reason}——已停在计划审批关"))).await;
                        self.publish_status(&task.repo, &task.id, "awaiting_approval", Some("plan")).await;
                        info!("[task-exec] 任务 {} 风险预评估高危，停计划关（supervised）", task.id);
                        return;
                    }
                    // 盲测 P0 修复（docs/blind-test-2026-10-02）：低危监督此前 diff/report 两关留痕"skipped"直通，
                    // 用户点一次"通过"后 18 个文件直接落盘——"监督与自动无法区分，核心卖点崩塌"。
                    // 新语义：低危只自动过计划关，执行完成后必须停在 diff 关人工审 diff、
                    // 再过报告关——三道关对监督模式真实存在（manual 是逐关前置审批，supervised 是计划关风险预评估）
                    self.record_approval(&task.id, "plan", "skipped", Some(&format!("监督模式风险预评估：{reason}——低危，计划关自动通过"))).await;
                    info!("[task-exec] 任务 {} 风险预评估低危，计划关自动通过；执行后停 diff 关（supervised）", task.id);
                    let _ = self.task_repo.update_status(&task.id, "running", None).await;
                    self.spawn_and_watch(task, 0).await;
                    return;
                }
                _ => {}
            }
            for gate in ["plan", "diff", "report"] {
                self.record_approval(&task.id, gate, "skipped", Some("自动模式直通，全程留痕")).await;
            }
            let _ = self.task_repo.update_status(&task.id, "running", None).await;
            self.spawn_and_watch(task, 0).await;
        }
    }

    /// 审批决策（路由层调用）：approved 按关卡推进；rejected 分两路——
    /// plan 关（任务书）驳回=终态（返工走复制为新任务），其余评审关（analysis/solution/diff/report）
    /// 驳回=带意见原地重跑本阶段（2026-10-05 用户裁定：与需求分析/方案设计同一打回机制，覆盖代码审查）。
    /// M4-2：驳回必须给理由（审批中心定稿），空理由 400。
    /// expected_gate：调用方所见关卡（N27 防双击穿透）；None 回退服务端当前关卡。
    pub async fn decide(self: &Arc<Self>, task_id: &str, decision: &str, note: Option<&str>, expected_gate: Option<&str>) -> Result<easyvibe_db::TaskRow, ApiError> {
        // P0 审查后端#2：审批是状态机迁移，不是自由函数——
        // ① decision 白名单（此前任何非 "rejected" 字符串都被当 approved）；
        // ② 仅 awaiting_approval 可审批（此前对 pending/failed/interrupted  decide 会复活并重新 spawn，
        //   与「驳回=终态、返工走复制新任务」的设计形成未设计旁路）。
        if !matches!(decision, "approved" | "rejected") {
            return Err(ApiError::BadRequest(format!("非法审批决定: {decision}（仅 approved/rejected）")));
        }
        if decision == "rejected" && note.map(str::trim).unwrap_or_default().is_empty() {
            return Err(ApiError::BadRequest("驳回必须填写理由（留痕可追溯）".into()));
        }
        let task = self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {task_id} 不存在")))?;
        if task.status != "awaiting_approval" {
            return Err(ApiError::Conflict(format!(
                "任务当前状态 {} 不可审批（仅 awaiting_approval 可 decide——打回由评审关裁决，终态任务请重试/修改复审/复制为新任务）",
                task.status
            )));
        }
        // N27 防穿透：以用户所见关卡为抢占条件——双击时第二发命中"关卡已推进"，原子 UPDATE 返回 0 行
        let gate = expected_gate.map(str::to_string).unwrap_or_else(|| task.gate.clone().unwrap_or_else(|| "plan".into()));
        // N27 原子化：先算目标态，再用条件 UPDATE 一次性抢占——影响行数 0 = 并发审批/状态漂移，409。
        // 审批留痕挪到抢占成功之后（此前先留痕再迁移，双击会双留痕 + 双 spawn）。
        // 分阶段执行（2026-10-03 用户裁定）：manual 批准后不再一口气跑完——
        // plan → 阶段1 需求矩阵（p:analysis）→ 评审 → 阶段2 方案（p:solution）→ 评审 → 阶段3 实施。
        // 需求矩阵/方案设计在实施前必须经用户评审（rule_development 2.1/2.2 硬规定）。
        // 2026-10-05 用户裁定：评审关打回 = 带意见原地重跑本阶段（覆盖 analysis/solution/diff/report），
        // 不再落终态 rejected——代码审查发现问题与矩阵/方案打回同一处置：意见注入 context.remediation，
        // 重跑完成后照旧走后续链（阶段1/2 → 初审 → 评审关；阶段3 → 采集 → 子 agent 复审 → diff 关）。
        let rework_phase: Option<u8> = match (decision, gate.as_str()) {
            ("rejected", "analysis") => Some(1),
            ("rejected", "solution") => Some(2),
            ("rejected", "diff") | ("rejected", "report") => Some(3),
            _ => None,
        };
        let (new_gate, new_status) = match (decision, gate.as_str()) {
            ("rejected", "plan") => (Some("rejected"), Some("rejected")),
            ("rejected", _) => (
                Some(match rework_phase {
                    Some(1) => "p:analysis",
                    Some(2) => "p:solution",
                    _ => "p:implement",
                }),
                Some("running"),
            ),
            (_, "plan") => (Some("p:analysis"), Some("running")),
            (_, "analysis") => (Some("p:solution"), Some("running")),
            (_, "solution") => (Some("p:implement"), Some("running")),
            (_, "diff") => (Some("report"), None),
            (_, "report") => (Some("done"), Some("done")),
            _ => return Err(ApiError::BadRequest(format!("未知关卡 {gate}"))),
        };
        let n = self.task_repo.try_advance_gate(&task.id, Some(&gate), new_gate, new_status).await?;
        if n == 0 {
            return Err(ApiError::Conflict("该任务刚被并发审批或状态已变化，请刷新后重试".into()));
        }
        self.record_approval(&task.id, &gate, if decision == "rejected" { "rejected" } else { "approved" }, note).await;
        if let Some(phase) = rework_phase {
            // 打回闭环：意见入库 + 带新上下文重跑本阶段。
            // spawn 用内存行装配 prompt——必须改到 task.context 再 spawn，否则打回意见丢失。
            let feedback = note.map(str::trim).unwrap_or_default().to_string();
            let instruction = match phase {
                1 => "上一轮需求矩阵经评审打回。逐条处理打回意见后重做需求矩阵（覆盖度/可测性/无歧义）；除产物文档外禁止修改任何文件。",
                2 => "上一轮方案设计经评审打回。逐条处理打回意见后重做方案设计（对齐需求矩阵/可行性/风险验证）；除产物文档外禁止修改任何文件。",
                _ => "上一轮实施经评审打回。先修复打回意见指出的问题，再做一轮完整自检；禁止无关改动（打回意见外的文件不要碰）。",
            };
            let mut task = task;
            task.context = inject_remediation(&task.context, &feedback, instruction);
            self.task_repo.set_context(&task.id, &task.context).await?;
            self.clone().spawn_and_watch(task, phase).await;
        } else if decision != "rejected" {
            let phase = match gate.as_str() {
                "plan" => 1u8,
                "analysis" => 2,
                "solution" => 3,
                _ => 0, // diff/report 审批不触发 spawn（会话已终态）
            };
            if phase > 0 {
                self.clone().spawn_and_watch(task.clone(), phase).await;
            }
        }
        self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::Internal("任务丢失".into()))
    }

    /// 就地重试（2026-10-03 现状重审 P0）：failed/interrupted → 清残留 → 重新入队。
    /// 不走 decide（decide 白名单只认 awaiting_approval，那是审批状态机，不是重试通道）。
    /// rejected 被刻意排除：驳回=用户已裁决返工，正确路径是"复制为新任务"（理由注入+反链）。
    pub async fn retry(self: &Arc<Self>, task_id: &str) -> Result<TaskRow, ApiError> {
        let task = self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {task_id} 不存在")))?;
        if !matches!(task.status.as_str(), "failed" | "interrupted") {
            return Err(ApiError::Conflict(format!(
                "任务当前状态 {} 不可重试（仅 failed/interrupted 可就地重试；驳回返工请复制为新任务）",
                task.status
            )));
        }
        // 2026-10-05 实弹（评审#B1 同源）：失败在 p: 阶段的中途任务 retry 不得清阶段——
        // reset_for_retry 会把 gate 置 NULL，execute() 就够不着阶段感知分支，
        // manual 任务会被打回 plan 关从头评审（进度第二次清零）。
        let preserved_gate = task.gate.clone().filter(|g| g.starts_with("p:"));
        let n = self.task_repo.reset_for_retry(task_id).await?;
        if n == 0 {
            return Err(ApiError::Conflict("该任务刚被并发操作或状态已变化，请刷新后重试".into()));
        }
        if let Some(g) = preserved_gate {
            let _ = self.task_repo.set_gate(task_id, Some(&g)).await;
        }
        // 清内存基线残留（重启后 baselines 本已作废；此处防同进程内重复 retry 的脏基线）
        if let Ok(mut m) = self.baselines.lock() {
            m.remove(task_id);
        }
        self.publish_status(&task.repo, task_id, "pending", None).await;
        info!("[task-exec] 任务 {} 就地重试（{} → pending），重新入队", task_id, task.status);
        self.clone().enqueue_pending(Some(&task.repo)).await;
        self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::Internal("任务丢失".into()))
    }

    /// 修改并复审（2026-10-03 用户裁定：审查打回后要有"带意见修改→复审"闭环，
    /// 此前唯一出路是复制为新任务——上下文/血缘断裂）：
    /// rejected（子 agent 审查自动打回）→ 注入审查意见到 context.remediation →
    /// 直达实施阶段重跑（phase 3）→ 完成后子 agent 自动复审（既有流程）→
    /// 再不通过再次打回，可循环直至通过或用户改走复制新任务。
    pub async fn remediate(self: &Arc<Self>, task_id: &str) -> Result<TaskRow, ApiError> {
        use easyvibe_db::ApprovalRepository as _;
        let task = self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {task_id} 不存在")))?;
        if task.status != "rejected" {
            return Err(ApiError::Conflict(format!(
                "任务当前状态 {} 不可修改复审（仅子 agent 审查打回的 rejected 任务可原地修改复审；用户打回请复制为新任务）",
                task.status
            )));
        }
        // 审查意见 = 最近一条 rejected 留痕（diff 关自动打回必留）
        let note = self
            .approval_repo
            .list_by_task(task_id)
            .await
            .ok()
            .map(|aps| {
                aps.into_iter()
                    .filter(|a| a.decision == "rejected" && a.note.as_deref().is_some_and(|n| !n.trim().is_empty()))
                    .last()
                    .and_then(|a| a.note)
            })
            .flatten()
            .unwrap_or_else(|| "（无审查意见留痕——按 error 字段修复后重新提交）".into());
        // 注入 remediation 反馈（保留既有 context 键）
        let ctx = inject_remediation(
            &task.context,
            note.trim(),
            "上一轮实施经子 agent 审查未通过。先修复反馈问题，再做一轮完整自检；禁止无关改动（审查意见外的文件不要碰）。",
        );
        let round = serde_json::from_str::<serde_json::Value>(&ctx).unwrap_or_default()["remediation"]["round"].as_u64().unwrap_or(1);
        let n = self.task_repo.reset_for_remediate(task_id).await?;
        if n == 0 {
            return Err(ApiError::Conflict("该任务刚被并发操作或状态已变化，请刷新后重试".into()));
        }
        self.task_repo.set_context(task_id, &ctx).await?;
        self.publish_status(&task.repo, task_id, "running", Some("p:implement")).await;
        info!("[task-exec] 任务 {} 进入修改并复审（第 {} 轮）: {}", task_id, round, note.trim());
        // 直达实施阶段（复用分阶段 prompt 装配；矩阵/方案已在 context 与 dev-docs 中）。
        // 异步 spawn——实施是几十分钟级长跑，HTTP 请求必须立即返回
        let this = self.clone();
        let tid = task_id.to_string();
        tokio::spawn(async move {
            match this.task_repo.get(&tid).await {
                Ok(Some(fresh)) => this.spawn_and_watch(fresh, 3).await,
                _ => warn!("[task-exec] 复审任务 {} 丢失，spawn 取消", tid),
            }
        });
        self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::Internal("任务丢失".into()))
    }

    /// 管道回看·节点重开（2026-10-05 方案 §3.1，子agent评审#B1/B2 修订版）：
    /// 把任务放回目标评审关（analysis/solution），后续动作完全复用 decide——
    /// 「通过」照常推进，「打回」走 3187d84 的带意见重跑闭环。
    /// 可 rewound 的源状态覆盖「想回头」的全部真实场景（评审#B1——kill 会话会把任务判
    /// failed，若 failed 不可 rewind 则「停下改方案」是死路）：
    ///   awaiting_approval（走到后面的关）/ failed / interrupted / rejected（审查打回想改上游）/ done（归档返工）
    /// 拒绝：running（先终止——kill→failed 后本方法接）、pending（写互斥瞬态 5s 自愈）、
    ///       auto 信任（从未有过评审关，rewind 会把它丢进未设计的人工流——评审#S2）。
    pub async fn rewind(self: &Arc<Self>, task_id: &str, target: &str) -> Result<TaskRow, ApiError> {
        // 目标关白名单（rewind 只放回到评审关；ORDER 含 diff/report 仅供源关比较——评审#S6）
        if !matches!(target, "analysis" | "solution") {
            return Err(ApiError::BadRequest(format!("rewind 目标关仅支持 analysis/solution，收到 {target}")));
        }
        const ORDER: &[(&str, u8)] = &[("analysis", 0), ("solution", 1), ("diff", 2), ("report", 3)];
        let target_order = ORDER.iter().find(|(g, _)| *g == target).map(|(_, o)| *o).unwrap_or(0);
        let task = self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {task_id} 不存在")))?;
        if task.trust == "auto" {
            return Err(ApiError::Conflict("自动信任任务从未经过评审关，不支持回退——如需返工请复制为新任务".into()));
        }
        match task.status.as_str() {
            "running" => {
                return Err(ApiError::Conflict("任务正在执行——请先在管理按钮终止会话，再回退（终止后任务为失败态，本操作可接）".into()));
            }
            "pending" => {
                return Err(ApiError::Conflict("任务正在排队等写互斥（瞬态，数秒后自愈）——请稍候刷新再试".into()));
            }
            "awaiting_approval" => {
                // 只允许往更早的关回退；同关/更前 = 你已经在那里
                let cur = task.gate.as_deref().and_then(|g| ORDER.iter().find(|(k, _)| *k == g).map(|(_, o)| *o));
                if let Some(o) = cur {
                    if o <= target_order {
                        return Err(ApiError::Conflict(format!("任务已在 {:?} 关或更前，无需回退", task.gate)));
                    }
                }
            }
            // failed / interrupted / rejected / done：允许——这正是「停下来改上游」的入口
            _ => {}
        }
        let n = self.task_repo.try_rewind(task_id, target).await?;
        if n == 0 {
            return Err(ApiError::Conflict("该任务刚被并发操作或状态已变化，请刷新后重试".into()));
        }
        let note = format!(
            "自 {} 回退到本关——可打回（带意见让 agent 重跑本阶段）或通过继续流水线",
            task.gate.as_deref().unwrap_or("（无关卡）")
        );
        self.record_approval(task_id, target, "rewind", Some(&note)).await;
        self.publish_status(&task.repo, task_id, "awaiting_approval", Some(target)).await;
        info!("[task-exec] 任务 {} 回退到 {} 关（源状态 {} 源关 {:?}）", task_id, target, task.status, task.gate);
        self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::Internal("任务丢失".into()))
    }

    /// 人工触发子 agent 复审（2026-10-05 用户裁定：代码审查节点 = 审查-修复闭环，
    /// 不是一键通过）：在 Diff（代码审查）关对当前工作区改动再审一轮。
    /// 与实施完成后的自动审查同一机制（run_subagent_review + review 槽配置）；
    /// fail 不自动打回——人工触发的复审是「人想再看一眼」，裁决仍由人做
    /// （未通过 → 前端 prominent「修改并复审」→ remediate 修复后自动再审，循环闭合）。
    /// 异步执行（审查是分钟级）——HTTP 立即返回，结论经 result.review + 留痕 + 事件送达。
    pub async fn review_now(self: &Arc<Self>, task_id: &str) -> Result<TaskRow, ApiError> {
        use easyvibe_db::TaskRepository as _;
        let task = self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {task_id} 不存在")))?;
        if task.status != "awaiting_approval" || task.gate.as_deref() != Some("diff") {
            return Err(ApiError::Conflict(format!(
                "仅「代码审查（Diff）」关可发起复审（当前 {} / {:?}）——实施完成后系统已自动审查一轮",
                task.status, task.gate
            )));
        }
        let repo = self
            .map_service
            .find_repo(&task.repo)
            .await
            .ok_or_else(|| ApiError::NotFound(format!("仓库 {} 未注册", task.repo)))?;
        // review 槽整体替换（可换便宜模型/收紧权限）——与自动审查同款解析链
        let resolved = easyvibe_ai_agent::agent_conf::resolve_agent(&self.settings_repo, Some("review"), &self.agent_command, &self.agent_args).await;
        info!("[task-exec] 任务 {} 人工触发子 agent 复审（会话即将启动）", task_id);
        let this = self.clone();
        let tid = task_id.to_string();
        tokio::spawn(async move {
            let (custom, rule_dev, rule_fix) = {
                let h = this.harness.read().await;
                (h.custom.clone(), h.rule_development.clone(), h.rule_bugfix.clone())
            };
            let verdict = run_subagent_review(
                &this.session_manager,
                &resolved.command,
                &resolved.args,
                &repo.id,
                &repo.root,
                &task,
                &custom,
                (&rule_dev, &rule_fix),
            )
            .await;
            let note = match &verdict {
                Some(v) => {
                    merge_review_verdict(&this.task_repo, &tid, v).await;
                    format!("人工复审：{} — {}", if v.verdict == "pass" { "通过" } else { "未通过" }, v.summary)
                }
                None => "人工复审：审查会话不可用（失败/超时/结论非法）——不阻断，可重试或人工审 Diff".to_string(),
            };
            this.record_approval(&tid, "diff", "flagged", Some(&note)).await;
            // 同状态广播——前端据 task 事件 reload，result.review 新结论（at 时间戳）随之到达
            this.publish_status(&task.repo, &tid, "awaiting_approval", Some("diff")).await;
            info!("[task-exec] 任务 {} 人工复审结束：{}", tid, note);
        });
        self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::Internal("任务丢失".into()))
    }

    /// 任务状态广播统一出口（执行引擎侧）——前端列表/徽标/注意力条的事件源。
    /// P0 教训（ui-test-2026-10-03）：只写库不广播 = 任务在前端"凭空消失"。
    async fn publish_status(&self, repo: &str, task_id: &str, status: &str, gate: Option<&str>) {
        if let Some(tx) = &self.events {
            easyvibe_event_bus::publish(tx, easyvibe_event_bus::BusEvent::TaskStatus {
                repo: repo.into(),
                task_id: task_id.into(),
                status: status.into(),
                gate: gate.map(Into::into),
            });
        }
    }

    async fn record_approval(&self, task_id: &str, gate: &str, decision: &str, note: Option<&str>) {        use easyvibe_db::ApprovalRepository as _;
        let id = format!("ap-{}-{}-{}", task_id, gate, std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
        let _ = self
            .approval_repo
            .record(&easyvibe_db::ApprovalRow {
                id,
                task_id: task_id.to_string(),
                gate: gate.into(),
                decision: decision.into(),
                note: note.map(Into::into),
                decided_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string(),
            })
            .await;
    }

    /// spawn agent 并看门：终态后 manual 任务回到审批流（diff 关），auto 直接 done
    /// spawn agent 并看门。phase：0=单次直通（auto/supervised，一口气跑完）；
    /// 1=需求矩阵 2=方案设计 3=实施（manual 分阶段——需求矩阵/方案设计在实施前
    /// 必须经用户评审，rule_development 2.1/2.2 硬规定，2026-10-03 用户裁定）。
    /// 阶段 1/2 成功后停 analysis/solution 关等评审（不采集 diff、不走审查 agent）；
    /// 阶段 0/3 走完整实施链（采集 → 审查 agent → diff 关）。
    async fn spawn_and_watch(self: Arc<Self>, task: TaskRow, phase: u8) {
        // 盲测 P0：执行成功后需要人工把关的任务 = 非 auto（manual 全前置审批；supervised 执行后停 diff/report 关）
        let review_after = task.trust != "auto";
        let repo = match self.map_service.find_repo(&task.repo).await {
            Some(r) => r,
            None => {
                let _ = self.task_repo.update_status(&task.id, "failed", Some("仓库未注册")).await;
                // 状态迁移必须广播（与 publish_status 惯例配对）——否则前端列表停在旧快照
                self.publish_status(&task.repo, &task.id, "failed", None).await;
                return;
            }
        };
        let Ok(permit) = self.permits.clone().try_acquire_owned() else {
            warn!("[task-exec] {} 并发已满（4），稍后重试", task.id);
            // N25 幽灵防线：execute 已把状态置 running，此处必须退回 pending——
            // retry 循环只扫 pending，留在 running 的任务永远捞不回（假活至重启）
            // 2026-10-06 实弹：只写库不广播，前端停在"执行中"快照——排队态必须同步给界面
            let _ = self.task_repo.update_status(&task.id, "pending", None).await;
            self.publish_status(&task.repo, &task.id, "pending", task.gate.as_deref()).await;
            let this = self.clone();
            let repo = task.repo.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                this.enqueue_pending(Some(&repo)).await;
            });
            return;
        };
        // 变更归因：记录任务启动时的 HEAD（脏工作区 diff 不再混历史改动）
        let base_head = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&repo.root)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        if let Some(head) = &base_head {
            let _ = self.task_repo.set_base_head(&task.id, head).await;
        }
        // 影响面合约·基线对账：启动时的脏文件快照（采集时扣减——任务前的陈年脏文件不算越界）
        {
            let baseline = dirty_files(&repo.root).await;
            if let Ok(mut m) = self.baselines.lock() {
                m.insert(task.id.clone(), baseline);
            }
        }
        // L2 哨兵原料：合约边界 + 基线（过程巡检用；终态采集另有权威计算）
        let contract = contract_patterns_from_context(&task.context);
        // 阶段化 prompt：1/2 只产出文档（禁改代码），3 才是完整实施 prompt
        // custom：自定义层补充（注入点 #1/#2——阶段 prompt 注 global+development，实施注 global）
        let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
        let custom = self.harness.read().await.custom.clone();
        let prompt = match phase {
            1 => assemble_phase1_prompt(&task, &user, &custom),
            2 => assemble_phase2_prompt(&task, &user, &custom),
            _ => {
                let h = self.harness.read().await;
                assemble_task_prompt(&h, &task)
            }
        };
        // M1 配置体系：spawn 现读 settings 优先解析链（env 为 fallback）——
        // 配置改动对下一次 spawn 生效，不缓存快照（与 resolve_llm 同一纪律）
        let resolved = easyvibe_ai_agent::agent_conf::resolve_agent(&self.settings_repo, Some("task"), &self.agent_command, &self.agent_args).await;
        match self
            .session_manager
            // N26：任务槽超时常态 90 分钟（自由 coding 40-60 分钟是常态，30 分钟一刀切会误杀）
            .start_induction(&repo.id, &repo.root, &prompt, &resolved.command, &resolved.args, Some(task_session_timeout()))
            .await
        {
            Ok(session) => {
                let session_id = session.session_id.clone();
                let _ = self.task_repo.set_session(&task.id, &session_id).await;
                // L1 归因：任务会话归属影响模块（多模块任务取首个；会话行此刻已由 Cli 元事件落库）
                let first_module = serde_json::from_str::<Vec<String>>(&task.modules)
                    .ok()
                    .and_then(|ms| ms.into_iter().next());
                if let (Some(repo), Some(m)) = (&self.agent_session_repo, first_module) {
                    if let Err(err) = repo.set_module_id(&session_id, &m).await {
                        tracing::warn!("[agent_sessions] 任务归因失败 {session_id}: {err}");
                    }
                }
                // I1：任务槽位占仓库活动会话，气泡标签「任务执行」不得缺（否则降级显示会话 id）
                self.session_manager.note_label(&session_id, "任务执行".into()).await;
                info!("[task-exec] 任务 {} 会话 {} 已启动（trust={}）", task.id, session_id, task.trust);
                // 终端直播命脉：set_session 后立刻广播——前端此刻才拿得到 sessionId，
                // 按它过滤 session.output；不发事件 = 前端列表停在旧快照（sessionId=null），
                // 终端永远收不到行（2026-10-03 实弹 bug：执行中终端一直"等待 agent 输出"）
                if let Some(tx) = &self.events {
                    easyvibe_event_bus::publish(tx, easyvibe_event_bus::BusEvent::TaskStatus {
                        repo: task.repo.clone(),
                        task_id: task.id.clone(),
                        status: "running".into(),
                        gate: None,
                    });
                }
                let this = self.clone();
                let task_id = task.id.clone();
                let repo_name = task.repo.clone();
                let repo_root = repo.root.clone();
                let contract = contract.clone();
                let task_for_review = task.clone(); // B案审查 prompt 需要任务书字段
                tokio::spawn(async move {
                    let _permit = permit; // 许可随看门任务生命周期，并发上限真实生效（审查 🔴4）
                    let review_after = review_after;
                    let phase = phase;
                    // B 案：审查槽（M1 起走完整解析链——review 槽参数整体替换，可换便宜模型/收紧权限）
                    let review_resolved =
                        easyvibe_ai_agent::agent_conf::resolve_agent(&this.settings_repo, Some("review"), &this.agent_command, &this.agent_args).await;
                    // L2 哨兵状态：已上报越界集合 + 巡检节拍器
                    let mut reported: std::collections::HashSet<String> = std::collections::HashSet::new();
                    let sentry_every = std::cmp::max(1, sentry_interval().as_secs() / 2) as u32;
                    let mut tick: u32 = 0;
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        tick += 1;
                        // L2 过程预警：周期巡检基线后的新增变更，越界即推 WS（哨兵预警 ≠ 终态红线判定）
                        if !contract.is_empty() && tick % sentry_every == 0 {
                            let baseline = this.baselines.lock().ok().and_then(|m| m.get(&task_id).cloned()).unwrap_or_default();
                            let base = base_head_from_task(&this.task_repo, &task_id).await;
                            let candidates: Vec<String> = changed_files(&repo_root, base.as_deref()).await
                                .into_iter()
                                .filter(|p| !baseline.contains(p))
                                .filter(|p| !p.starts_with(".easyvibe/") && !p.starts_with(".claude/"))
                                .filter(|p| !path_within_contract(p, &contract))
                                .collect();
                            let fresh = new_violators(&candidates, &mut reported);
                            if !fresh.is_empty() {
                                warn!("[task-exec] 任务 {} 过程越界预警：{}", task_id, fresh.join("、"));
                                if let Some(tx) = &this.events {
                                    easyvibe_event_bus::publish(tx, easyvibe_event_bus::BusEvent::TaskContractAlert {
                                        repo: repo_name.clone(),
                                        task_id: task_id.clone(),
                                        files: fresh,
                                    });
                                }
                            }
                        }
                        match this.session_manager.status_of_session(&session_id).await {
                            Some(s)
                                if !matches!(
                                    s.status,
                                    easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running
                                ) =>
                            {
                                let failed = s.status == easyvibe_api_types::SessionStatus::Failed;
                                // M4-1：执行成功 → 产物采集（RESULT 行 + git 摘要 + development_docs 归档），
                                // 供 diff 关审批展示；采集失败不阻断终态回写。
                                // 阶段 1/2 只产文档不改代码——跳过采集（diff 语义不适用）。
                                // 采集适用于单次直通（phase 0）与实施阶段（phase 3）。
                                if !failed && (phase == 0 || phase == 3) {
                                    let baseline = this.baselines.lock().ok().and_then(|mut m| m.remove(&task_id)).unwrap_or_default();
                                    if let Some(json) =
                                        collect_task_result(&this.session_manager, &session_id, &repo_root, &task_id, &this.task_repo, &baseline).await
                                    {
                                        // R2 裂缝#3：auto/supervised 无审批关——越界经 WS 事件主动送达（toast+系统通知）
                                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                                            let files: Vec<String> = v["contractViolations"].as_array().map(|a| {
                                                a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()
                                            }).unwrap_or_default();
                                            if !files.is_empty() {
                                                if let Some(tx) = &this.events {
                                                    // broadcast::Sender::send 是同步方法
                                                    easyvibe_event_bus::publish(tx, easyvibe_event_bus::BusEvent::TaskContractViolated {
                                                        repo: repo_name.clone(),
                                                        task_id: task_id.clone(),
                                                        files,
                                                    });
                                                }
                                            }
                                        }
                                        let _ = this.task_repo.set_result(&task_id, &json).await;
                                        info!("[task-exec] 任务 {} 产物已归档（diff 关供料）", task_id);
                                    }
                                }
                                let (status, gate) = if failed {
                                    ("failed", None)
                                } else if phase == 1 || phase == 2 {
                                    // 阶段产物初审（2026-10-03 用户裁定）：需求矩阵/方案设计到人工关之前，
                                    // 先派子 agent 预筛（fail 不自动打回——人是最终裁决，初审只是预筛）。
                                    // 结论进 result.phaseReviews，评审卡横幅展示；初审不可用不阻断。
                                    let key = if phase == 1 { "analysis" } else { "solution" };
                                    match run_phase_doc_review(
                                        &this.session_manager,
                                        &review_resolved.command,
                                        &review_resolved.args,
                                        &repo_name,
                                        &repo_root,
                                        &task_for_review,
                                        phase,
                                        &this.harness.read().await.custom,
                                    )
                                    .await
                                    {
                                        Some(v) => {
                                            if v.verdict == "fail" {
                                                warn!("[task-exec] 任务 {} {}初审未通过：{}", task_id, key, v.summary);
                                            }
                                            merge_phase_review(&this.task_repo, &task_id, key, &v).await;
                                        }
                                        None => info!("[task-exec] 任务 {} {}初审不可用，人工关照常", task_id, key),
                                    }
                                    ("awaiting_approval", Some(if phase == 1 { "analysis" } else { "solution" }))
                                } else if review_after {
                                    // manual/supervised：执行成功 → 独立子agent审查（B案，harness 2.3.2）
                                    // → 通过才回审批流（diff 关）；fail 自动打回（rejected，理由入留痕）
                                    let (custom, rule_dev, rule_fix) = {
                                        let h = this.harness.read().await;
                                        (h.custom.clone(), h.rule_development.clone(), h.rule_bugfix.clone())
                                    };
                                    let review_v = run_subagent_review(
                                        &this.session_manager,
                                        &review_resolved.command,
                                        &review_resolved.args,
                                        &repo_name,
                                        &repo_root,
                                        &task_for_review,
                                        &custom,
                                        (&rule_dev, &rule_fix),
                                    )
                                    .await;
                                    match review_v
                                    {Some(v) if v.verdict == "fail" => {
                                            let note = format!("子agent审查未通过：{}", v.summary);
                                            warn!("[task-exec] 任务 {} 审查打回：{}", task_id, v.summary);
                                            this.record_approval(&task_id, "diff", "rejected", Some(&note)).await;
                                            let _ = this.task_repo.update_status(&task_id, "rejected", Some(&note)).await;
                                            let _ = this.task_repo.set_gate(&task_id, Some("rejected")).await;
                                            if let Some(tx) = &this.events {
                                                easyvibe_event_bus::publish(tx, easyvibe_event_bus::BusEvent::TaskStatus {
                                                    repo: repo_name.clone(),
                                                    task_id: task_id.clone(),
                                                    status: "rejected".into(),
                                                    gate: Some("rejected".into()),
                                                });
                                            }
                                            ("__already_final__", None) // 状态已回写，下面的统一回写跳过
                                        }
                                        Some(v) => {
                                            // 审查通过：结论并入 result（④格"子agent初审"卡的数据源），照常进 diff 关
                                            merge_review_verdict(&this.task_repo, &task_id, &v).await;
                                            ("awaiting_approval", Some("diff"))
                                        }
                                        None => {
                                            // 审查不可用（会话失败/超时/结论非法）：不阻断，diff 关照常（方案：人机审查兜底）
                                            info!("[task-exec] 任务 {} 审查不可用，照常进 diff 关", task_id);
                                            ("awaiting_approval", Some("diff"))
                                        }
                                    }
                                } else {
                                    ("done", Some("done"))
                                };
                                if status != "__already_final__" {
                                    let _ = this.task_repo.update_status(&task_id, status, None).await;
                                    if let Some(g) = gate {
                                        let _ = this.task_repo.set_gate(&task_id, Some(g)).await;
                                    }
                                    // 存量缺口补发：任务执行终态此前只写库不发事件（前端靠轮询才发现）——
                                    // 与 decide_task 对齐，终态即广播 task.statusChanged
                                    if let Some(tx) = &this.events {
                                        easyvibe_event_bus::publish(tx, easyvibe_event_bus::BusEvent::TaskStatus {
                                            repo: repo_name.clone(),
                                            task_id: task_id.clone(),
                                            status: status.into(),
                                            gate: gate.map(Into::into),
                                        });
                                    }
                                }
                                info!("[task-exec] 任务 {} 终态 {:?} → {}", task_id, s.status, status);
                                break;
                            }
                            // None：会话状态被清理等异常——按失败收尸，防幽灵 running
                            None => {
                                let _ = this.task_repo.update_status(&task_id, "failed", Some("会话状态丢失")).await;
                                this.publish_status(&repo_name, &task_id, "failed", None).await;
                                break;
                            }
                            _ => {}
                        }
                    }
                });
            }
            Err(ApiError::Conflict(_)) => {
                // R3 P0-1：execute 已把状态置 running，写互斥（归纳/巡检进行中）是临时态——
                // 必须退回 pending，否则 retry 循环只扫 pending，任务假活 running 至重启
                // （N25 幽灵在 permits 满路径防过、此处漏掉；348-358 曾留有同款意图的死代码）
                warn!("[task-exec] 任务 {} 遇到写互斥，退回 pending 排队延迟重试", task.id);
                let _ = self.task_repo.update_status(&task.id, "pending", None).await;
                self.publish_status(&task.repo, &task.id, "pending", task.gate.as_deref()).await;
                let this = self.clone();
                let repo = task.repo.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    this.enqueue_pending(Some(&repo)).await;
                });
            }
            Err(e) => {
                warn!("[task-exec] 任务 {} spawn 失败: {e}", task.id);
                let _ = self.task_repo.update_status(&task.id, "failed", Some(&e.to_string())).await;
                self.publish_status(&task.repo, &task.id, "failed", None).await;
            }
        }
    }
}

#[cfg(test)] mod test_util;
#[cfg(test)] mod tests_changes;
#[cfg(test)] mod tests_contract;
#[cfg(test)] mod tests_flow;
#[cfg(test)] mod tests_gates;
#[cfg(test)] mod tests_harness;
#[cfg(test)] mod tests_prompt;
