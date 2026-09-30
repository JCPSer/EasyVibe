//! 任务执行引擎（M3-3）：pending 任务 → 组装 harness 上下文 → spawn agent 执行 → 状态回写。
//! 分工：路由/拷问/豁免由 LLM 决断（注入框架+规则正文，§9 定稿）；
//! 并行上限 4（§11 🟡7）；透明 agent 不注入 grill-me（§9 #4）。
use easyvibe_common::ApiError;
use easyvibe_db::{TaskRepository as _, TaskRow};
use easyvibe_map::MapService;
use easyvibe_session::SessionManager;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{info, warn};

pub struct TaskExecutor {
    pub task_repo: Arc<easyvibe_db::SqliteTaskRepository>,
    pub approval_repo: Arc<easyvibe_db::SqliteApprovalRepository>,
    pub session_manager: Arc<SessionManager>,
    pub map_service: Arc<MapService>,
    /// inject-prompt 框架（路径已适配到本机 harness 目录）
    pub harness_framework: Arc<String>,
    pub agent_command: Arc<String>,
    pub agent_args: Arc<Vec<String>>,
    /// 并行执行上限（§11 🟡7 = 4）
    pub permits: Arc<Semaphore>,
}

impl TaskExecutor {
    pub fn new(
        task_repo: Arc<easyvibe_db::SqliteTaskRepository>,
        approval_repo: Arc<easyvibe_db::SqliteApprovalRepository>,
        session_manager: Arc<SessionManager>,
        map_service: Arc<MapService>,
        harness_framework: Arc<String>,
        agent_command: Arc<String>,
        agent_args: Arc<Vec<String>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            task_repo,
            approval_repo,
            session_manager,
            map_service,
            harness_framework,
            agent_command,
            agent_args,
            permits: Arc::new(Semaphore::new(4)),
        })
    }

    /// 扫描 pending 任务并执行（启动恢复 + 创建后触发共用）
    pub fn enqueue_pending<'a>(self: &'a Arc<Self>, repo_filter: Option<&'a str>) -> impl std::future::Future<Output = ()> + Send + 'a {
        async move {
        let repos: Vec<String> = match repo_filter {
            Some(r) => vec![r.to_string()],
            None => self.map_service.repos().into_iter().map(|r| r.id).collect(),
        };
        for repo_id in repos {
            let Ok(tasks) = self.task_repo.list(&repo_id, 50).await else { continue };
            for t in tasks.into_iter().filter(|t| t.status == "pending") {
                self.clone().execute(t).await;
            }
        }
        }
    }

    /// 执行单个任务（F5 两档分流）：manual 停在计划审批关（不 spawn）；
    /// auto 直通（三道关记 skipped 留痕）后 spawn。Conflict/并发满载的排队语义在 spawn_and_watch。
    pub fn execute(self: Arc<Self>, task: TaskRow) -> impl std::future::Future<Output = ()> + Send {
        async move {
            if task.trust == "manual" {
                let _ = self.task_repo.update_status(&task.id, "awaiting_approval", None).await;
                let _ = self.task_repo.set_gate(&task.id, Some("plan")).await;
                info!("[task-exec] 任务 {} 等待计划审批（manual）", task.id);
                return;
            }
            for gate in ["plan", "diff", "report"] {
                self.record_approval(&task.id, gate, "skipped", Some("自动模式直通，全程留痕")).await;
            }
            self.spawn_and_watch(task).await;
        }
    }

    /// 审批决策（路由层调用）：approved 按关卡推进，rejected 终止；返回最新任务行
    pub async fn decide(self: &Arc<Self>, task_id: &str, decision: &str, note: Option<&str>) -> Result<easyvibe_db::TaskRow, ApiError> {
        let task = self.task_repo.get(task_id).await?.ok_or_else(|| ApiError::NotFound(format!("任务 {task_id} 不存在")))?;
        let gate = task.gate.clone().unwrap_or_else(|| "plan".into());
        self.record_approval(&task.id, &gate, if decision == "rejected" { "rejected" } else { "approved" }, note).await;
        match (decision, gate.as_str()) {
            ("rejected", _) => {
                self.task_repo.update_status(&task.id, "rejected", note).await?;
                self.task_repo.set_gate(&task.id, Some("rejected")).await?;
            }
            (_, "plan") => {
                self.task_repo.update_status(&task.id, "running", None).await?;
                self.task_repo.set_gate(&task.id, Some("diff")).await?;
                self.clone().spawn_and_watch(task.clone()).await;
                return Ok(task);
            }
            (_, "diff") => {
                self.task_repo.set_gate(&task.id, Some("report")).await?;
            }
            (_, "report") => {
                self.task_repo.update_status(&task.id, "done", None).await?;
                self.task_repo.set_gate(&task.id, Some("done")).await?;
            }
            _ => return Err(ApiError::BadRequest(format!("未知关卡 {gate}"))),
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
        let trust_manual = task.trust == "manual";
        let repo = match self.map_service.find_repo(&task.repo) {
            Some(r) => r,
            None => {
                let _ = self.task_repo.update_status(&task.id, "failed", Some("仓库未注册")).await;
                return;
            }
        };
        let Ok(permit) = self.permits.clone().try_acquire_owned() else {
            warn!("[task-exec] {} 并发已满（4），稍后重试", task.id);
            let this = self.clone();
            let repo = task.repo.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                this.enqueue_pending(Some(&repo)).await;
            });
            return;
        };
        let prompt = assemble_task_prompt(&self.harness_framework, &task);
        match self
            .session_manager
            .start_induction(&repo.id, &repo.root, &prompt, &self.agent_command, &self.agent_args)
            .await
        {
            Ok(session) => {
                let session_id = session.session_id.clone();
                let _ = self.task_repo.set_session(&task.id, &session_id).await;
                info!("[task-exec] 任务 {} 会话 {} 已启动（trust={}）", task.id, session_id, task.trust);
                let this = self.clone();
                let task_id = task.id.clone();
                tokio::spawn(async move {
                    let _permit = permit; // 许可随看门任务生命周期，并发上限真实生效（审查 🔴4）
                    let trust_manual = trust_manual;
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        match this.session_manager.status_of_session(&session_id).await {
                            Some(s)
                                if !matches!(
                                    s.status,
                                    easyvibe_api_types::SessionStatus::Starting | easyvibe_api_types::SessionStatus::Running
                                ) =>
                            {
                                let failed = s.status == easyvibe_api_types::SessionStatus::Failed;
                                let (status, gate) = if failed {
                                    ("failed", None)
                                } else if trust_manual {
                                    // manual：执行成功 → 回到审批流（当前 gate=diff）
                                    ("awaiting_approval", None)
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
                warn!("[task-exec] 任务 {} 遇到写互斥，排队延迟重试", task.id);
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

/// harness 目录解析 + 框架装载（§9 适配动作：路径换姓 .claude → .easyvibe）。
/// 优先 ~/.easyvibe/harness/（缺失时从 reference/ 拷贝建仓）；框架文本内的
/// ~/.claude/hooks/ 路径替换为实际规则正文位置。
pub fn load_harness(workspace_reference: &std::path::Path) -> Result<(std::path::PathBuf, String), ApiError> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dir = std::env::var("EASYVIBE_HARNESS_DIR").map(Into::into).unwrap_or_else(|_| {
        let d = format!("{home}/.easyvibe/harness");
        if !std::path::Path::new(&d).join("inject-prompt.md").exists() && workspace_reference.join("inject-prompt.md").exists() {
            let _ = std::fs::create_dir_all(&d);
            for f in ["inject-prompt.md", "rule_development.md", "rule_bugfix.md"] {
                let _ = std::fs::copy(workspace_reference.join(f), std::path::Path::new(&d).join(f));
            }
        }
        d
    });
    let framework_path = std::path::Path::new(&dir).join("inject-prompt.md");
    let framework = std::fs::read_to_string(&framework_path)
        .map_err(|e| ApiError::Internal(format!("harness 框架不可读 {}: {e}", framework_path.display())))?;
    // 路径换姓：框架内引用的规则正文位置指向本机 harness 目录
    let adapted = framework.replace("~/.claude/hooks/", &format!("{}/", dir.trim_end_matches('/')));
    Ok((dir.into(), adapted))
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
            prompt_tokens: None,
            completion_tokens: None,
            created_at: "1".into(),
            updated_at: "1".into(),
        }
    }

    #[test]
    fn prompt_assembles_all_parts() {
        let framework = "框架内容 cat ~/.claude/hooks/rule_development.md";
        let dir = std::env::temp_dir().join("ev-harness-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("inject-prompt.md"), framework).unwrap();
        std::env::set_var("EASYVIBE_HARNESS_DIR", &dir);
        let (_, adapted) = load_harness(&dir).unwrap();
        assert!(adapted.contains(&format!("{}/rule_development.md", dir.display())), "路径换姓生效");

        let prompt = assemble_task_prompt(&adapted, &sample_task("pending"));
        assert!(prompt.contains("框架内容"));
        assert!(prompt.contains("把双向依赖改为单向"));
        assert!(prompt.contains("m1"));
        assert!(prompt.contains("无新增逆向"));
        assert!(prompt.contains("[EASYVIBE-RESULT]"));
        assert!(prompt.contains("\"inject\""));
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
            Arc::new("框架".into()),
            Arc::new("true".into()),
            Arc::new(vec![]),
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
        executor.decide("task-t1", "approved", None).await.unwrap();
        // 等看门任务回写（2s 轮询）
        let mut t = task_repo.get("task-t1").await.unwrap().unwrap();
        for _ in 0..10 {
            if t.status == "awaiting_approval" && t.gate.as_deref() == Some("diff") { break }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            t = task_repo.get("task-t1").await.unwrap().unwrap();
        }
        assert_eq!(t.gate.as_deref(), Some("diff"), "执行成功后应停在 diff 关");
        // diff → report → done
        executor.decide("task-t1", "approved", None).await.unwrap();
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.gate.as_deref(), Some("report"));
        executor.decide("task-t1", "approved", None).await.unwrap();
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
            Arc::new("框架".into()),
            Arc::new("true".into()),
            Arc::new(vec![]),
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
            Arc::new("框架".into()),
            Arc::new("true".into()), // stub：立即成功
            Arc::new(vec![]),
        );
        let mut task = sample_task("pending");
        task.trust = "auto".into(); // auto 直通 → spawn_and_watch → 仓库未注册 → failed
        task_repo.create(&task).await.unwrap();
        executor.clone().enqueue_pending(Some("demo")).await;
        let t = task_repo.get("task-t1").await.unwrap().unwrap();
        assert_eq!(t.status, "failed");
    }
}
