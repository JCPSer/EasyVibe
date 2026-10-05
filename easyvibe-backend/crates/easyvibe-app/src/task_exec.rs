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
                self.publish_status(&task.repo, &task.id, "running", task.gate.as_deref()).await;
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
            let verdict = run_subagent_review(
                &this.session_manager,
                &resolved.command,
                &resolved.args,
                &repo.id,
                &repo.root,
                &task,
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
                return;
            }
        };
        let Ok(permit) = self.permits.clone().try_acquire_owned() else {
            warn!("[task-exec] {} 并发已满（4），稍后重试", task.id);
            // N25 幽灵防线：execute 已把状态置 running，此处必须退回 pending——
            // retry 循环只扫 pending，留在 running 的任务永远捞不回（假活至重启）
            let _ = self.task_repo.update_status(&task.id, "pending", None).await;
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
        let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
        let prompt = match phase {
            1 => assemble_phase1_prompt(&task, &user),
            2 => assemble_phase2_prompt(&task, &user),
            _ => {
                let h = self.harness.read().await;
                assemble_task_prompt(&h.framework_transparent, &task)
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
                                    let review_v = run_subagent_review(
                                        &this.session_manager,
                                        &review_resolved.command,
                                        &review_resolved.args,
                                        &repo_name,
                                        &repo_root,
                                        &task_for_review,
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
            }
        }
    }
}

/// 打回反馈注入 context.remediation（decide 打回与 remediate 复审共用）：
/// round 在既有值上累加，review_feedback/instruction 整段替换——新一轮打回覆盖上一轮。
/// 返回新的 context JSON 字符串（保留既有其他键）。
fn inject_remediation(context_json: &str, feedback: &str, instruction: &str) -> String {
    let mut ctx: serde_json::Value = serde_json::from_str(context_json).unwrap_or_else(|_| serde_json::json!({}));
    let round = ctx["remediation"]["round"].as_u64().unwrap_or(0) + 1;
    ctx["remediation"] = serde_json::json!({
        "round": round,
        "review_feedback": feedback,
        "instruction": instruction,
    });
    serde_json::to_string(&ctx).unwrap_or_else(|_| "{}".into())
}

/// 打回反馈展示段：context.remediation 存在时拼进阶段 prompt（重跑带着意见做），否则空串。
fn remediation_section(context_json: &str) -> String {
    let ctx: serde_json::Value = serde_json::from_str(context_json).unwrap_or_else(|_| serde_json::json!({}));
    match (ctx["remediation"]["round"].as_u64(), ctx["remediation"]["review_feedback"].as_str()) {
        (Some(round), Some(fb)) if !fb.is_empty() => format!(
            "\n## 打回反馈（第 {round} 轮）\n{fb}\n\n处置要求：{}\n",
            ctx["remediation"]["instruction"].as_str().unwrap_or("")
        ),
        _ => String::new(),
    }
}

/// 阶段 1 prompt：只产出需求矩阵，禁改代码（rule_development 2.1）。
/// 产出后由用户在 analysis 关评审——评审通过才进阶段 2。
fn assemble_phase1_prompt(task: &TaskRow, user: &str) -> String {
    let modules: Vec<String> = serde_json::from_str(&task.modules).unwrap_or_default();
    format!(
        r#"你是需求分析 agent（harness 规则正文 2.1 的执行者）。**只做需求分析，禁止修改任何代码文件。**

## 任务书
- 需求描述：{description}
- 影响模块：{modules}
- 验收标准：{acceptance}
{feedback}
## 要求
1. 研读仓库代码与 .easyvibe/map/map.json，将需求拆解为需求矩阵（背景、目标、描述、优先级、难度、风险）。
2. 需求矩阵写入 .easyvibe/development_docs/{user}/1_requirements_matrix/<yyyy-MM-dd-hh-mm>-<brief>.md，
   文档必须含「评审意见栏」（留空待用户填写）。
3. 更新同目录 INDEX-<yyyy-MM>.md 索引。
4. 不要实施任何代码改动——实施发生在用户评审通过之后。
5. 最后一行输出：[EASYVIBE-RESULT] {{"summary":"需求矩阵已产出：一句话概括","changed_modules":[]}}"#,
        description = task.description,
        modules = if modules.is_empty() { "（未指定，由你分析）".into() } else { modules.join(", ") },
        acceptance = if task.acceptance.is_empty() { "（未指定）".into() } else { task.acceptance.clone() },
        feedback = remediation_section(&task.context),
        user = user,
    )
}
/// 阶段 2 prompt：只产出方案设计，禁改代码（rule_development 2.2）。
/// 基于已评审通过的需求矩阵；产出后由用户在 solution 关评审——通过才进阶段 3 实施。
fn assemble_phase2_prompt(task: &TaskRow, user: &str) -> String {
    let modules: Vec<String> = serde_json::from_str(&task.modules).unwrap_or_default();
    format!(
        r#"你是方案设计 agent（harness 规则正文 2.2 的执行者）。**只做方案设计，禁止修改任何代码文件。**

## 任务书
- 需求描述：{description}
- 影响模块：{modules}
- 验收标准：{acceptance}

## 输入
- 已评审通过的需求矩阵：.easyvibe/development_docs/{user}/1_requirements_matrix/ 下最新文档（先读它）
{feedback}
## 要求
1. 针对需求矩阵中的每个子需求进行方案设计：背景、目标、描述、详细方案设计。
2. 方案用伪代码或流程图表述，**禁止大段真实代码**（保证方案可读性）。
3. 涉及 UI 变动的部分必须包含 UI 设计说明。
4. 方案写入 .easyvibe/development_docs/{user}/2_requirements_solutions/<yyyy-MM-dd-hh-mm>-<brief>.md，
   文档必须含「评审意见栏」（留空待用户填写）。
5. 更新同目录 INDEX-<yyyy-MM>.md 索引。
6. 不要实施任何代码改动——实施发生在用户评审通过之后。
7. 最后一行输出：[EASYVIBE-RESULT] {{"summary":"方案设计已产出：一句话概括","changed_modules":[]}}"#,
        description = task.description,
        modules = if modules.is_empty() { "（未指定，由你分析）".into() } else { modules.join(", ") },
        acceptance = if task.acceptance.is_empty() { "（未指定）".into() } else { task.acceptance.clone() },
        feedback = remediation_section(&task.context),
        user = user,
    )
}

/// 独立子 agent 代码审查（用户裁定「全做」B 案，harness 2.3.2 的独立可信落地）：
/// 实施完成后、diff 关之前，spawn 一个审查会话——只读 diff + 架构规则，产出审查报告与结论。
/// 报告落 `.easyvibe/development_docs/<user>/3_test_results/review-<task_id>.md`
/// （与 A 案协议同一规范路径，dev-docs 端点自动捞取）。
/// 结论行协议：`[EASYVIBE-REVIEW] {"verdict":"pass|fail","summary":"一句话"}`
/// 审查自身失败/超时 → None（不阻断：diff 关照常，人机审查兜底）。
fn assemble_review_prompt(task: &TaskRow, user: &str) -> String {
    let modules: Vec<String> = serde_json::from_str(&task.modules).unwrap_or_default();
    format!(
        r#"你是独立代码审查 agent（harness 2.3.2 的执行者），只做审查，不做实现。
被审任务的实施刚完成，工作区的未提交改动就是它的产出。

## 被审任务
- 需求描述：{description}
- 影响模块：{modules}
- 验收标准：{acceptance}

## 审查材料
- 改动全文：工作目录即仓库根目录，执行 `git diff` 查看（不要 git checkout/stash 等任何写操作）
- 规则正文：cat {harness_dir}/rule_development.md（功能开发）或 {harness_dir}/rule_bugfix.md（Bug 修复），按任务性质择一
- 模块职责与边界：.easyvibe/map/map.json

## 审查维度（逐项给出结论）
代码规范、代码结构、可读性、可维护性、性能；对照影响面合约检查越界改动；
对照验收标准检查完整性。**除写审查报告外，禁止修改任何文件。**

## 输出（两者都必须）
1. 审查报告写入 .easyvibe/development_docs/{user}/3_test_results/review-{task_id}.md
   （含明确的审查意见：通过 / 打回 + 理由；发现问题逐条列出）
2. 最后一行输出：[EASYVIBE-REVIEW] {{"verdict":"pass","summary":"一句话结论"}}
   verdict 只能是 pass 或 fail；有任一阻断性问题必须 fail。"#,
        description = task.description,
        modules = if modules.is_empty() { "（未指定）".into() } else { modules.join(", ") },
        acceptance = if task.acceptance.is_empty() { "（未指定）".into() } else { task.acceptance.clone() },
        user = user,
        task_id = task.id,
        harness_dir = harness_dir().to_string_lossy(),
    )
}

/// 审查结论（解析自 [EASYVIBE-REVIEW] 行）
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReviewVerdict {
    pub verdict: String, // pass / fail
    pub summary: String,
}

/// 跑独立审查会话并等终态。25 分钟上限（审查是分钟级任务；超时判不可用不阻断）。
async fn run_subagent_review(
    session_manager: &SessionManager,
    agent_command: &str,
    agent_args: &[String],
    repo_id: &str,
    repo_root: &std::path::Path,
    task: &TaskRow,
) -> Option<ReviewVerdict> {
    let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
    let prompt = assemble_review_prompt(task, &user);
    // 审查会话沿用任务槽 CLI 参数（可经 settings `agent.args.review` 单独收紧/换模型）
    let session = session_manager
        .start_induction(repo_id, repo_root, &prompt, agent_command, agent_args, Some(std::time::Duration::from_secs(25 * 60)))
        .await
        .ok()?;
    let sid = session.session_id.clone();
    info!("[task-exec] 任务 {} 审查会话 {} 已启动", task.id, sid);
    session_manager.note_label(&sid, "任务执行·审查".into()).await;
    await_review_verdict(session_manager, &sid, &task.id, std::time::Duration::from_secs(25 * 60)).await
}

/// 阶段产物初审（2026-10-03 用户裁定）：需求矩阵（phase 1）/方案设计（phase 2）
/// 到人工关之前，先派子 agent 预筛一遍——完整性/可测性/一致性。
/// 与实施后审查的关键差异：fail 不自动打回——人是最终裁决，初审只是给审批人
/// 多一双眼睛（结论进 result.phaseReviews，评审卡横幅展示）。
async fn run_phase_doc_review(
    session_manager: &SessionManager,
    agent_command: &str,
    agent_args: &[String],
    repo_id: &str,
    repo_root: &std::path::Path,
    task: &TaskRow,
    phase: u8,
) -> Option<ReviewVerdict> {
    let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
    let prompt = assemble_phase_review_prompt(task, phase, &user);
    // 初审是读文档+下结论，比实施审查轻——15 分钟上限足够
    let session = session_manager
        .start_induction(repo_id, repo_root, &prompt, agent_command, agent_args, Some(std::time::Duration::from_secs(15 * 60)))
        .await
        .ok()?;
    let sid = session.session_id.clone();
    info!("[task-exec] 任务 {} 阶段 {} 初审会话 {} 已启动", task.id, phase, sid);
    session_manager.note_label(&sid, "任务执行·初审".into()).await;
    await_review_verdict(session_manager, &sid, &task.id, std::time::Duration::from_secs(15 * 60)).await
}

/// 审查会话终态等待 + [EASYVIBE-REVIEW] 行解析（实施审查与阶段初审共用）。
/// 会话异常终态/超时/结论非法 → None（不可用不阻断，人机审查兜底）。
async fn await_review_verdict(
    session_manager: &SessionManager,
    session_id: &str,
    task_id: &str,
    timeout: std::time::Duration,
) -> Option<ReviewVerdict> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match session_manager.status_of_session(session_id).await {
            Some(s)
                if matches!(
                    s.status,
                    easyvibe_api_types::SessionStatus::Succeeded | easyvibe_api_types::SessionStatus::Failed
                ) =>
            {
                if s.status != easyvibe_api_types::SessionStatus::Succeeded {
                    warn!("[task-exec] 任务 {} 审查会话异常终态：{:?}", task_id, s.status);
                    return None;
                }
                let out = session_manager.take_output(session_id).await.unwrap_or_default();
                let line = out.lines().rev().find(|l| l.contains("[EASYVIBE-REVIEW]"))?;
                let json_str = line.split("[EASYVIBE-REVIEW]").nth(1)?.trim();
                let v: serde_json::Value = serde_json::from_str(json_str).ok()?;
                let verdict = v["verdict"].as_str().unwrap_or("").to_string();
                if verdict != "pass" && verdict != "fail" {
                    warn!("[task-exec] 任务 {} 审查结论 verdict 非法：{}", task_id, verdict);
                    return None;
                }
                let summary = v["summary"].as_str().unwrap_or("（无结论摘要）").to_string();
                info!("[task-exec] 任务 {} 审查结论：{} — {}", task_id, verdict, summary);
                return Some(ReviewVerdict { verdict, summary });
            }
            None => return None,
            _ => {
                if tokio::time::Instant::now() > deadline {
                    warn!("[task-exec] 任务 {} 审查会话超时，按不可用处理", task_id);
                    let _ = session_manager.kill(session_id).await;
                    return None;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
}

/// 阶段初审 prompt：只审不改（禁止修改文件），按修改时间找最新产物文档通读，
/// 对照任务书核质量，最后一行输出 [EASYVIBE-REVIEW] 结论。
fn assemble_phase_review_prompt(task: &TaskRow, phase: u8, user: &str) -> String {
    let (name, dir, focus) = if phase == 1 {
        (
            "需求矩阵",
            format!(".easyvibe/development_docs/{user}/1_requirements_matrix/"),
            "- 覆盖度：任务描述里的每个诉求都有对应需求条目吗？有没有漏项？\n\
             - 可测性：每条验收标准是否可判定（有明确的完成口径，而非'优化/提升'这类模糊词）？\n\
             - 无歧义：需求条目之间是否自洽，有没有互相矛盾或重复？",
        )
    } else {
        (
            "方案设计",
            format!(".easyvibe/development_docs/{user}/2_requirements_solutions/"),
            "- 对齐性：方案是否逐条回应了需求矩阵（R 编号可回溯）？有没有矩阵里的需求被方案漏掉？\n\
             - 可行性：改动路径与当前代码结构是否矛盾（是否引用了不存在的文件/接口）？\n\
             - 风险：方案有没有明显的回归风险未给验证手段？",
        )
    };
    format!(
        r#"你是 EasyVibe 的阶段产物审查 agent。**只审不改：禁止创建/修改/删除任何文件**。

## 任务书（审查的对照基准）
- 需求描述：{description}
- 验收标准：{acceptance}

## 待审产物
{name}，目录：{dir}（该目录下修改时间最新的 .md 文档——用 ls -t 找到它并完整读取）

## 审查要点
{focus}

## 输出
审查过程不需要长篇大论；最后一行必须严格是：
[EASYVIBE-REVIEW] {{"verdict":"pass 或 fail","summary":"一句话结论（≤80 字：通过理由或关键问题）"}}"#,
        description = task.description,
        acceptance = if task.acceptance.is_empty() { "（未指定——按需求描述推断合理验收口径）".into() } else { task.acceptance.clone() },
        name = name,
        dir = dir,
        focus = focus,
    )
}

/// 实施审查结论并入 tasks.result.review（含 at 毫秒时间戳——前端「人工复审」
/// 轮次完成判定的依据：点击发起后轮询到 at ≥ 点击时刻即知本轮已出结论）。
/// result 可能不存在（复审发生在采集前），此时新建 JSON 骨架。
async fn merge_review_verdict(task_repo: &easyvibe_db::SqliteTaskRepository, task_id: &str, v: &ReviewVerdict) {
    use easyvibe_db::TaskRepository as _;
    let Ok(Some(row)) = task_repo.get(task_id).await else { return };
    let mut rv: serde_json::Value = row
        .result
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    rv["review"] = serde_json::json!({ "verdict": v.verdict, "summary": v.summary, "at": at });
    if let Ok(s) = serde_json::to_string(&rv) {
        let _ = task_repo.set_result(task_id, &s).await;
    }
}

/// 初审结论并入 tasks.result.phaseReviews（key = analysis / solution）——
/// result 可能尚不存在（阶段 1/2 不采集产物），此时新建 JSON 骨架。
async fn merge_phase_review(
    task_repo: &easyvibe_db::SqliteTaskRepository,
    task_id: &str,
    key: &str,
    v: &ReviewVerdict,
) {
    use easyvibe_db::TaskRepository as _;
    let Ok(Some(row)) = task_repo.get(task_id).await else { return };
    let mut rv: serde_json::Value = row
        .result
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    rv["phaseReviews"][key] = serde_json::json!({ "verdict": v.verdict, "summary": v.summary });
    if let Ok(s) = serde_json::to_string(&rv) {
        let _ = task_repo.set_result(task_id, &s).await;
    }
}

/// 组装任务执行 prompt：harness 框架（路径适配）+ 表单字段 + 事前注入上下文。
/// 路由/拷问/豁免全交给 LLM 决断（§9 #3/#5）。
pub fn assemble_task_prompt(framework: &str, task: &TaskRow) -> String {
    let modules = serde_json::from_str::<Vec<String>>(&task.modules).unwrap_or_default();
    format!(
        r#"{framework}

---

## 本次任务（来自 EasyVibe 结构化任务表单）

- 需求描述：{description}
- 影响模块：{modules}
- 验收标准：{acceptance}

## 事前注入上下文（模块职责 / 边界 / 问题 / 违规边——据此把握结构与边界）

```json
{context}
```

## 已评审通过的输入（分阶段评审的产物——先读再动手，严格按方案实施）

- 需求矩阵：.easyvibe/development_docs/{user}/1_requirements_matrix/ 下最新文档
- 方案设计：.easyvibe/development_docs/{user}/2_requirements_solutions/ 下最新文档（若有）

## 执行要求

- 工作目录即仓库根目录；框架与规则正文中的路径已适配到本机，直接 cat 读取。
- **严格按已评审通过的方案实施**，不要偏离方案另作设计；发现方案有硬伤时停下来在 [EASYVIBE-RESULT] 的 summary 中说明。
- 改动规模评估与是否走完整评审流程由你决断（框架内的豁免条款），全程留痕。
- 统计/验证类结论用工具数准，禁止估算。
- 实施完成后对接口/界面进行测试，测试结果写入 .easyvibe/development_docs/{user}/3_test_results/（规范路径，含 INDEX 索引）——对应规则正文 2.3.1。
- 代码审查（规则正文 2.3.2）由系统独立审查 agent 执行，你无需自审；不要伪造审查结论。
- 完成后最后一行输出：`[EASYVIBE-RESULT] {{"summary": "一句话总结", "changed_modules": ["模块id"]}}` 便于系统归档。"#,
        framework = framework,
        description = task.description,
        modules = if modules.is_empty() { "（未指定，由你分析）".into() } else { modules.join(", ") },
        acceptance = if task.acceptance.is_empty() { "（未指定）".into() } else { task.acceptance.clone() },
        context = task.context,
        user = std::env::var("USER").unwrap_or_else(|_| "default".into()),
    )
}

/// 出厂 harness 只读底账：编译期内嵌（§12c 决策——reference/ 只是构建源，运行时唯一
/// 生效副本是数据目录的用户可编辑层；"改哪份才生效"的二义就此消灭）
/// 注意：内嵌的仍是 reference/ 原稿（含 .claude 路径）——换姓发生在写盘时
/// （adapt_builtin_content），原稿保持用户参考材料原样不动
pub const BUILTIN_HARNESS: &[(&str, &str)] = &[
    ("manifest.json", include_str!("../../../../reference/manifest.json")),
    ("inject-prompt.md", include_str!("../../../../reference/inject-prompt.md")),
    ("rule_development.md", include_str!("../../../../reference/rule_development.md")),
    ("rule_bugfix.md", include_str!("../../../../reference/rule_bugfix.md")),
    ("skills/grill-me/SKILL.md", include_str!("../../../../reference/grill-me/SKILL.md")),
];

/// harness 产物路径换姓（方案 v3 §4.4）：三种 .claude 形态 → .easyvibe 自有路径。
/// 顺序敏感：先长后短，`<user_name>` 兜底最后；`~/.claude/hooks/` 是受保护的
/// load 时替换锚点（task_exec.rs load_harness_from），换姓前先占位保护、最后还原。
/// 1) `.claude/<user_name>/development_docs` → `.easyvibe/development_docs/<user>`
///    （inject-prompt.md 的 task.json 路径，形态与 2 不同，漏换则 task.json 写回旧位置）
/// 2) `.claude/development_docs` → `.easyvibe/development_docs`（规则正文 20+ 处）
/// 3) 裸 `.claude/` → `.easyvibe/`（rule_development.md 附录目录树根行，复审残留）
/// 4) `<user_name>` → 本机用户名（剩余形态兜底，取不到用 default）
pub fn adapt_builtin_content(content: &str) -> String {
    let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
    const HOOKS_ANCHOR: &str = "\u{1}HOOKS\u{1}";
    content
        .replace("~/.claude/hooks/", HOOKS_ANCHOR)
        .replace(".claude/<user_name>/development_docs", &format!(".easyvibe/development_docs/{user}"))
        .replace(".claude/development_docs", ".easyvibe/development_docs")
        .replace(".claude/", ".easyvibe/")
        .replace("<user_name>", &user)
        .replace(HOOKS_ANCHOR, "~/.claude/hooks/")
}

/// 版本号比较（a > b）。"1.2.0" vs "1.10.0" 按段数值比较，解析失败段按 0。
fn version_gt(a: &str, b: &str) -> bool {
    let segs = |s: &str| s.split('.').map(|x| x.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (sa, sb) = (segs(a), segs(b));
    for i in 0..sa.len().max(sb.len()) {
        let (x, y) = (sa.get(i).copied().unwrap_or(0), sb.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

const TRANSPARENT_MODE_LINE: &str = "（透明执行模式：禁止向用户提问或要求确认；需求有歧义时按最合理假设直接执行，并在 [EASYVIBE-RESULT] 的 summary 中说明你做出的假设。）";

/// Harness manifest（§12c 边界定稿：控制面声明，装配层唯一需要解析的文件）
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessManifest {
    pub id: String,
    pub version: String,
    #[serde(default)] pub builtin: bool,
    #[serde(default)] pub route_rules: Vec<String>,
    #[serde(default)] pub skills: HarnessSkills,
    /// 透明装配时中和的指令模式（实弹#3 防线的配置化——从硬编码 grep 升级为 manifest 声明）
    #[serde(default)] pub transparent_neutralize: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSkills {
    /// user_entry 插槽：仅用户入口对话注入（§9 #4）——grill-me 挂在这里
    #[serde(default)] pub user_entry: Vec<String>,
    /// transparent 插槽：透明 agent（任务/归纳/巡检）——缺省空，不是文本删除
    #[serde(default)] pub transparent: Vec<String>,
}

/// 装载完成的 harness：三种装配产物同源不同形（同一个 manifest，两种装配产物）
#[derive(Clone)]
pub struct Harness {
    pub dir: PathBuf,
    pub manifest: HarnessManifest,
    /// 透明执行装配框架：路径换姓 + 按 manifest 中和拷问类指令（供任务 prompt）
    pub framework_transparent: String,
    /// user_entry 插槽 skill 正文（供对话 prompt；透明 agent 永不注入）
    pub user_entry_skills: Vec<String>,
}

pub fn harness_dir() -> PathBuf {
    match std::env::var("EASYVIBE_HARNESS_DIR") {
        Ok(v) => PathBuf::from(v),
        Err(_) => {
            // Windows 无 HOME——USERPROFILE 兜底，都缺时落当前目录（独立 exe 场景 = exe 旁）
            let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())
                .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()))
                .unwrap_or_else(|| ".".into());
            PathBuf::from(format!("{home}/.easyvibe/harness"))
        }
    }
}

/// 出厂底账部署：缺失文件从内嵌底账补齐（写盘时路径换姓）；**不覆盖**用户已编辑的文件。
/// 版本迁移：内置 manifest 版本高于磁盘 → 三份规则正文仅在"磁盘内容仍等于出厂原稿"
/// （即用户未改动）时重写为换姓版；manifest 只抬版本号、保留用户其余字段。
/// 恢复默认（reset）走 deploy_builtin_force。
pub fn deploy_builtin(dir: &std::path::Path) -> Result<(), ApiError> {
    // 版本迁移判定（manifest 版本比较）
    let disk_ver = std::fs::read_to_string(dir.join("manifest.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v["version"].as_str().map(str::to_string));
    let builtin_ver = BUILTIN_HARNESS
        .iter()
        .find(|(rel, _)| *rel == "manifest.json")
        .and_then(|(_, c)| serde_json::from_str::<serde_json::Value>(c).ok())
        .and_then(|v| v["version"].as_str().map(str::to_string));
    let migrate = match (&builtin_ver, &disk_ver) {
        (Some(b), Some(d)) => version_gt(b, d),
        // 磁盘无 manifest（首次部署）或版本不可解析：不触发迁移，走缺失补齐
        _ => false,
    };

    for (rel, content) in BUILTIN_HARNESS {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ApiError::Internal(format!("harness 目录创建失败: {e}")))?;
        }
        if !p.exists() {
            std::fs::write(&p, adapt_builtin_content(content))
                .map_err(|e| ApiError::Internal(format!("harness 底账写入失败 {}: {e}", p.display())))?;
            continue;
        }
        if !migrate {
            continue;
        }
        if *rel == "manifest.json" {
            // 只抬版本号：磁盘 manifest 的其余字段（用户可能加过 routeRules）保留
            if let Ok(mut v) = serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&p).unwrap_or_default()) {
                if let (Some(obj), Some(bv)) = (v.as_object_mut(), &builtin_ver) {
                    obj.insert("version".into(), serde_json::Value::String(bv.clone()));
                    if let Ok(s) = serde_json::to_string_pretty(&v) {
                        let _ = std::fs::write(&p, s);
                    }
                }
            }
            continue;
        }
        // 规则正文：磁盘仍等于出厂原稿 = 未改动 → 重写为换姓版；已改动则保留用户版
        let disk = std::fs::read_to_string(&p).unwrap_or_default();
        if disk == *content {
            std::fs::write(&p, adapt_builtin_content(content))
                .map_err(|e| ApiError::Internal(format!("harness 换姓重写失败 {}: {e}", p.display())))?;
        }
    }
    Ok(())
}

/// 恢复默认：全量覆盖用户层（与 deploy_builtin 的"缺失才补"语义相反）；同样写盘时换姓
pub fn deploy_builtin_force(dir: &std::path::Path) -> Result<(), ApiError> {
    for (rel, content) in BUILTIN_HARNESS {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ApiError::Internal(format!("harness 目录创建失败: {e}")))?;
        }
        std::fs::write(&p, adapt_builtin_content(content))
            .map_err(|e| ApiError::Internal(format!("harness 底账写入失败 {}: {e}", p.display())))?;
    }
    Ok(())
}

/// harness 装载（§12c 插槽内核）：
/// 1) 底账补齐（缺失才写）→ 2) 解析 manifest → 3) 透明框架=路径换姓+按 manifest 中和
/// 4) user_entry 插槽正文读取。装配层唯一解析 manifest，正文如何演化与防线解耦
/// （实弹#3 教训：grep 硬编码与框架文本演化会漂移）。
pub fn load_harness() -> Result<Harness, ApiError> {
    let dir = harness_dir();
    load_harness_from(&dir)
}

/// 从指定目录装载（测试注入点——避免 env 变量在并行测试间的竞态）
pub fn load_harness_from(dir: &std::path::Path) -> Result<Harness, ApiError> {
    deploy_builtin(dir)?;
    let manifest: HarnessManifest = serde_json::from_str(
        &std::fs::read_to_string(dir.join("manifest.json"))
            .map_err(|e| ApiError::Internal(format!("harness manifest 不可读: {e}")))?,
    )
    .map_err(|e| ApiError::Internal(format!("harness manifest 解析失败: {e}")))?;
    let framework = std::fs::read_to_string(dir.join("inject-prompt.md"))
        .map_err(|e| ApiError::Internal(format!("harness 框架不可读: {e}")))?;
    // 路径换姓：框架内引用的规则正文位置指向本机 harness 目录
    let adapted = framework.replace("~/.claude/hooks/", &format!("{}/", dir.to_string_lossy().trim_end_matches('/')));
    let framework_transparent = neutralize_transparent(&adapted, &manifest.transparent_neutralize);
    let mut user_entry_skills = vec![];
    for rel in &manifest.skills.user_entry {
        let p = dir.join(rel);
        let content = std::fs::read_to_string(&p)
            .map_err(|e| ApiError::Internal(format!("user_entry 插槽文件不可读 {}: {e}", p.display())))?;
        user_entry_skills.push(content);
    }
    Ok(Harness { dir: dir.to_path_buf(), manifest, framework_transparent, user_entry_skills })
}

/// 透明执行中和：命中 manifest 声明模式的行替换为透明执行指令（§9 #4 对齐）
fn neutralize_transparent(text: &str, patterns: &[String]) -> String {
    text.lines()
        .map(|l| {
            if patterns.iter().any(|p| !p.is_empty() && l.to_lowercase().contains(&p.to_lowercase())) {
                TRANSPARENT_MODE_LINE
            } else {
                l
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 改进#7 supervised 风险预评估（v1 确定性规则——LLM 评估为记档增强）。
/// 高危信号：大范围改动（>3 模块）/ 高危关键词 / 动低分模块（<50 分，手术风险高）。
/// 返回 (是否高危, 理由)。
pub fn risk_assess(task: &TaskRow) -> (bool, String) {
    let modules: Vec<String> = serde_json::from_str(&task.modules).unwrap_or_default();
    let mut reasons: Vec<String> = vec![];
    if modules.len() > 3 {
        reasons.push(format!("影响 {} 个模块（>3，大范围改动）", modules.len()));
    }
    for kw in ["重构", "架构", "整体", "全部", "删除", "迁移"] {
        if task.description.contains(kw) {
            reasons.push(format!("描述含高危关键词「{kw}」"));
            break;
        }
    }
    // 低分模块由调用方上下文难以获取——用 description 长度代理复杂度（长描述=大需求）
    if task.description.chars().count() > 200 {
        reasons.push("需求描述超长（>200 字，需求可能未收敛）".to_string());
    }
    if reasons.is_empty() {
        (false, format!("影响 {} 个模块，无高危信号", modules.len()))
    } else {
        (true, reasons.join("；"))
    }
}

/// 解析 agent stdout 的 `[EASYVIBE-RESULT] {json}` 归档行
/// （assemble_task_prompt 要求 agent 最后一行输出；从尾部找，容忍前后缀文字）
pub fn parse_result_line(output: &str) -> Option<serde_json::Value> {
    let line = output.lines().rev().find(|l| l.contains("[EASYVIBE-RESULT]"))?;
    let start = line.find("[EASYVIBE-RESULT]")? + "[EASYVIBE-RESULT]".len();
    let payload = line[start..].trim();
    serde_json::from_str(payload)
        .ok()
        .or_else(|| easyvibe_ai_agent::extract_json(payload).ok())
}

/// L2 哨兵：从当前越界候选中剔出**未上报过**的新文件（幂等——同一文件只预警一次，
/// 已上报集合随任务生命周期累计）。纯函数便于测试。
pub fn new_violators(candidates: &[String], reported: &mut std::collections::HashSet<String>) -> Vec<String> {
    let fresh: Vec<String> = candidates.iter().filter(|p| !reported.contains(*p)).cloned().collect();
    reported.extend(fresh.iter().cloned());
    fresh
}

/// L2 哨兵巡检间隔（秒）：默认 15s——够快能拦住"越界写一大片"的趋势，
/// 又不至于让 git 调用频率喧宾夺主（env 可调）。
fn sentry_interval() -> std::time::Duration {
    std::env::var("EASYVIBE_CONTRACT_SENTRY_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(15))
}

/// N26：任务槽会话超时（env 可调）——任务执行是自由 coding，40-60 分钟常态；
/// 透明槽位（归纳/巡检/子图）仍用 SessionManager 默认 30 分钟。
pub fn task_session_timeout() -> std::time::Duration {
    std::env::var("EASYVIBE_TASK_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or(std::time::Duration::from_secs(90 * 60))
}

// ---------- 影响面合约（战略审查第一 P0：任务声明的模块从注释升级为确定性边界） ----------

/// 路径是否落在合约 glob 范围内——与前端 moduleOfFile 同口径且**带路径段边界**：
/// `**` 前前缀去尾斜杠后，命中条件：路径恰为 base（精确文件型 glob，如 "src/main.rs"）、
/// 以 `base/` 开头（目录前缀）、或整段包含 `/base/`——
/// R2 审查实锤：starts_with("src/core") 会放过 src/coreography/，前缀必须有边界；
/// 自托管实弹（dogfood）：漏掉 path == base 会让精确文件型 glob 永不命中（回归锁死）。
pub fn path_within_contract(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|g| {
        let base = match g.find("**") {
            Some(i) => &g[..i],
            None => g.as_str(),
        };
        let base = base.trim_end_matches('/');
        if base.is_empty() {
            return false;
        }
        path == base || path.starts_with(&format!("{base}/")) || path.contains(&format!("/{base}/"))
    })
}

/// 从任务 context 提取影响面合约（创建任务时由模块展开写入；无合约返回空 = 不约束）
pub fn contract_patterns_from_context(context_json: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(context_json)
        .ok()
        .and_then(|v| v["contract"]["patterns"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|p| p.as_str().map(str::to_string))
        .collect()
}

/// 任务启动后变更的文件（越界校验候选集）：
/// - 已跟踪：`git diff --name-only <base>`（base 缺省 HEAD）——只算基线之后的改动
/// - 未跟踪：porcelain -uall 的 `??` 行（diff 系命令漏未跟踪，新建文件恰是最常见越界形态）
pub async fn changed_files(repo_root: &std::path::Path, base: Option<&str>) -> Vec<String> {
    let run = |args: &[&str]| {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::process::Command::new("git").args(args).current_dir(repo_root).output(),
        )
    };
    let mut out: Vec<String> = vec![];
    let diff_args: Vec<&str> = match base {
        Some(b) => vec!["diff", "--name-only", b],
        None => vec!["diff", "--name-only", "HEAD"],
    };
    if let Ok(Ok(o)) = run(&diff_args).await {
        if o.status.success() {
            out.extend(String::from_utf8_lossy(&o.stdout).lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()));
        }
    }
    if let Ok(Ok(o)) = run(&["status", "--porcelain=v1", "-uall"]).await {
        if o.status.success() {
            out.extend(
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .filter(|l| l.starts_with("?? "))
                    .filter_map(|l| l.get(3..).map(str::trim).map(str::to_string))
                    .filter(|l| !l.is_empty()),
            );
        }
    }
    out.sort();
    out.dedup();
    out
}

/// 启动基线快照：任务开始时工作区的全部脏文件（已跟踪改动 + 未跟踪）。
/// 采集时从越界候选集中扣减——任务之前就在那儿的陈年脏文件不进冤案（R2 审查裂缝#1）。
/// 存内存（spawn 与采集同进程；重启会把 running 任务标 interrupted，基线随之失效）。
///
/// 2026-10-03 实弹修订（hover-client 644 越界冤案）：基线的未跟踪部分必须**无视
/// .gitignore**——上一轮回被打回的实施改了 .gitignore（把 docs/ 加了忽略），基线
/// 快照时这批文件"被消失"，该轮 agent 恢复 .gitignore 后它们在采集时首次进入 git
/// 视野，644 个文件全部误判为"任务新增越界"。修复 = 基线并集
/// `git ls-files --others --ignored`（被忽略但存在的文件也是"早就有的"）。
/// 上限 5 万条防巨型 ignored 目录（node_modules 类）内存爆炸——超限则放弃该部分
/// （退回旧行为，宁可漏排不炸内存）。
pub async fn dirty_files(repo_root: &std::path::Path) -> Vec<String> {
    let Ok(Ok(o)) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new("git").args(["status", "--porcelain=v1", "-uall"]).current_dir(repo_root).output(),
    )
    .await
    else {
        return vec![]
    };
    if !o.status.success() {
        return vec![];
    }
    let mut v: Vec<String> = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| l.get(3..).map(str::trim).map(str::to_string))
        .filter(|l| !l.is_empty())
        .map(|p| p.split_once(" -> ").map(|(_, to)| to.trim().to_string()).unwrap_or(p))
        .collect();
    v.sort();
    v.dedup();
    // 被忽略但存在的文件：gitignore 游戏免疫（见上方注释）
    if let Ok(Ok(ig)) = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::process::Command::new("git")
            .args(["ls-files", "--others", "--ignored", "--exclude-standard", "-z"])
            .current_dir(repo_root)
            .output(),
    )
    .await
    {
        if ig.status.success() {
            let ignored: Vec<String> = String::from_utf8_lossy(&ig.stdout)
                .split('\0')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect();
            if ignored.len() <= 50_000 {
                v.extend(ignored);
                v.sort();
                v.dedup();
            }
        }
    }
    v
}

/// git 变更摘要（diff 关供料）：`git diff --stat`（已跟踪改动）+ `git status --porcelain`
/// （未跟踪新文件）。非 git 仓库返回 None——git 是增强项不是硬依赖（设计定稿）。
pub async fn git_change_summary(repo_root: &std::path::Path, base: Option<&str>) -> Option<String> {
    let run = |args: &[&str]| {
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::process::Command::new("git").args(args).current_dir(repo_root).output(),
        )
    };
    let mut parts: Vec<String> = vec![];
    let stat_args: Vec<&str> = match base {
        Some(b) => vec!["diff", "--stat", b],
        None => vec!["diff", "--stat", "HEAD"],
    };
    match run(&stat_args).await {
        Ok(Ok(out)) if out.status.success() => {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                parts.push(s);
            }
        }
        _ => return None, // 非 git 仓库或 git 不可用：无摘要可给
    }
    if let Ok(Ok(out)) = run(&["status", "--porcelain"]).await {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                parts.push(format!("工作区状态：\n{s}"));
            }
        }
    }
    if parts.is_empty() { None } else { Some(parts.join("\n")) }
}

/// 完整 diff（M4-3 diff 可视化）：`git diff HEAD`，256KB 封顶（超帽截断并标注）。
/// 非 git 仓库返回 None。落归档文件供 GET task-diff 读取，tasks.result 只带 stat 摘要不带全文。
pub async fn git_full_diff(repo_root: &std::path::Path, base: Option<&str>) -> Option<String> {
    let out = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        tokio::process::Command::new("git").args(match base {
            Some(b) => vec!["diff", b],
            None => vec!["diff", "HEAD"],
        }).current_dir(repo_root).output(),
    )
    .await
    .ok()?
    .ok()?;
    if !out.status.success() {
        return None; // 非 git 仓库
    }
    let mut s = String::from_utf8_lossy(&out.stdout).to_string();
    if s.is_empty() {
        return None;
    }
    const CAP: usize = 262_144;
    if s.len() > CAP {
        s.truncate(CAP);
        s.push_str("\n…（diff 超 256KB 已截断）");
    }
    Some(s)
}

/// 变更归因：读任务行拿 base_head（spawn 时已落库）；读不到（竞态/旧库）回退 None=HEAD 口径
async fn base_head_from_task(task_repo: &easyvibe_db::SqliteTaskRepository, task_id: &str) -> Option<String> {
    use easyvibe_db::TaskRepository as _;
    task_repo.get(task_id).await.ok().flatten().and_then(|t| t.base_head)
}

/// M4-1 终态采集：stdout 的 RESULT 行 + git 变更摘要 → `.easyvibe/development_docs/` 归档
/// （§9 #2：git 可见、随 PR 评审）→ tasks.result JSON（diff 关审批的展示原料）。
/// 两者皆无（agent 无输出且非 git 仓库）返回 None。
pub async fn collect_task_result(
    session_manager: &SessionManager,
    session_id: &str,
    repo_root: &std::path::Path,
    task_id: &str,
    task_repo: &easyvibe_db::SqliteTaskRepository,
    baseline: &[String],
) -> Option<String> {
    let output = session_manager.take_output(session_id).await.unwrap_or_default();
    let parsed = parse_result_line(&output);
    // 变更归因：优先用任务 base_head，缺省回退 HEAD（兼容旧任务与无 git）
    let base_owned = base_head_from_task(task_repo, task_id).await;
    let diff_stat = git_change_summary(repo_root, base_owned.as_deref()).await;
    let diff_full = git_full_diff(repo_root, base_owned.as_deref()).await;
    if parsed.is_none() && diff_stat.is_none() && diff_full.is_none() {
        return None;
    }
    // 实弹#3 防线：会话判成功但 agent 未输出 [EASYVIBE-RESULT] 行（可能被带偏/模型未遵从）——
    // 不阻断终态（与归纳产物核验同款哲学），但给审批人亮警告，diff 关须警惕"空执行"
    let mut warnings: Vec<String> = vec![];
    if parsed.is_none() {
        warnings.push("agent 未输出 [EASYVIBE-RESULT] 归档行——执行可能未按协议完成，审批时请核对 diff 是否为本任务改动".into());
    }
    // 影响面合约：越界写文件 = 红线（注入式护栏哲学——不阻断，但审批人必须看见）。
    // 确定性校验：基线之后的变更文件（扣启动时已有的脏文件，防冤案）× 创建时展开的模块 glob，零 LLM。
    let task_ctx = task_repo.get(task_id).await.ok().flatten().map(|t| t.context).unwrap_or_default();
    let contract = contract_patterns_from_context(&task_ctx);
    let baseline_set: std::collections::HashSet<&str> = baseline.iter().map(String::as_str).collect();
    // 产品自身 bookkeeping（.easyvibe/ 归档/地图产物）不属于 agent 改动——排除出合约校验
    let contract_violations: Vec<String> = if contract.is_empty() {
        vec![]
    } else {
        changed_files(repo_root, base_owned.as_deref())
            .await
            .into_iter()
            .filter(|p| !baseline_set.contains(p.as_str()))
            // 产品自身与 agent 脚手架的 bookkeeping（.easyvibe/ 归档/地图、.claude/ STAR 记忆）
            // 不属于任务改动——排除出合约校验（自托管实弹：harness STAR 归档路径约定待统一，见债务登记）
            .filter(|p| !p.starts_with(".easyvibe/") && !p.starts_with(".claude/"))
            .filter(|p| !path_within_contract(p, &contract))
            .collect()
    };
    if !contract_violations.is_empty() {
        let preview: Vec<String> = contract_violations.iter().take(5).cloned().collect();
        warnings.push(format!(
            "影响面合约：{} 个文件越出任务声明的模块边界——{}（diff 关须逐条确认或驳回）",
            contract_violations.len(),
            preview.join("、")
        ));
    }
    let mut archive = serde_json::json!({
        "taskId": task_id,
        "sessionId": session_id,
        "collectedAt": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string(),
        "result": parsed,
        "diffStat": diff_stat,
        "diffFull": diff_full,
        "warnings": warnings,
        "contractViolations": contract_violations,
        "archivedPath": serde_json::Value::Null,
    });
    // 阶段初审结论（phaseReviews）在终态采集时保留——实施阶段的 set_result 是整体重写，
    // 不携带会把矩阵/方案的初审记录冲掉（评审轮回缺一环）
    if let Ok(Some(row)) = task_repo.get(task_id).await {
        if let Some(res) = &row.result {
            if let Ok(old) = serde_json::from_str::<serde_json::Value>(res) {
                if let Some(pr) = old.get("phaseReviews") {
                    archive["phaseReviews"] = pr.clone();
                }
            }
        }
    }
    let dir = repo_root.join(".easyvibe/development_docs");
    if tokio::fs::create_dir_all(&dir).await.is_ok() {
        let path = dir.join(format!("{task_id}.json"));
        if easyvibe_map::atomic_write_json(&path, &archive).await.is_ok() {
            archive["archivedPath"] = serde_json::json!(path.to_string_lossy());
        }
    }
    // tasks.result 只带 stat 与归档路径（任务列表载荷可控）；diff 全文只进归档文件，
    // 由 GET /repos/{id}/tasks/{tid}/diff 按需读取（M4-3）
    let mut slim = archive;
    slim.as_object_mut()?.remove("diffFull");
    serde_json::to_string(&slim).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task(status: &str) -> TaskRow {
        TaskRow {
            id: "task-t1".into(),
            repo: "demo".into(),
            title: "修复耦合".into(),
            description: "把双向依赖改为单向".into(),
            modules: "[\"m1\"]".into(),
            acceptance: "无新增逆向".into(),
            source: "concern".into(),
            context: "{\"inject\":{\"module\":{\"id\":\"m1\"}}}".into(),
            status: status.into(),
            trust: "manual".into(),
            error: None,
            session_id: None,
            gate: None,
            conversation_id: None,
            prompt_tokens: None,
            completion_tokens: None,
            result: None,
            base_head: None,
            created_at: "1".into(),
            updated_at: "1".into(),
            origin_task_id: None,
            successor_task_id: None,
        }
    }

    /// 测试桩：最小 harness（只有框架正文，无 skill/中和）
    fn harness_stub(framework: &str) -> Arc<tokio::sync::RwLock<Harness>> {
        Arc::new(tokio::sync::RwLock::new(Harness {
            dir: std::env::temp_dir(),
            manifest: HarnessManifest {
                id: "stub".into(),
                version: "0".into(),
                builtin: false,
                route_rules: vec![],
                skills: HarnessSkills::default(),
                transparent_neutralize: vec![],
            },
            framework_transparent: framework.into(),
            user_entry_skills: vec![],
        }))
    }

    fn write_manifest(dir: &std::path::Path, neutralize: &[&str], user_entry: &[&str]) {
        let m = serde_json::json!({
            "id": "test-harness", "version": "1.0.0",
            "routeRules": [], "resultProtocol": "[EASYVIBE-RESULT]",
            "skills": { "userEntry": user_entry, "transparent": [] },
            "transparentNeutralize": neutralize,
        });
        std::fs::write(dir.join("manifest.json"), serde_json::to_string(&m).unwrap()).unwrap();
    }

    #[test]
    fn prompt_assembles_all_parts() {
        let dir = std::env::temp_dir().join("ev-harness-test2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("inject-prompt.md"), "框架内容 cat ~/.claude/hooks/rule_development.md").unwrap();
        write_manifest(&dir, &[], &[]);
        let harness = load_harness_from(&dir).unwrap();
        assert!(
            harness.framework_transparent.contains(&format!("{}/rule_development.md", dir.display())),
            "路径换姓生效"
        );

        let prompt = assemble_task_prompt(&harness.framework_transparent, &sample_task("pending"));
        assert!(prompt.contains("框架内容"));
        assert!(prompt.contains("把双向依赖改为单向"));
        assert!(prompt.contains("m1"));
        assert!(prompt.contains("无新增逆向"));
        assert!(prompt.contains("[EASYVIBE-RESULT]"));
        assert!(prompt.contains("\"inject\""));
    }

    #[test]
    fn harness_slots_assembly_and_neutralize_from_manifest() {
        // §12c 插槽内核回归：①透明装配的中和模式来自 manifest 声明（退役硬编码 grep）
        // ②user_entry 插槽正文装载（grill-me 以 skill 形态挂插槽，§9 #4）
        let dir = std::env::temp_dir().join("ev-harness-grill-test2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("skills/grill-me")).unwrap();
        std::fs::write(
            dir.join("inject-prompt.md"),
            "用户的表达一定是片面的，正式开始动手前，必须使用grill-me技能拷问用户。\ncat ~/.claude/hooks/rule_development.md",
        )
        .unwrap();
        std::fs::write(dir.join("skills/grill-me/SKILL.md"), "# grill-me\n访谈方法论正文").unwrap();

        // 场景 1：manifest 声明中和模式 → 透明框架不含拷问指令
        write_manifest(&dir, &["grill-me", "拷问"], &["skills/grill-me/SKILL.md"]);
        let h = load_harness_from(&dir).unwrap();
        assert!(!h.framework_transparent.to_lowercase().contains("grill-me"), "透明 agent 不得收到 grill-me 指令");
        assert!(!h.framework_transparent.contains("拷问"), "拷问指令必须被替换");
        assert!(h.framework_transparent.contains("透明执行模式"));
        assert_eq!(h.user_entry_skills.len(), 1, "user_entry 插槽正文应装载");
        assert!(h.user_entry_skills[0].contains("grill-me"), "插槽内容应是 skill 本体");

        // 场景 2：manifest 不声明中和 → 不过滤（证明驱动者是 manifest 而非硬编码）
        write_manifest(&dir, &[], &[]);
        let h2 = load_harness_from(&dir).unwrap();
        assert!(h2.framework_transparent.contains("grill-me"), "中和由 manifest 声明驱动");

        // 场景 3：出厂底账部署——清空目录后 load_harness 从内嵌底账补齐全部文件
        let dir3 = std::env::temp_dir().join("ev-harness-builtin-test");
        let _ = std::fs::remove_dir_all(&dir3);
        let h3 = load_harness_from(&dir3).unwrap();
        assert_eq!(h3.manifest.id, "builtin-default", "底账 manifest 应就位");
        assert!(!h3.framework_transparent.contains("拷问"), "出厂底账透明装配仍须中和");
        assert_eq!(h3.user_entry_skills.len(), 1, "出厂底账 user_entry=grill-me");
        assert!(dir3.join("rule_development.md").exists(), "规则正文应补齐");
    }

    #[test]
    fn contract_matcher_matches_frontend_semantics() {
        // 影响面合约：与前端 moduleOfFile 同口径——** 前缀匹配 + 路径段包含
        let pats = vec!["src/core/**".to_string(), "src/ui".to_string()];
        assert!(path_within_contract("src/core/a/b.ts", &pats));
        assert!(path_within_contract("lib/src/core/x.ts", &pats), "路径段包含命中");
        assert!(path_within_contract("src/ui/Button.tsx", &pats));
        assert!(!path_within_contract("src/other/c.ts", &pats));
        assert!(!path_within_contract("README.md", &pats));
        assert!(!path_within_contract("src/coreography/data.ts", &pats), "R2 实锤：前缀必须有段边界");
        assert!(!path_within_contract("src/core_plus/x.ts", &pats), "下划线前缀同样不得误配");
        // 精确文件型 glob（自托管实弹回归：path == base 必须命中，否则边界文件永被误报越界）
        let file_pats = vec!["src/main.rs".to_string()];
        assert!(path_within_contract("src/main.rs", &file_pats));
        assert!(!path_within_contract("src/main.rs.bak", &file_pats));
        // 空合约 = 不约束（未声明模块的任务不校验）
        assert!(!path_within_contract("anything", &[]));
        // context 提取
        let ctx = r#"{"contract":{"patterns":["a/**","b"]}}"#;
        assert_eq!(contract_patterns_from_context(ctx), vec!["a/**".to_string(), "b".to_string()]);
        assert!(contract_patterns_from_context("{}").is_empty());
    }

    #[tokio::test]
    async fn contract_violations_detected_at_collection() {
        // 影响面合约实弹：声明 allowed/**，agent 改了界内文件 + 越界新文件（未跟踪）——
        // 未跟踪必须被 porcelain 捕获（diff --stat 会漏，新建文件恰是最常见越界形态）
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let sessions = SessionManager::new(tx);
        let db = easyvibe_db::Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(easyvibe_db::SqliteTaskRepository::new(db.pool().clone()));
        let repo = std::env::temp_dir().join("ev-contract-test");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("allowed")).unwrap();
        std::fs::create_dir_all(repo.join("stray")).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("allowed/base.txt"), "1\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);

        let mut task = sample_task("running");
        task.id = "task-contract".into();
        task.context = r#"{"contract":{"patterns":["allowed/**"]}}"#.into();
        task_repo.create(&task).await.unwrap();

        // 界内修改 + 越界未跟踪新文件
        std::fs::write(repo.join("allowed/base.txt"), "1\n2\n").unwrap();
        std::fs::write(repo.join("stray/out.txt"), "oops\n").unwrap();
        let json = collect_task_result(&sessions, "no-such-session", &repo, "task-contract", &task_repo, &[]).await.expect("有 diff 即应采集");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let viol = v["contractViolations"].as_array().expect("必须有越界列表");
        assert_eq!(viol.len(), 1, "allowed/base.txt 界内、stray/out.txt 越界: {viol:?}");
        assert_eq!(viol[0].as_str().unwrap(), "stray/out.txt");
        assert!(
            v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("影响面合约")),
            "warnings 必须亮红线: {v}"
        );
        // 界内任务零误报
        std::fs::remove_file(repo.join("stray/out.txt")).unwrap();
        let mut clean = sample_task("running");
        clean.id = "task-contract-clean".into();
        clean.context = r#"{"contract":{"patterns":["allowed/**"]}}"#.into();
        task_repo.create(&clean).await.unwrap();
        let json = collect_task_result(&sessions, "no-such-session", &repo, "task-contract-clean", &task_repo, &[]).await.expect("有 diff 即应采集");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["contractViolations"].as_array().unwrap().is_empty(), "界内改动零误报: {v}");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[tokio::test]
    async fn contract_baseline_excludes_preexisting_dirt() {
        // R2 裂缝#1 回归：任务启动前就存在的脏文件（baseline 快照）不得算越界——
        // 否则红线出冤案：陈年脏仓库里每个任务都被误报
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let sessions = SessionManager::new(tx);
        let db = easyvibe_db::Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(easyvibe_db::SqliteTaskRepository::new(db.pool().clone()));
        let repo = std::env::temp_dir().join("ev-contract-baseline-test");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("allowed")).unwrap();
        std::fs::create_dir_all(repo.join("legacy")).unwrap();
        std::fs::create_dir_all(repo.join("stray")).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("allowed/base.txt"), "1\n").unwrap();
        std::fs::write(repo.join("legacy/old.txt"), "old\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);

        // 任务启动前 legacy/old.txt 已被改脏（baseline 快照会收录它）
        std::fs::write(repo.join("legacy/old.txt"), "old\ndirty\n").unwrap();
        // 任务执行：界内改动 + 全新越界文件
        std::fs::write(repo.join("allowed/base.txt"), "1\n2\n").unwrap();
        std::fs::write(repo.join("stray/new.txt"), "agent\n").unwrap();

        let mut task = sample_task("running");
        task.id = "task-baseline".into();
        task.context = r#"{"contract":{"patterns":["allowed/**"]}}"#.into();
        task_repo.create(&task).await.unwrap();
        // 基线 = 启动时脏文件（legacy/old.txt）——spawn 路径由 dirty_files 提供，测试直接给等价快照
        let baseline = vec!["legacy/old.txt".to_string()];
        let json = collect_task_result(&sessions, "no-such-session", &repo, "task-baseline", &task_repo, &baseline).await.expect("有 diff 即应采集");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let viol = v["contractViolations"].as_array().unwrap();
        assert_eq!(viol.len(), 1, "启动前的脏文件 legacy/old.txt 不得算越界: {viol:?}");
        assert_eq!(viol[0].as_str().unwrap(), "stray/new.txt");
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[tokio::test]
    async fn collect_warns_when_result_line_missing() {
        // 实弹#3 防线：会话成功但无 RESULT 行 + 有 git 改动 → 采集带警告（审批人警惕空执行/归因错位）
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let sessions = SessionManager::new(tx); // 无此会话 → 无输出 → 解析不到 RESULT
        let db = easyvibe_db::Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(easyvibe_db::SqliteTaskRepository::new(db.pool().clone()));
        let repo = std::env::temp_dir().join("ev-git-repo-warn-test");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").args(args).current_dir(&repo).output().unwrap();
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("x.txt"), "1\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        std::fs::write(repo.join("x.txt"), "1\n2\n").unwrap();
        let json = collect_task_result(&sessions, "no-such-session", &repo, "task-warn", &task_repo, &[]).await.expect("有 diff 即应采集");
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["result"].is_null());
        assert!(
            v["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("EASYVIBE-RESULT")),
            "缺 RESULT 行必须亮警告: {v}"
        );
    }

    #[test]
    fn sentry_violators_are_incremental_and_idempotent() {
        // L2 哨兵：只报新越界文件，重复巡检同一文件不重复预警（幂等）
        let mut reported = std::collections::HashSet::new();
        let c1 = vec!["stray/a.ts".to_string(), "stray/b.ts".to_string()];
        assert_eq!(new_violators(&c1, &mut reported), c1, "首报全量");
        assert!(new_violators(&c1, &mut reported).is_empty(), "重复巡检不重复报");
        let c2 = vec!["stray/a.ts".to_string(), "stray/c.ts".to_string()];
        assert_eq!(new_violators(&c2, &mut reported), vec!["stray/c".to_string() + ".ts"], "只报新增");
        assert_eq!(reported.len(), 3);
    }

    #[tokio::test]
    async fn decide_is_atomic_against_double_submit() {
        // N27 回归：原子关卡推进——同一审批关的第二次 decide（双击/重试）必须 409，
        // 不得双留痕、不得双 spawn。顺序执行即可复现（第一次推进后条件不再匹配）。
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-decide-atomic-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("awaiting_approval");
        task.repo = "ev-decide-atomic-test".into();
        task.id = "task-atomic".into();
        task.gate = Some("diff".into());
        task_repo.create(&task).await.unwrap();

        executor.decide("task-atomic", "approved", None, Some("diff")).await.unwrap();
        // 双击穿透复现：UI 仍停在 diff 关的第二次提交（声称 diff）→ 必须 409，不得推进到 report/done
        let err = executor.decide("task-atomic", "approved", None, Some("diff")).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))), "第二次 decide 必须 409: {err:?}");
        let t = task_repo.get("task-atomic").await.unwrap().unwrap();
        assert_eq!(t.gate.as_deref(), Some("report"), "只推进一次");
        let aps = approvals.list_by_task("task-atomic").await.unwrap();
        assert_eq!(aps.len(), 1, "只留痕一次（双审批留痕是 N27 的实弹症状）");
    }

    #[tokio::test]
    async fn decide_state_machine_guards() {
        // P0 审查后端#2：decision 白名单 + 仅 awaiting_approval 可审批（复活旁路封死）
        use easyvibe_db::{Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-decide-guard-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo_root = dir.clone();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&repo_root)]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );

        // pending 任务不可审批（此前会复活 spawn）
        let mut task = sample_task("pending");
        task.repo = "ev-decide-guard-test".into();
        task.id = "task-guard-pending".into();
        task_repo.create(&task).await.unwrap();
        let err = executor.decide("task-guard-pending", "approved", None, None).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))), "pending 任务必须 409: {err:?}");
        assert_eq!(task_repo.get("task-guard-pending").await.unwrap().unwrap().status, "pending", "状态不得被污染");

        // failed 任务不可审批（复活旁路）
        let mut task2 = sample_task("failed");
        task2.repo = "ev-decide-guard-test".into();
        task2.id = "task-guard-failed".into();
        task2.gate = Some("plan".into());
        task_repo.create(&task2).await.unwrap();
        let err = executor.decide("task-guard-failed", "approved", None, None).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))), "failed 任务必须 409");

        // 非法 decision 字符串一律 400（此前会被当 approved）
        let mut task3 = sample_task("awaiting_approval");
        task3.repo = "ev-decide-guard-test".into();
        task3.id = "task-guard-bad".into();
        task3.gate = Some("diff".into());
        task_repo.create(&task3).await.unwrap();
        let err = executor.decide("task-guard-bad", "whatever", None, None).await;
        assert!(matches!(err, Err(ApiError::BadRequest(_))), "非法 decision 必须 400: {err:?}");

        // 驳回无理由仍被拒（既有纪律不回归）
        let err = executor.decide("task-guard-bad", "rejected", None, None).await;
        assert!(matches!(err, Err(ApiError::BadRequest(_))));
    }

    #[tokio::test]
    async fn phase_review_merges_into_result_without_clobbering() {
        // 阶段初审结论入库语义：result 为 NULL 时新建骨架；二次合并不冲掉前一阶段结论
        use easyvibe_db::{Database, SqliteTaskRepository, TaskRepository as _};
        let db = Database::connect_memory().await.unwrap();
        let task_repo = SqliteTaskRepository::new(db.pool().clone());
        task_repo.create(&sample_task("awaiting_approval")).await.unwrap();

        merge_phase_review(&task_repo, "task-t1", "analysis", &ReviewVerdict { verdict: "pass".into(), summary: "矩阵完整".into() }).await;
        let r1: serde_json::Value =
            serde_json::from_str(&task_repo.get("task-t1").await.unwrap().unwrap().result.unwrap()).unwrap();
        assert_eq!(r1["phaseReviews"]["analysis"]["verdict"], "pass");

        merge_phase_review(&task_repo, "task-t1", "solution", &ReviewVerdict { verdict: "fail".into(), summary: "方案漏 R3".into() }).await;
        let r2: serde_json::Value =
            serde_json::from_str(&task_repo.get("task-t1").await.unwrap().unwrap().result.unwrap()).unwrap();
        assert_eq!(r2["phaseReviews"]["analysis"]["summary"], "矩阵完整", "analysis 结论必须保留");
        assert_eq!(r2["phaseReviews"]["solution"]["verdict"], "fail");
    }

    #[test]
    fn phase_review_prompt_targets_right_dir_and_readonly() {
        // 初审 prompt：阶段 1/2 指向各自产物目录；只审不改纪律与结论协议行必备
        let t = sample_task("running");
        let p1 = assemble_phase_review_prompt(&t, 1, "liyuhang");
        assert!(p1.contains("1_requirements_matrix/"));
        assert!(p1.contains("只审不改"));
        assert!(p1.contains("[EASYVIBE-REVIEW]"));
        assert!(p1.contains("把双向依赖改为单向"), "任务书必须注入作对照基准");
        let p2 = assemble_phase_review_prompt(&t, 2, "liyuhang");
        assert!(p2.contains("2_requirements_solutions/"));
    }

    #[tokio::test]
    async fn dirty_files_baseline_includes_gitignored_untracked() {
        // 2026-10-03 实弹回归（hover-client 644 越界冤案）：上轮被打回的 agent 改 .gitignore
        // 把 docs/ 变忽略 → 基线快照这批文件"被消失" → 本轮恢复 .gitignore 后它们首次进
        // git 视野，全部被误判"任务新增越界"。基线必须收录"被忽略但存在"的文件。
        let dir = std::env::temp_dir().join(format!("ev-dirty-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("docs")).unwrap();
        std::fs::write(dir.join(".gitignore"), "docs/\n").unwrap();
        std::fs::write(dir.join("docs/asset.png"), "x").unwrap();
        std::fs::write(dir.join("tracked.txt"), "t").unwrap();
        for args in [
            ["init", "-q"].as_slice(),
            ["add", "."].as_slice(),
            ["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"].as_slice(),
        ] {
            std::process::Command::new("git").args(args).current_dir(&dir).output().unwrap();
        }
        // 提交后制造"被忽略但未跟踪"与"普通未跟踪"
        std::fs::write(dir.join("docs/asset.png"), "y").unwrap();
        std::fs::write(dir.join("new.txt"), "n").unwrap();
        let d = dirty_files(&dir).await;
        assert!(d.iter().any(|p| p.contains("docs/asset.png")), "被忽略但存在的文件必须进基线: {:?}", d);
        assert!(d.iter().any(|p| p.contains("new.txt")), "普通未跟踪文件必须在基线: {:?}", d);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn manual_task_waits_for_approval_then_full_flow() {
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-task-exec-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo_root = dir.clone();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&repo_root)]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.repo = "ev-task-exec-test".into();
        task.trust = "manual".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-task-exec-test")).await;
        // manual：不 spawn，等待计划审批
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.status, "awaiting_approval");
        assert_eq!(t.gate.as_deref(), Some("plan"));
        // 分阶段执行流（2026-10-03）：plan → 阶段1 需求矩阵 → analysis 关 →
        // 阶段2 方案 → solution 关 → 阶段3 实施 →（审查不可用：true 无输出）→ diff → report → done。
        // true 立即成功，每关都异步回写——统一等关助手。
        async fn wait_gate(task_repo: &easyvibe_db::SqliteTaskRepository, want: &str) -> easyvibe_db::TaskRow {
            let mut t = task_repo.get("task-t1").await.unwrap().unwrap();
            for _ in 0..40 {
                if t.status == "awaiting_approval" && t.gate.as_deref() == Some(want) { return t }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                t = task_repo.get("task-t1").await.unwrap().unwrap();
            }
            panic!("未等到 {want} 关（当前 {:?}/{:?}）", t.status, t.gate);
        }
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // plan → 阶段1
        wait_gate(&task_repo, "analysis").await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // analysis → 阶段2
        wait_gate(&task_repo, "solution").await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // solution → 阶段3 实施
        wait_gate(&task_repo, "diff").await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // diff → report
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.gate.as_deref(), Some("report"));
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // report → done
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.status, "done");
        // 留痕：plan/analysis/solution/diff/report 五条 approved（分阶段全链路）
        let aps = approvals.list_by_task("task-t1").await.unwrap();
        assert_eq!(aps.len(), 5);
        assert!(aps.iter().all(|a| a.decision == "approved"));
    }

    #[tokio::test]
    async fn review_fail_auto_rejects_task() {
        // B案：独立子agent审查 fail → 任务自动打回（rejected），理由入留痕，不进 diff 关
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-task-review-fail-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        // echo 同时充当执行 agent 与审查 agent：打印 REVIEW 结论行（fail）
        // 主会话 echo 该行至 stdout——collect 无 RESULT 行不阻断；审查会话解析同一行 → fail
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("echo".into()),
            Arc::new(vec!["[EASYVIBE-REVIEW] {\"verdict\":\"fail\",\"summary\":\"存在阻断性问题\"}".into()]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.repo = "ev-task-review-fail-test".into();
        task.trust = "manual".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-task-review-fail-test")).await;
        // 分阶段流：plan → analysis → solution 三关都过，阶段3 实施后审查会话（echo 打 fail）→ 自动打回
        async fn wait_status(task_repo: &easyvibe_db::SqliteTaskRepository, want_status: &str, want_gate: Option<&str>) -> easyvibe_db::TaskRow {
            let mut t = task_repo.get("task-t1").await.unwrap().unwrap();
            for _ in 0..60 {
                let gate_ok = want_gate.map_or(true, |g| t.gate.as_deref() == Some(g));
                if t.status == want_status && gate_ok { return t }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                t = task_repo.get("task-t1").await.unwrap().unwrap();
            }
            panic!("未等到 {want_status}/{want_gate:?}（当前 {:?}/{:?}）", t.status, t.gate);
        }
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // plan → 阶段1
        wait_status(&task_repo, "awaiting_approval", Some("analysis")).await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // analysis → 阶段2
        wait_status(&task_repo, "awaiting_approval", Some("solution")).await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // solution → 阶段3 实施
        let t = wait_status(&task_repo, "rejected", None).await;
        assert_eq!(t.status, "rejected", "审查 fail 必须自动打回，不进 diff 关");
        assert!(t.error.as_deref().unwrap_or("").contains("存在阻断性问题"), "打回理由必须入 error 留痕");
        // 留痕：diff 关有一条 rejected 审批（审查打回），用户可见可溯源
        let aps = approvals.list_by_task("task-t1").await.unwrap();
        assert!(aps.iter().any(|a| a.gate == "diff" && a.decision == "rejected"), "审查打回必须留审批痕");
    }

    #[tokio::test]
    async fn review_unavailable_does_not_block_diff_gate() {
        // B案降级路径：审查会话产出无 [EASYVIBE-REVIEW] 行（如 true 命令）→ 审查不可用
        // → 不阻断，照常进 diff 关（人机审查兜底）
        use easyvibe_db::{Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-task-review-na-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()), // 无输出：审查会话拿不到结论行
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.repo = "ev-task-review-na-test".into();
        task.trust = "manual".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-task-review-na-test")).await;
        // 分阶段流三关走通；阶段3 实施后审查会话（true 无输出）拿不到结论 → 不阻断
        async fn wait_gate2(task_repo: &easyvibe_db::SqliteTaskRepository, want: &str) -> easyvibe_db::TaskRow {
            let mut t = task_repo.get("task-t1").await.unwrap().unwrap();
            for _ in 0..60 {
                if t.status == "awaiting_approval" && t.gate.as_deref() == Some(want) { return t }
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                t = task_repo.get("task-t1").await.unwrap().unwrap();
            }
            panic!("未等到 {want} 关（当前 {:?}/{:?}）", t.status, t.gate);
        }
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // plan → 阶段1
        wait_gate2(&task_repo, "analysis").await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // analysis → 阶段2
        wait_gate2(&task_repo, "solution").await;
        executor.decide("task-t1", "approved", None, None).await.unwrap(); // solution → 阶段3 实施
        let t = wait_gate2(&task_repo, "diff").await;
        assert_eq!(t.gate.as_deref(), Some("diff"), "审查不可用不得阻断 diff 关");
    }

    #[tokio::test]
    async fn auto_task_runs_straight_with_skipped_trace() {
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-task-exec-test2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.id = "task-auto".into();
        task.repo = "ev-task-exec-test2".into();
        task.trust = "auto".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-task-exec-test2")).await;
        let mut t = task_repo.get("task-auto").await.unwrap().unwrap();
        for _ in 0..10 {
            if t.status == "done" { break }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = task_repo.get("task-auto").await.unwrap().unwrap();
        }
        assert_eq!(t.status, "done");
        let aps = approvals.list_by_task("task-auto").await.unwrap();
        assert_eq!(aps.iter().filter(|a| a.decision == "skipped").count(), 3);
    }

    #[test]
    fn parse_result_line_tolerant() {
        let good = "前置输出若干行\n[EASYVIBE-RESULT] {\"summary\":\"修复完成\",\"changed_modules\":[\"m1\"]}";
        let v = parse_result_line(good).unwrap();
        assert_eq!(v["summary"], "修复完成");
        assert_eq!(v["changed_modules"][0], "m1");
        // 取最后一行（agent 可能多次提及）
        let multi = "[EASYVIBE-RESULT] {\"summary\":\"旧\"}\nnoise\n[EASYVIBE-RESULT] {\"summary\":\"新\"}";
        assert_eq!(parse_result_line(multi).unwrap()["summary"], "新");
        assert!(parse_result_line("没有任何归档行").is_none());
        assert!(parse_result_line("[EASYVIBE-RESULT] 不是json").is_none());
    }

    #[tokio::test]
    async fn git_change_summary_tracks_and_untracked() {
        // 非 git 目录 → None
        let plain = std::env::temp_dir().join("ev-not-a-repo");
        let _ = std::fs::remove_dir_all(&plain);
        std::fs::create_dir_all(&plain).unwrap();
        assert!(git_change_summary(&plain, None).await.is_none(), "非 git 仓库无摘要");

        // 真 git 仓库：提交后修改已跟踪文件 + 新增未跟踪文件
        let repo = std::env::temp_dir().join("ev-git-repo-test");
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(&repo).output().expect("git 执行失败")
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);
        std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(repo.join("b_new.txt"), "new\n").unwrap();
        let summary = git_change_summary(&repo, None).await.expect("git 仓库应有摘要");
        assert!(summary.contains("a.txt"), "已跟踪改动应入摘要: {summary}");
        assert!(summary.contains("b_new.txt"), "未跟踪新文件应入摘要: {summary}");
        // M4-3：完整 diff 含增行；非 git 目录返回 None
        let full = git_full_diff(&repo, None).await.expect("应有完整 diff");
        assert!(full.contains("+two"), "diff 应含新增行: {full}");
        assert!(full.contains("diff --git"), "标准 diff 格式: {full}");
        assert!(git_full_diff(&plain, None).await.is_none(), "非 git 仓库无完整 diff");
    }

    #[tokio::test]
    async fn auto_task_collects_result_and_archives() {
        use easyvibe_db::{Database, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-task-collect-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir.join(".easyvibe/map")).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(easyvibe_db::SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        // echo 打印 RESULT 行到 stdout（忽略 stdin prompt）——采集链路的零成本验证
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals,
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("echo".into()),
            Arc::new(vec!["[EASYVIBE-RESULT] {\"summary\":\"修复完成\",\"changed_modules\":[\"m1\"]}".to_string()]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.id = "task-collect".into();
        task.repo = "ev-task-collect-test".into();
        task.trust = "auto".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-task-collect-test")).await;
        let mut t = task_repo.get("task-collect").await.unwrap().unwrap();
        for _ in 0..10 {
            if t.status == "done" { break }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = task_repo.get("task-collect").await.unwrap().unwrap();
        }
        assert_eq!(t.status, "done");
        let result: serde_json::Value =
            serde_json::from_str(&t.result.expect("终态采集应写入 tasks.result")).unwrap();
        assert_eq!(result["result"]["summary"], "修复完成");
        assert_eq!(result["result"]["changed_modules"][0], "m1");
        assert!(result.get("diffFull").is_none(), "tasks.result 不背 diff 全文（列表载荷可控，M4-3）");
        // 归档文件落 development_docs/（§9 #2），且含 diffFull 键（按需端点读取）
        let archived = result["archivedPath"].as_str().expect("应返回归档路径");
        assert!(archived.contains("development_docs"), "归档须在 development_docs/: {archived}");
        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(archived).expect("归档文件应存在")).unwrap();
        assert_eq!(on_disk["taskId"], "task-collect");
        assert!(on_disk.get("diffFull").is_some(), "归档文件应含 diffFull（按需读取）");
    }

    #[tokio::test]
    async fn supervised_low_risk_runs_high_risk_stops_at_plan() {
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let dir = std::env::temp_dir().join("ev-supervised-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let maps = MapService::new(vec![easyvibe_map::repo_from_root(&dir)]);
        let executor = TaskExecutor::new(
            task_repo.clone(), approvals.clone(), sessions, maps,
            harness_stub("框架"), Arc::new("true".into()), Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        // 低危（盲测 P0 新语义）：计划关自动通过，执行完成后停 diff 关等人工审批——
        // 不再直通 done（此前 diff/report 两关留痕 skipped，监督与自动无法区分）
        let mut low = sample_task("pending");
        low.id = "task-low".into();
        low.repo = "ev-supervised-test".into();
        low.trust = "supervised".into();
        task_repo.create(&low).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-supervised-test")).await;
        let mut t = task_repo.get("task-low").await.unwrap().unwrap();
        for _ in 0..10 {
            if t.status == "awaiting_approval" { break }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = task_repo.get("task-low").await.unwrap().unwrap();
        }
        assert_eq!(t.status, "awaiting_approval", "低危执行完成后必须停 diff 关（盲测 P0：监督要有真审批）");
        assert_eq!(t.gate.as_deref(), Some("diff"), "停在 diff 关");
        let aps = approvals.list_by_task("task-low").await.unwrap();
        assert!(aps.iter().any(|a| a.decision == "skipped" && a.note.as_deref().unwrap_or("").contains("计划关自动通过")), "低危计划关自动通过须留痕带理由");
        // diff 关通过 → 报告关；报告关通过 → done
        executor.decide("task-low", "approved", None, Some("diff")).await.unwrap();
        let t = task_repo.get("task-low").await.unwrap().unwrap();
        assert_eq!((t.status.as_str(), t.gate.as_deref()), ("awaiting_approval", Some("report")), "diff 通过后停报告关");
        executor.decide("task-low", "approved", None, Some("report")).await.unwrap();
        let t = task_repo.get("task-low").await.unwrap().unwrap();
        assert_eq!(t.status, "done", "报告关通过后 done");
        // 高危：多模块 + 高危词 → 停 plan 关 + flagged 留痕
        let mut high = sample_task("pending");
        high.id = "task-high".into();
        high.repo = "ev-supervised-test".into();
        high.trust = "supervised".into();
        high.modules = "[\"a\",\"b\",\"c\",\"d\"]".into();
        high.description = "整体重构".into();
        task_repo.create(&high).await.unwrap();
        executor.clone().enqueue_pending(Some("ev-supervised-test")).await;
        let t = task_repo.get("task-high").await.unwrap().unwrap();
        assert_eq!(t.status, "awaiting_approval", "高危应停计划关");
        assert_eq!(t.gate.as_deref(), Some("plan"));
        let aps = approvals.list_by_task("task-high").await.unwrap();
        assert!(aps.iter().any(|a| a.decision == "flagged" && a.note.as_deref().unwrap_or("").contains("风险预评估")), "高危须 flagged 留痕带理由");
    }

    #[tokio::test]
    async fn reject_requires_reason() {
        use easyvibe_db::{ApprovalRepository as _, Database, SqliteApprovalRepository, SqliteTaskRepository};
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let approvals = Arc::new(SqliteApprovalRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![]);
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals.clone(),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.trust = "manual".into();
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("demo")).await;
        // M4-2：驳回无理由 → 400；有理由 → 终止且留痕
        let err = executor.decide("task-t1", "rejected", None, None).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)), "空理由驳回必须被拒: {err}");
        let err = executor.decide("task-t1", "rejected", Some("  "), None).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)), "空白理由同样被拒");
        executor.decide("task-t1", "rejected", Some("方案风险过大"), None).await.unwrap();
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.status, "rejected");
        let aps = approvals.list_by_task("task-t1").await.unwrap();
        assert_eq!(aps.len(), 1);
        assert_eq!(aps[0].decision, "rejected");
        assert_eq!(aps[0].note.as_deref(), Some("方案风险过大"));
    }

    #[tokio::test]
    async fn executes_to_terminal_with_stub() {
        use easyvibe_db::{Database, SqliteTaskRepository};
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let maps = MapService::new(vec![]);
        let approvals = Arc::new(easyvibe_db::SqliteApprovalRepository::new(db.pool().clone()));
        let executor = TaskExecutor::new(
            task_repo.clone(),
            approvals,
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()), // stub：立即成功
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.trust = "auto".into(); // auto 直通 → spawn_and_watch → 仓库未注册 → failed
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("demo")).await;
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.status, "failed");
    }

    #[tokio::test]
    async fn conflict_arm_returns_task_to_pending() {
        // R3 P0-1 回归：写互斥（仓库已有活动会话）时 spawn_and_watch 必须把任务退回 pending——
        // execute 已置 running，若不退回，retry 循环只扫 pending，任务假活 running 至重启
        // （N25 幽灵任务在 permits 满路径防过、Conflict 路径漏掉的孪生 bug）
        use easyvibe_db::{Database, SqliteTaskRepository};
        let dir = std::env::temp_dir().join("ev-conflict-arm-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Database::connect_memory().await.unwrap();
        let task_repo = Arc::new(SqliteTaskRepository::new(db.pool().clone()));
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sessions = SessionManager::new(tx);
        let repo = easyvibe_map::repo_from_root(&dir);
        let repo_id = repo.id.clone();
        let maps = MapService::new(vec![repo]);
        // 占住仓库：一个活动会话（等价于归纳/巡检进行中）
        sessions
            .try_register(easyvibe_api_types::SessionStatusChanged {
                repo: repo_id.clone(),
                session_id: "ind-blocker".into(),
                status: easyvibe_api_types::SessionStatus::Running,
            })
            .await
            .unwrap();
        let executor = TaskExecutor::new(
            task_repo.clone(),
            Arc::new(easyvibe_db::SqliteApprovalRepository::new(db.pool().clone())),
            sessions,
            maps,
            harness_stub("框架"),
            Arc::new("true".into()),
            Arc::new(vec![]),
            4,
            Arc::new(easyvibe_db::SqliteSettingsRepository::new(db.pool().clone())),
            None,
        );
        let mut task = sample_task("pending");
        task.repo = repo_id.clone();
        task.id = "task-conflict".into();
        task.trust = "auto".into();
        task_repo.create(&task).await.unwrap();

        executor.clone().execute(task).await;
        // 第一时间：execute 曾把状态推进 running，Conflict 臂必须已退回 pending
        let t = task_repo.get("task-conflict").await.unwrap().unwrap();
        assert_eq!(t.status, "pending", "写互斥必须退回 pending，不得滞留 running（假活）");

        // 等过 retry 节拍（5s 后 enqueue 重扫）： Conflict 依旧（会话仍占用），仍应停在 pending
        tokio::time::sleep(std::time::Duration::from_secs(7)).await;
        let t = task_repo.get("task-conflict").await.unwrap().unwrap();
        assert_eq!(t.status, "pending", "retry 重扫撞上持续互斥，任务应排队而非假活");

        // 2026-10-05 实弹回归（治理任务进度清零）：gate 带 p:implement 的 manual 任务
        // 被写互斥退回后，retry 必须原地保留阶段重跑——不得重置回 plan 关
        // （此前 manual 分支把 gate 重置回 plan = 已评审的矩阵/方案全部作废）。
        let mut task2 = sample_task("pending");
        task2.repo = repo_id.clone();
        task2.id = "task-conflict-phase".into();
        task2.trust = "manual".into();
        task2.gate = Some("p:implement".into());
        task_repo.create(&task2).await.unwrap();
        executor.clone().execute(task2).await;
        let t2 = task_repo.get("task-conflict-phase").await.unwrap().unwrap();
        assert_eq!(t2.status, "pending", "互斥退回 pending");
        assert_eq!(t2.gate.as_deref(), Some("p:implement"), "退回不得清阶段标记");
        tokio::time::sleep(std::time::Duration::from_secs(7)).await;
        let t2 = task_repo.get("task-conflict-phase").await.unwrap().unwrap();
        assert_eq!(t2.status, "pending", "持续互斥仍排队");
        assert_eq!(
            t2.gate.as_deref(),
            Some("p:implement"),
            "retry 重扫不得把 manual 任务重置回 plan 关（进度清零 bug 回归）"
        );
        assert_ne!(t2.status, "awaiting_approval", "绝不允许回退到计划审批关");
    }

    // ---------- 方案 v3 §4.4：路径换姓 + 版本迁移 ----------

    #[test]
    fn adapt_renames_all_three_claude_forms() {
        let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
        // 形态 1：inject-prompt.md 的 task.json 路径（user_name 在 development_docs 之前）
        let s1 = adapt_builtin_content(".claude/<user_name>/development_docs/task_260928_x.json");
        assert_eq!(s1, format!(".easyvibe/development_docs/{user}/task_260928_x.json"), "形态1换姓");
        // 形态 2：规则正文的产物路径（20+ 处）
        let s2 = adapt_builtin_content(".claude/development_docs/<user_name>/1_requirements_matrix/x.md");
        assert_eq!(s2, format!(".easyvibe/development_docs/{user}/1_requirements_matrix/x.md"), "形态2换姓");
        // 形态 3：附录目录树的裸 .claude/ 根行（复审残留）
        let s3 = adapt_builtin_content(".claude/\n├── development_docs/");
        assert_eq!(s3, ".easyvibe/\n├── development_docs/", "形态3换姓");
        // hooks 锚点保护：load 时替换链（load_harness_from）依赖 ~/.claude/hooks/ 原样存在
        let s4 = adapt_builtin_content("cat ~/.claude/hooks/rule_development.md");
        assert_eq!(s4, "cat ~/.claude/hooks/rule_development.md", "hooks 锚点不得换姓");
    }

    #[test]
    fn version_gt_semantics() {
        assert!(version_gt("1.2.0", "1.1.0"));
        assert!(version_gt("2.0.0", "1.10.0"), "数值段比较，非字典序");
        assert!(!version_gt("1.2.0", "1.2.0"));
        assert!(!version_gt("1.1.0", "1.2.0"));
        assert!(!version_gt("1.2", "1.2.0"), "缺段按 0，相等");
    }

    #[test]
    fn deploy_migrates_unmodified_rules_but_keeps_user_edits() {
        let dir = std::env::temp_dir().join("ev-harness-migrate-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 预置"旧版部署"：manifest 1.1.0 + 三份规则正文 = 出厂原稿（含 .claude）
        let builtin_ver = |s: &str| {
            BUILTIN_HARNESS.iter().find(|(r, _)| *r == "manifest.json").map(|(_, c)| {
                serde_json::from_str::<serde_json::Value>(c).unwrap()["version"].as_str().unwrap().to_string()
            }).unwrap_or_else(|| s.into())
        };
        let builtin_version = builtin_ver("");
        // 模拟磁盘上的旧 manifest：内置版本降一段，其余字段照抄
        let old_manifest = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../reference/manifest.json")).unwrap()
            .replace(&format!("\"version\": \"{builtin_version}\""), "\"version\": \"1.0.0\"");
        std::fs::write(dir.join("manifest.json"), &old_manifest).unwrap();
        let rd = BUILTIN_HARNESS.iter().find(|(r, _)| *r == "rule_development.md").unwrap().1;
        let rb = BUILTIN_HARNESS.iter().find(|(r, _)| *r == "rule_bugfix.md").unwrap().1;
        std::fs::write(dir.join("rule_development.md"), rd).unwrap();
        std::fs::write(dir.join("rule_bugfix.md"), rb).unwrap();
        // 用户编辑过的文件：加一行批注（内容不等于出厂原稿）
        std::fs::write(dir.join("inject-prompt.md"), format!("{}\n\n用户自定义补充", BUILTIN_HARNESS.iter().find(|(r, _)| *r == "inject-prompt.md").unwrap().1)).unwrap();

        deploy_builtin(&dir).unwrap();

        // 未改动的规则正文：重写为换姓版
        let new_rd = std::fs::read_to_string(dir.join("rule_development.md")).unwrap();
        assert!(!new_rd.contains(".claude/"), "未改动文件必须完成换姓");
        assert!(new_rd.contains(".easyvibe/development_docs"), "换姓目标路径");
        // 用户编辑过的文件：原样保留
        let kept = std::fs::read_to_string(dir.join("inject-prompt.md")).unwrap();
        assert!(kept.contains("用户自定义补充"), "用户编辑不得被迁移覆盖");
        // manifest：版本被抬到内置版，其余字段保留
        let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!(m["version"].as_str().unwrap(), builtin_version, "manifest 版本抬升");
        assert!(m["routeRules"].is_array(), "manifest 其余字段保留");
    }

    #[test]
    fn deploy_no_migrate_when_versions_equal() {
        let dir = std::env::temp_dir().join("ev-harness-nomigrate-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 磁盘版本 = 内置版本 → 即使文件还是旧内容也不重写（防无限迁移循环）
        deploy_builtin(&dir).unwrap(); // 首次：缺失补齐（已是换姓版）
        let rd_before = std::fs::read_to_string(dir.join("rule_development.md")).unwrap();
        deploy_builtin(&dir).unwrap(); // 第二次：版本相等，不动
        let rd_after = std::fs::read_to_string(dir.join("rule_development.md")).unwrap();
        assert_eq!(rd_before, rd_after, "版本相等时不得重写");
    }
}
