//! 会话生命周期与状态机：注册/单会话纪律、kill/grace 收尸、状态查询、
//! 旁路表（startedAt/label）、状态发布护栏（B1）、终态归属。
//! 拆自 lib.rs（2026-10-05 防膨胀）。

use crate::{SessionManager, SessionMetaEvent, SessionMetaUpdate, SessionOutput};
use chrono::{DateTime, Utc};
use easyvibe_api_types::{SessionStatus, SessionStatusChanged};
use easyvibe_common::ApiError;

impl SessionManager {
    /// 主动终止会话（P0 审查后端#1）：kill 信号 → 看门任务 select 命中 → start_kill → 判 Failed。
    /// 仅活动会话可 kill；已终态返回 409。
    pub async fn kill(&self, session_id: &str) -> Result<(), ApiError> {
        let Some(s) = self.by_id.read().await.get(session_id).cloned() else {
            return Err(ApiError::NotFound(format!("会话 {session_id} 不存在")));
        };
        if !matches!(s.status, SessionStatus::Starting | SessionStatus::Running) {
            return Err(ApiError::Conflict(format!("会话 {session_id} 已终态（{:?}）", s.status)));
        }
        let notify = self.killers.read().await.get(session_id).cloned();
        match notify {
            Some(n) => {
                n.notify_one();
                Ok(())
            }
            None => Err(ApiError::Conflict(format!("会话 {session_id} 无终止通道（外部注册会话不支持 kill）"))),
        }
    }

    /// grace 收尸（2026-10-03 重审 P1：归纳进度 100% 后 agent 不退出 = 僵尸进程泄漏）：
    /// 杀进程，但终态必须记 Succeeded——产物已交付，判 Failed 是冤案（实弹#4 的原始诉求）。
    /// 顺序保证：notify kill → 轮询等看门狗写出终态（Failed）→ note_status 覆盖为 Succeeded。
    /// 覆盖放在"观察到终态之后"，消除与看门狗终态写的竞态。
    pub async fn grace_finish(&self, session_id: &str) -> Result<(), ApiError> {
        let s = self.by_id.read().await.get(session_id).cloned().ok_or_else(|| ApiError::NotFound(format!("会话 {session_id} 不存在")))?;
        if !matches!(s.status, SessionStatus::Starting | SessionStatus::Running) {
            return Err(ApiError::Conflict(format!("会话 {session_id} 已终态（{:?}）", s.status)));
        }
        let notify = self.killers.read().await.get(session_id).cloned().ok_or_else(|| {
            ApiError::Conflict(format!("会话 {session_id} 无终止通道（外部注册会话不支持 kill）"))
        })?;
        notify.notify_one();
        // 等看门狗把 Killed→Failed 的终态写出来（kill 是即时的，2.5s 上限宽到离谱）
        for _ in 0..50 {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            if let Some(cur) = self.by_id.read().await.get(session_id) {
                if !matches!(cur.status, SessionStatus::Starting | SessionStatus::Running) {
                    break;
                }
            }
        }
        self.note_status(SessionStatusChanged {
            repo: s.repo,
            session_id: session_id.to_string(),
            status: SessionStatus::Succeeded,
        })
        .await;
        Ok(())
    }

    /// 订阅会话 stdout 行（app 层翻译为 WS session.output）
    pub fn subscribe_output(&self) -> tokio::sync::broadcast::Receiver<SessionOutput> {
        self.output_tx.subscribe()
    }

    /// M1/U1：订阅会话元事件（app 层落盘 agent_sessions 的数据源）
    pub fn subscribe_meta(&self) -> tokio::sync::broadcast::Receiver<SessionMetaEvent> {
        self.meta_tx.subscribe()
    }

    /// 会话 stdout（M4-1）：任务终态后解析 [EASYVIBE-RESULT] 的原料；无捕获返回 None。
    /// 注意：只读不取——缓冲在 take_output 消费前一直保留（契约：终态后随时可读）。
    pub async fn output_of(&self, session_id: &str) -> Option<String> {
        let buf = self.outputs.read().await.get(session_id)?.clone();
        Some(buf.lock().map(|s| s.clone()).unwrap_or_default())
    }

    /// 消费式领取 stdout 缓冲（领取即从表移除——SessionManager 内存只增不减债务的消解点：
    /// 任务执行器终态采集走这里；无人消费的历史缓冲由 map 容量自然受限于会话数，1MB/会话封顶）
    pub async fn take_output(&self, session_id: &str) -> Option<String> {
        let buf = self.outputs.write().await.remove(session_id)?;
        Some(buf.lock().map(|mut s| std::mem::take(&mut *s)).unwrap_or_default())
    }

    pub async fn status_of(&self, repo_id: &str) -> Option<SessionStatusChanged> {
        self.active.read().await.get(repo_id).cloned()
    }

    /// 全局活动会话（跨仓库视角——单仓库互斥，但 A 仓库分析时切到 B 发起分析 =
    /// 两会话并行合法；状态丸/运行页的全局指示器消费此查询）
    pub async fn all_active(&self) -> Vec<SessionStatusChanged> {
        self.active
            .read()
            .await
            .values()
            .filter(|s| matches!(s.status, SessionStatus::Starting | SessionStatus::Running))
            .cloned()
            .collect()
    }

    /// I1：app 层各发起入口打展示标签（「归纳」「巡检」「分析模块 X」「自动归纳」「任务执行」）
    pub async fn note_label(&self, session_id: &str, label: String) {
        self.labels.write().await.insert(session_id.to_string(), label.clone());
        // M1.1：标签同时经元事件落库（运行期 kind/label 可见——用量/运行页不再等终态）
        let repo_id = self
            .by_id
            .read()
            .await
            .get(session_id)
            .map(|s| s.repo.clone())
            .unwrap_or_default();
        let _ = self.meta_tx.send(SessionMetaEvent {
            repo: repo_id,
            session_id: session_id.to_string(),
            update: SessionMetaUpdate::Label(label),
        });
    }

    /// I1：会话注册时刻（气泡已运行时长的数据源；缺失则不显示时长）
    pub async fn started_at_of(&self, session_id: &str) -> Option<DateTime<Utc>> {
        self.started.read().await.get(session_id).copied()
    }

    /// I1：会话展示标签（缺失时前端降级显示「会话 {id}」）
    pub async fn label_of(&self, session_id: &str) -> Option<String> {
        self.labels.read().await.get(session_id).cloned()
    }

    /// 注册一个外部活动会话（如巡检）；仓库已有活动会话（归纳/巡检任一）则拒绝。
    /// 与 start_induction 共用同一纪律：地图写操作全局单飞。
    /// 注意：检查与插入必须在同一把写锁内完成（修代码审查发现的 TOCTOU 竞态）。
    pub async fn try_register(&self, s: SessionStatusChanged) -> Result<(), ApiError> {
        let mut map = self.active.write().await;
        if let Some(existing) = map.get(&s.repo) {
            if matches!(existing.status, SessionStatus::Starting | SessionStatus::Running) {
                return Err(ApiError::Conflict(format!(
                    "仓库 {} 有活动会话 {}，地图写操作需排队",
                    s.repo, existing.session_id
                )));
            }
        }
        map.insert(s.repo.clone(), s.clone());
        drop(map);
        // I1：注册即盖 startedAt 戳（唯一汇聚点——所有 spawn/外部注册路径都经这里）
        self.started.write().await.insert(s.session_id.clone(), Utc::now());
        // 非阻塞送达：通道满（下游死亡/测试无消费者）时事件丢弃——
        // 会话注册绝不能被监控通道背压卡死（2026-10-03 实弹：阶段初审让每任务
        // 会话数 4→6，测试 channel(16) 被填满，第 16 个 send 永久阻塞注册，死锁）
        if let Err(e) = self.events.try_send(s) {
            tracing::warn!("[session] 状态事件通道已满，丢弃事件（下游可能已死亡）: {e}");
        }
        Ok(())
    }

    /// 外部会话状态变更（不校验，直接发布）——巡检等自管生命周期的会话使用
    pub async fn note_status(&self, s: SessionStatusChanged) {
        self.publish(s).await;
    }
    pub(crate) async fn set_status(&self, repo: &str, session_id: &str, status: SessionStatus) {
        let s = SessionStatusChanged { repo: repo.into(), session_id: session_id.into(), status };
        self.publish(s).await;
    }

    /// 状态发布（B1 护栏在此）：`active[repo]` 已存在**不同 session_id 的活动会话**
    /// （Starting/Running）时，本条迟到事件（含 grace 收尸的迟到 Succeeded 终态）
    /// 不得覆写 active——整条事件丢弃 + warn。同 session_id 的正常状态推进不受限；
    /// by_id 按 session_id 键，天然不受跨会话影响。
    async fn publish(&self, s: SessionStatusChanged) {
        {
            let mut active = self.active.write().await;
            if let Some(existing) = active.get(&s.repo) {
                if existing.session_id != s.session_id
                    && matches!(existing.status, SessionStatus::Starting | SessionStatus::Running)
                {
                    tracing::warn!(
                        "[session] B1 护栏：丢弃迟到事件——仓库 {} 已有活动会话 {}，事件 {}（{:?}）不得覆写 active",
                        s.repo, existing.session_id, s.session_id, s.status
                    );
                    return;
                }
            }
            active.insert(s.repo.clone(), s.clone());
        }
        self.by_id.write().await.insert(s.session_id.clone(), s.clone());
        // B4（M1，2026-10-05）：终态不再删旁路表——startedAt/label 是 runs 页列表与
        // agent_sessions finalize 的兜底来源；两张表仅按会话数自然增长（每项几十字节，可控）。
        // （原清理逻辑移除；测试 bypass_tables_* 已同步改期望）
        // 非阻塞送达（背压卡死防线，同 try_register）
        if let Err(e) = self.events.try_send(s) {
            tracing::warn!("[session] 状态事件通道已满，丢弃事件: {e}");
        }
    }

    /// 按 session_id 查询状态（终态归属的唯一依据——避免按仓库轮询的归属竞态）
    pub async fn status_of_session(&self, session_id: &str) -> Option<SessionStatusChanged> {
        self.by_id.read().await.get(session_id).cloned()
    }}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn single_session_discipline() {
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        // 第一个会话：用 sleep 占住（macOS/Linux 均有）
        let s1 = mgr
            .start_induction("repo1", &dir, "test", "sleep", &["30".to_string()], None)
            .await
            .unwrap();
        assert_eq!(s1.status, SessionStatus::Running);
        // 同仓库第二个会话必须被拒
        let err = mgr.start_induction("repo1", &dir, "test", "sleep", &["1".to_string()], None).await;
        assert!(matches!(err, Err(ApiError::Conflict(_))));
        // 其他仓库不受影响
        assert!(mgr.start_induction("repo2", &dir, "test", "sleep", &["1".to_string()], None).await.is_ok());
    }

    // 收事件直到终态（队列里有 Starting/Running 前态）
    async fn recv_terminal(rx: &mut tokio::sync::mpsc::Receiver<SessionStatusChanged>) -> SessionStatusChanged {
        loop {
            let evt = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
                .await
                .expect("超时未收到会话事件")
                .expect("事件通道关闭");
            if matches!(evt.status, SessionStatus::Succeeded | SessionStatus::Failed) {
                return evt;
            }
        }
    }

    #[tokio::test]
    async fn write_mutex_shared_between_kinds() {
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        // 归纳会话占住仓库
        let _s = mgr.start_induction("repo1", &dir, "t", "sleep", &["30".to_string()], None).await.unwrap();
        // 巡检注册必须被拒（跨类型的写互斥）
        let err = mgr
            .try_register(SessionStatusChanged {
                repo: "repo1".into(),
                session_id: "patrol-x".into(),
                status: SessionStatus::Running,
            })
            .await;
        assert!(matches!(err, Err(ApiError::Conflict(_))));
        // 其他仓库的巡检不受影响
        assert!(mgr
            .try_register(SessionStatusChanged {
                repo: "repo2".into(),
                session_id: "patrol-y".into(),
                status: SessionStatus::Running,
            })
            .await
            .is_ok());
    }

    // P0 审查后端#1：主动 kill——挂死的 agent 可被杀掉并释放写互斥
    #[tokio::test]
    async fn kill_active_session_terminates_and_releases_mutex() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        let s = mgr.start_induction("repo1", &dir, "t", "sleep", &["30".to_string()], None).await.unwrap();
        mgr.kill(&s.session_id).await.unwrap();
        let evt = recv_terminal(&mut rx).await;
        assert_eq!(evt.session_id, s.session_id);
        assert!(matches!(evt.status, SessionStatus::Failed));
        // 终态后再 kill → 409；互斥随终态释放（新会话可启动）
        assert!(matches!(mgr.kill(&s.session_id).await, Err(ApiError::Conflict(_))));
        assert!(mgr.start_induction("repo1", &dir, "t", "sleep", &["0".to_string()], None).await.is_ok());
        // 不存在的会话 → 404
        assert!(matches!(mgr.kill("nope").await, Err(ApiError::NotFound(_))));
    }

    // P0 审查后端#1：超时防线——agent 挂死也不会永占写互斥（无需人工 kill）
    #[tokio::test]
    async fn hung_agent_killed_by_timeout() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new_with_timeout(tx, std::time::Duration::from_millis(150));
        let dir = std::env::temp_dir();
        let s = mgr.start_induction("repo1", &dir, "t", "sleep", &["30".to_string()], None).await.unwrap();
        let evt = recv_terminal(&mut rx).await;
        assert_eq!(evt.session_id, s.session_id);
        assert!(matches!(evt.status, SessionStatus::Failed), "超时必须判 Failed");
        assert!(mgr.start_induction("repo1", &dir, "t", "sleep", &["0".to_string()], None).await.is_ok(), "互斥已释放");
        // 终态清理：首个会话的 kill 通道已回收（第二个会话自己的通道随其终态回收）
        assert!(!mgr.killers.read().await.contains_key(&s.session_id));
    }

    #[tokio::test]
    async fn try_register_race_is_closed() {
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let mgr = SessionManager::new(tx);
        let mk = |sid: &str| SessionStatusChanged {
            repo: "race".into(),
            session_id: sid.into(),
            status: SessionStatus::Running,
        };
        // 真并发双注册：必须恰好一个成功（审查 🔴2 的回归测试）
        let (a, b) = tokio::join!(mgr.try_register(mk("s1")), mgr.try_register(mk("s2")));
        let ok_count = [a.is_ok(), b.is_ok()].into_iter().filter(|x| *x).count();
        assert_eq!(ok_count, 1, "并发注册必须恰好一个成功");
    }

    #[tokio::test]
    async fn spawn_failure_releases_registration() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        // 不存在的命令 → spawn 失败 → 会话必须被判 Failed 并释放（审查 Y1）
        let err = mgr.start_induction("repo1", &dir, "t", "definitely-not-a-real-cmd-xyz", &[], None).await;
        assert!(matches!(err, Err(ApiError::Internal(_))));
        // 终态 Failed 事件已发布
        let mut saw_failed = false;
        while let Ok(evt) = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await {
            let Some(e) = evt else { break };
            if e.status == SessionStatus::Failed {
                saw_failed = true;
                break;
            }
        }
        assert!(saw_failed, "spawn 失败必须发布 Failed 终态");
        // 注册已释放：同仓库可再注册
        assert!(mgr.try_register(SessionStatusChanged { repo: "repo1".into(), session_id: "next".into(), status: SessionStatus::Running }).await.is_ok());
    }

    #[tokio::test]
    async fn session_lifecycle_succeeded() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        // true 命令立即成功退出
        let s = mgr.start_induction("repo1", &dir, "test", "true", &[], None).await.unwrap();
        assert_eq!(s.status, SessionStatus::Running);
        let final_evt = recv_terminal(&mut rx).await;
        assert_eq!(final_evt.status, SessionStatus::Succeeded);
        assert_eq!(final_evt.repo, "repo1");
    }

    #[tokio::test]
    async fn session_output_captured_for_result_parsing() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        // echo 忽略 stdin 直接打印——stdout 必须被捕获供 M4-1 解析 [EASYVIBE-RESULT]
        let s = mgr.start_induction("repo1", &dir, "ignored", "echo", &["hello-easyvibe [EASYVIBE-RESULT] {\"summary\":\"x\"}".to_string()], None).await.unwrap();
        let _ = recv_terminal(&mut rx).await;
        let out = mgr.output_of(&s.session_id).await.expect("stdout 应被捕获");
        assert!(out.contains("hello-easyvibe"), "捕获内容: {out}");
        assert!(out.contains("[EASYVIBE-RESULT]"), "归档行必须保留（即使超帽路径）");
        // 未知会话返回 None
        assert!(mgr.output_of("nope").await.is_none());
    }

    #[tokio::test]
    async fn session_lifecycle_failed() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        let mgr = SessionManager::new(tx);
        let dir = std::env::temp_dir();
        let _ = mgr.start_induction("repo1", &dir, "test", "false", &[], None).await.unwrap();
        let final_evt = recv_terminal(&mut rx).await;
        assert_eq!(final_evt.status, SessionStatus::Failed);
    }

    // B1 护栏单测：迟到事件不得覆写新活动会话（整条丢弃，by_id 不受影响）
    #[tokio::test]
    async fn publish_guard_drops_late_event_from_other_session() {
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let mgr = SessionManager::new(tx);
        let mk = |sid: &str, status: SessionStatus| SessionStatusChanged {
            repo: "g1".into(),
            session_id: sid.into(),
            status,
        };
        // S1 走完终态 → S2 抢注为新活动会话
        mgr.note_status(mk("s1", SessionStatus::Failed)).await;
        mgr.try_register(mk("s2", SessionStatus::Running)).await.unwrap();
        // 迟到的非终态事件（同仓库旧会话）必须被丢弃
        mgr.note_status(mk("s1", SessionStatus::Running)).await;
        let cur = mgr.status_of("g1").await.unwrap();
        assert_eq!(cur.session_id, "s2", "迟到事件不得顶掉新活动会话");
        assert_eq!(cur.status, SessionStatus::Running);
        // grace 乱序：迟到的 Succeeded 终态同样不得覆写（B1 原始场景）
        mgr.note_status(mk("s1", SessionStatus::Succeeded)).await;
        let cur = mgr.status_of("g1").await.unwrap();
        assert_eq!(cur.session_id, "s2", "grace 迟到 Succeeded 不得顶掉新活动会话");
        assert_eq!(cur.status, SessionStatus::Running);
        // by_id 按 session_id 键：s1 保留自己的终态归属（try_register 不写 by_id，s2 经 status_of 观测）
        assert_eq!(mgr.status_of_session("s1").await.unwrap().status, SessionStatus::Failed);
        // 同 session_id 的正常推进不受限
        mgr.note_status(mk("s2", SessionStatus::Succeeded)).await;
        assert_eq!(mgr.status_of("g1").await.unwrap().status, SessionStatus::Succeeded);
    }

    // I1 旁路表 + B4（M1）：try_register 盖 startedAt 戳、note_label 打标；
    // 终态 publish 不再清理两表（历史可查，见 publish 注释）
    #[tokio::test]
    async fn bypass_tables_stamp_label_and_keep_on_terminal() {
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let mgr = SessionManager::new(tx);
        mgr.try_register(SessionStatusChanged {
            repo: "b1".into(),
            session_id: "s-1".into(),
            status: SessionStatus::Running,
        })
        .await
        .unwrap();
        let started = mgr.started_at_of("s-1").await.expect("注册必须盖 startedAt 戳");
        assert!((Utc::now() - started).num_seconds() < 5, "戳应为刚才: {started}");
        assert!(mgr.label_of("s-1").await.is_none());
        mgr.note_label("s-1", "归纳".into()).await;
        assert_eq!(mgr.label_of("s-1").await.as_deref(), Some("归纳"));
        // 活动期两表都在
        mgr.note_status(SessionStatusChanged { repo: "b1".into(), session_id: "s-1".into(), status: SessionStatus::Running }).await;
        assert!(mgr.started_at_of("s-1").await.is_some());
        assert!(mgr.label_of("s-1").await.is_some());
        // 终态 publish 保留（B4：runs 页列表与 agent_sessions finalize 的兜底来源）
        mgr.note_status(SessionStatusChanged { repo: "b1".into(), session_id: "s-1".into(), status: SessionStatus::Succeeded }).await;
        assert!(mgr.started_at_of("s-1").await.is_some(), "终态后 started 表项保留（B4）");
        assert!(mgr.label_of("s-1").await.is_some(), "终态后 label 表项保留（B4）");
        // 无竞争会话时 grace 路径的 note_status(Succeeded) 仍全量生效（既有行为不受护栏影响）
        mgr.try_register(SessionStatusChanged { repo: "b2".into(), session_id: "s-2".into(), status: SessionStatus::Running }).await.unwrap();
        mgr.note_status(SessionStatusChanged { repo: "b2".into(), session_id: "s-2".into(), status: SessionStatus::Succeeded }).await;
        assert_eq!(mgr.status_of("b2").await.unwrap().status, SessionStatus::Succeeded);
    }}
