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
    pub events: Option<tokio::sync::broadcast::Sender<crate::BusEvent>>,
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
        events: Option<tokio::sync::broadcast::Sender<crate::BusEvent>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            task_repo,
            approval_repo,
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
            match task.trust.as_str() {
                "manual" => {
                    let _ = self.task_repo.update_status(&task.id, "awaiting_approval", None).await;
                    let _ = self.task_repo.set_gate(&task.id, Some("plan")).await;
                    info!("[task-exec] 任务 {} 等待计划审批（manual）", task.id);
                    return;
                }
                "supervised" => {
                    let (high, reason) = risk_assess(&task);
                    if high {
                        let _ = self.task_repo.update_status(&task.id, "awaiting_approval", None).await;
                        let _ = self.task_repo.set_gate(&task.id, Some("plan")).await;
                        self.record_approval(&task.id, "plan", "flagged", Some(&format!("监督模式风险预评估：{reason}——已停在计划审批关"))).await;
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
                    self.spawn_and_watch(task).await;
                    return;
                }
                _ => {}
            }
            for gate in ["plan", "diff", "report"] {
                self.record_approval(&task.id, gate, "skipped", Some("自动模式直通，全程留痕")).await;
            }
            let _ = self.task_repo.update_status(&task.id, "running", None).await;
            self.spawn_and_watch(task).await;
        }
    }

    /// 审批决策（路由层调用）：approved 按关卡推进，rejected 终止；返回最新任务行。
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
                "任务当前状态 {} 不可审批（仅 awaiting_approval 可 decide——驳回返工请复制为新任务）",
                task.status
            )));
        }
        // N27 防穿透：以用户所见关卡为抢占条件——双击时第二发命中"关卡已推进"，原子 UPDATE 返回 0 行
        let gate = expected_gate.map(str::to_string).unwrap_or_else(|| task.gate.clone().unwrap_or_else(|| "plan".into()));
        // N27 原子化：先算目标态，再用条件 UPDATE 一次性抢占——影响行数 0 = 并发审批/状态漂移，409。
        // 审批留痕挪到抢占成功之后（此前先留痕再迁移，双击会双留痕 + 双 spawn）。
        let (new_gate, new_status) = match (decision, gate.as_str()) {
            ("rejected", _) => (Some("rejected"), Some("rejected")),
            (_, "plan") => (Some("diff"), Some("running")),
            (_, "diff") => (Some("report"), None),
            (_, "report") => (Some("done"), Some("done")),
            _ => return Err(ApiError::BadRequest(format!("未知关卡 {gate}"))),
        };
        let n = self.task_repo.try_advance_gate(&task.id, Some(&gate), new_gate, new_status).await?;
        if n == 0 {
            return Err(ApiError::Conflict("该任务刚被并发审批或状态已变化，请刷新后重试".into()));
        }
        self.record_approval(&task.id, &gate, if decision == "rejected" { "rejected" } else { "approved" }, note).await;
        if decision != "rejected" && gate == "plan" {
            self.clone().spawn_and_watch(task.clone()).await;
        }
        self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::Internal("任务丢失".into()))
    }

    async fn record_approval(&self, task_id: &str, gate: &str, decision: &str, note: Option<&str>) {
        use easyvibe_db::ApprovalRepository as _;
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
    async fn spawn_and_watch(self: Arc<Self>, task: TaskRow) {
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
        let prompt = {
            let h = self.harness.read().await;
            assemble_task_prompt(&h.framework_transparent, &task)
        };
        // Y5 清债：任务槽位参数覆盖——settings `agent.args.task`（JSON 数组）优先于全局，
        // 可把任务执行从 skip-permissions 收紧为带确认（归纳/巡检等透明槽位不受影响）
        let args = slot_args(&self.settings_repo, "task", &self.agent_args).await;
        match self
            .session_manager
            // N26：任务槽超时常态 90 分钟（自由 coding 40-60 分钟是常态，30 分钟一刀切会误杀）
            .start_induction(&repo.id, &repo.root, &prompt, &self.agent_command, &args, Some(task_session_timeout()))
            .await
        {
            Ok(session) => {
                let session_id = session.session_id.clone();
                let _ = self.task_repo.set_session(&task.id, &session_id).await;
                info!("[task-exec] 任务 {} 会话 {} 已启动（trust={}）", task.id, session_id, task.trust);
                let this = self.clone();
                let task_id = task.id.clone();
                let repo_name = task.repo.clone();
                let repo_root = repo.root.clone();
                let contract = contract.clone();
                tokio::spawn(async move {
                    let _permit = permit; // 许可随看门任务生命周期，并发上限真实生效（审查 🔴4）
                    let review_after = review_after;
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
                                    crate::publish(tx, crate::BusEvent::TaskContractAlert {
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
                                // 供 diff 关审批展示；采集失败不阻断终态回写
                                if !failed {
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
                                                    crate::publish(tx, crate::BusEvent::TaskContractViolated {
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
                                } else if review_after {
                                    // manual/supervised：执行成功 → 回到审批流（diff 关审产物；
                                    // supervised 低危首次停此处，gate 需从 None 置为 diff）
                                    ("awaiting_approval", Some("diff"))
                                } else {
                                    ("done", Some("done"))
                                };
                                let _ = this.task_repo.update_status(&task_id, status, None).await;
                                if let Some(g) = gate {
                                    let _ = this.task_repo.set_gate(&task_id, Some(g)).await;
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

## 执行要求

- 工作目录即仓库根目录；框架与规则正文中的路径已适配到本机，直接 cat 读取。
- 需求类型（功能开发 / Bug 修复 / 重构）由你按框架路由决断，选择对应规则正文执行。
- 改动规模评估与是否走完整评审流程由你决断（框架内的豁免条款），全程留痕。
- 统计/验证类结论用工具数准，禁止估算。
- 完成后最后一行输出：`[EASYVIBE-RESULT] {{"summary": "一句话总结", "changed_modules": ["模块id"]}}` 便于系统归档。"#,
        framework = framework,
        description = task.description,
        modules = if modules.is_empty() { "（未指定，由你分析）".into() } else { modules.join(", ") },
        acceptance = if task.acceptance.is_empty() { "（未指定）".into() } else { task.acceptance.clone() },
        context = task.context,
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
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
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

/// Y5：槽位级 agent 参数解析（settings `agent.args.<slot>` = JSON 字符串数组；
/// 解析失败/未配置回退全局默认——失败不阻断）
pub async fn slot_args(
    settings: &easyvibe_db::SqliteSettingsRepository,
    slot: &str,
    default: &[String],
) -> Vec<String> {
    use easyvibe_db::SettingsRepository as _;
    let v = settings.get("global", &format!("agent.args.{slot}")).await.ok().flatten()
        .and_then(|r| serde_json::from_str::<Vec<String>>(&r.value).ok());
    v.unwrap_or_else(|| default.to_vec())
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
        // 通过计划关 → 执行（true 立即成功）→ 回审批流（diff 关）
        executor.decide("task-t1", "approved", None, None).await.unwrap();
        // 等看门任务回写（2s 轮询）
        let mut t = task_repo.get("task-t1").await.unwrap().unwrap();
        for _ in 0..10 {
            if t.status == "awaiting_approval" && t.gate.as_deref() == Some("diff") { break }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = task_repo.get("task-t1").await.unwrap().unwrap();
        }
        assert_eq!(t.gate.as_deref(), Some("diff"), "执行成功后应停在 diff 关");
        // diff → report → done
        executor.decide("task-t1", "approved", None, None).await.unwrap();
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.gate.as_deref(), Some("report"));
        executor.decide("task-t1", "approved", None, None).await.unwrap();
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.status, "done");
        // 留痕：plan/diff/report 三条 approved
        let aps = approvals.list_by_task("task-t1").await.unwrap();
        assert_eq!(aps.len(), 3);
        assert!(aps.iter().all(|a| a.decision == "approved"));
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
