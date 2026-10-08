//! task_exec::ports —— task-engine 切片内的持久化端口与本地 DTO（依赖倒置，c-arch-7 R2）。
//!
//! 为什么：task-engine 与 server-api 是同层 application 模块，直接共享同一套具体仓储
//! （persistence 层）会让 schema 变更同时击穿两侧，且分层图上不可见。端口定义在**依赖方**（本切片），
//! 具体仓储的适配器落**组合根**（db_ports.rs）——与 `easyvibe-event-bus::queue::QueueHost`
//! 同构（端口在依赖方、适配器在编排层），依赖方向维持 server-api → task-engine，不成环。
//!
//! 注释纪律：本文件属 task_exec 生产文件，persistence 层 crate 名在此为**禁用字面量**（判据见
//! `tests/arch_guard.rs` 与 `scripts/check_app_db_boundary.py`）——故注释中亦不书写该标识符。

use easyvibe_common::ApiError;

/// 任务记录（本地 DTO）：字段与持久层任务行 **全等**，使 `task.gate` / `task.status` 等
/// 访问点在状态机内零改动；由组合根适配器做一次性字段搬运。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub repo: String,
    pub title: String,
    pub description: String,
    pub modules: String,
    pub acceptance: String,
    pub source: String,
    pub context: String,
    pub status: String,
    pub trust: String,
    pub error: Option<String>,
    pub session_id: Option<String>,
    pub gate: Option<String>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub result: Option<String>,
    pub base_head: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub conversation_id: Option<String>,
    pub origin_task_id: Option<String>,
    pub successor_task_id: Option<String>,
}

/// 审批留痕记录（本地 DTO）：id 与 decided_at 由组合根适配器补齐（调用方只关心语义留痕）。
#[derive(Debug, Clone)]
pub struct ApprovalRecord {
    pub task_id: String,
    pub gate: String,
    pub decision: String,
    pub note: Option<String>,
}

/// 任务读写端口（全部方法一一对应持久层语义；`try_*` 系列保留原子条件 UPDATE 的影响行数契约）。
#[async_trait::async_trait]
pub trait TaskStore: Send + Sync + 'static {
    async fn get(&self, id: &str) -> Result<Option<TaskRecord>, ApiError>;
    async fn list(&self, repo: &str, limit: i64) -> Result<Vec<TaskRecord>, ApiError>;
    async fn update_status(&self, id: &str, status: &str, error: Option<&str>) -> Result<(), ApiError>;
    async fn set_gate(&self, id: &str, gate: Option<&str>) -> Result<(), ApiError>;
    /// N27 原子关卡推进：影响行数 0 = 并发审批/状态漂移（调用方 409）。
    async fn try_advance_gate(
        &self,
        id: &str,
        expected_gate: Option<&str>,
        new_gate: Option<&str>,
        new_status: Option<&str>,
    ) -> Result<u64, ApiError>;
    async fn set_context(&self, id: &str, context: &str) -> Result<(), ApiError>;
    async fn reset_for_retry(&self, id: &str) -> Result<u64, ApiError>;
    async fn reset_for_remediate(&self, id: &str) -> Result<u64, ApiError>;
    async fn try_rewind(&self, id: &str, target_gate: &str) -> Result<u64, ApiError>;
    async fn set_session(&self, id: &str, session_id: &str) -> Result<(), ApiError>;
    async fn set_base_head(&self, id: &str, head: &str) -> Result<(), ApiError>;
    async fn set_result(&self, id: &str, result_json: &str) -> Result<(), ApiError>;
}

/// 审批落痕与查询端口（`list_by_task` 供「修改并复审」取最近一条打回意见）。
#[async_trait::async_trait]
pub trait ApprovalStore: Send + Sync + 'static {
    async fn record(&self, approval: &ApprovalRecord) -> Result<(), ApiError>;
    async fn list_by_task(&self, task_id: &str) -> Result<Vec<ApprovalRecord>, ApiError>;
}

/// 会话归属写端口（原 AgentSessionRepo 在切片内的唯一写点：任务 → 影响模块归因）。
#[async_trait::async_trait]
pub trait SessionAttribution: Send + Sync + 'static {
    async fn set_module_id(&self, session_id: &str, module_id: &str) -> Result<(), ApiError>;
}

/// 槽位 agent 解析端口（settings 优先解析链在组合根侧适配；返回既有 ResolvedAgent，零新面）。
#[async_trait::async_trait]
pub trait AgentSlotResolver: Send + Sync + 'static {
    async fn resolve(&self, slot: Option<&str>) -> easyvibe_ai_agent::agent_conf::ResolvedAgent;
}

/// 固定槽位解析器：不读设置、直接返回给定命令/参数（测试夹具专用——切片内不得引用组合根适配器）。
#[cfg(test)]
pub struct FixedAgentSlot {
    pub command: std::sync::Arc<String>,
    pub args: std::sync::Arc<Vec<String>>,
}

#[cfg(test)]
impl FixedAgentSlot {
    pub fn new(command: std::sync::Arc<String>, args: std::sync::Arc<Vec<String>>) -> Self {
        Self { command, args }
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl AgentSlotResolver for FixedAgentSlot {
    async fn resolve(&self, _slot: Option<&str>) -> easyvibe_ai_agent::agent_conf::ResolvedAgent {
        easyvibe_ai_agent::agent_conf::ResolvedAgent {
            command: (*self.command).clone(),
            args: (*self.args).clone(),
            agent_type: "claude".into(),
            source: "fixed".into(),
            preset: "fixed".into(),
        }
    }
}

// ---------------------------------------------------------------- 测试用内存端口实现（仅测试编译）
//
// 为什么（c-arch-15）：切片测试曾直接构造持久层具体仓储（`Database::connect_memory` +
// `Sqlite*Repository`），使地图上出现一条**只由测试面支撑**的 `task-engine → persistence` 边——
// 边的语义混合了生产与测试依赖。测试夹具改走本文件定义的内存端口后，切片内（含测试面）
// 对持久层 crate 的引用归零，地图只表达生产依赖；端口谓词（`is_test_face`）另在归纳口径侧
// 兜底同类假边（见 scripts/check_map_edge_test_face.py）。
//
// 保真纪律（与持久层仓储逐条对齐，见 easyvibe-db::task）：
// - `list` 过滤 repo 后按 `created_at` 降序、`LIMIT` 截断；
// - `try_*` 系列保留**原子条件 UPDATE 的影响行数契约**（0 行 = 并发/状态漂移，调用方 409）：
//   判定与写入在同一把锁内完成（check-then-act 原子），条件表达式逐字对齐 SQL 的 WHERE：
//   `status = 'awaiting_approval' AND gate IS ?`（NULL 安全）；
// - `COALESCE(?, col)` 语义：新值为 None 时不覆盖原列；
// - 时间戳口径同 tasks 表：epoch 毫秒（13 位）；
// - `list_by_task` 返回 `decided_at` 升序（= 插入序），供生产侧 `.last()` 取最近一条打回意见。

/// 切片内时间戳口径（epoch 毫秒，与持久层 tasks 表一致）。
#[cfg(test)]
fn now_ms() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
        .to_string()
}

/// 内存任务端口实现（仅测试编译）：语义对齐持久层任务仓储。
#[cfg(test)]
pub struct InMemoryTaskStore {
    rows: std::sync::Mutex<Vec<TaskRecord>>,
}

#[cfg(test)]
impl InMemoryTaskStore {
    pub fn new() -> Self {
        Self { rows: std::sync::Mutex::new(Vec::new()) }
    }

    /// 播种（对应持久层 `TaskRepository::create`）：同 id 覆盖。
    /// 显式偏差：持久层 `create` 撞主键会报错（INSERT 语义），本播种方法为测试便利**覆盖**同名行
    /// （测试内 id 唯一，不触达该差异）。
    pub fn seed(&self, t: &TaskRecord) {
        let mut rows = self.rows.lock().unwrap();
        rows.retain(|r| r.id != t.id);
        rows.push(t.clone());
    }

    /// 条件更新助手：命中行则交给 `f` 改列并刷新 `updated_at`，返回是否命中（= 影响行数 1/0）。
    fn update_if<F: FnOnce(&mut TaskRecord)>(&self, id: &str, f: F) -> bool {
        let mut rows = self.rows.lock().unwrap();
        match rows.iter_mut().find(|r| r.id == id) {
            Some(r) => {
                f(r);
                r.updated_at = now_ms();
                true
            }
            None => false,
        }
    }

    /// 非条件更新助手：命中则改列（**不**刷 `updated_at`，对齐持久层 set_gate/set_session/... 的口径）。
    fn update_plain<F: FnOnce(&mut TaskRecord)>(&self, id: &str, f: F) {
        let mut rows = self.rows.lock().unwrap();
        if let Some(r) = rows.iter_mut().find(|r| r.id == id) {
            f(r);
        }
    }
}

#[cfg(test)]
impl Default for InMemoryTaskStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl TaskStore for InMemoryTaskStore {
    async fn get(&self, id: &str) -> Result<Option<TaskRecord>, ApiError> {
        Ok(self.rows.lock().unwrap().iter().find(|r| r.id == id).cloned())
    }

    async fn list(&self, repo: &str, limit: i64) -> Result<Vec<TaskRecord>, ApiError> {
        let rows = self.rows.lock().unwrap();
        // ORDER BY created_at DESC LIMIT ?——与持久层**实际执行计划**逐字等价：迁移建有
        // idx_tasks_repo(repo, created_at)，DESC 走索引反向扫描，故并列 created_at 的行按 rowid
        // **倒序**返回（实测：同 created_at 依序插入 a,b,c → 返回 c,b,a）。此处以「倒插入序」对齐；
        // 若按插入序（rowid 升序）会与仓储不同序，而 enqueue_pending 依 list 顺序派发任务。
        let mut picked: Vec<(usize, &TaskRecord)> =
            rows.iter().enumerate().filter(|(_, r)| r.repo == repo).collect();
        picked.sort_by(|(ia, a), (ib, b)| b.created_at.cmp(&a.created_at).then(ib.cmp(ia)));
        let mut out: Vec<TaskRecord> = picked.into_iter().map(|(_, r)| r.clone()).collect();
        if limit >= 0 {
            out.truncate(limit as usize);
        }
        Ok(out)
    }

    async fn update_status(&self, id: &str, status: &str, error: Option<&str>) -> Result<(), ApiError> {
        self.update_if(id, |r| {
            r.status = status.to_string();
            r.error = error.map(str::to_string);
        });
        Ok(())
    }

    async fn set_gate(&self, id: &str, gate: Option<&str>) -> Result<(), ApiError> {
        self.update_plain(id, |r| r.gate = gate.map(str::to_string));
        Ok(())
    }

    async fn try_advance_gate(
        &self,
        id: &str,
        expected_gate: Option<&str>,
        new_gate: Option<&str>,
        new_status: Option<&str>,
    ) -> Result<u64, ApiError> {
        // 同一把锁内完成判定 + 写入：等价于单条条件 UPDATE（N27 双击穿透的消解点）
        let mut rows = self.rows.lock().unwrap();
        let Some(r) = rows.iter_mut().find(|r| r.id == id) else { return Ok(0) };
        let matched = r.status == "awaiting_approval" && r.gate.as_deref() == expected_gate;
        if !matched {
            return Ok(0);
        }
        if let Some(g) = new_gate {
            r.gate = Some(g.to_string()); // COALESCE(?, gate)
        }
        if let Some(s) = new_status {
            r.status = s.to_string(); // COALESCE(?, status)
        }
        r.updated_at = now_ms();
        Ok(1)
    }

    async fn set_context(&self, id: &str, context: &str) -> Result<(), ApiError> {
        self.update_plain(id, |r| r.context = context.to_string());
        Ok(())
    }

    async fn reset_for_retry(&self, id: &str) -> Result<u64, ApiError> {
        // WHERE status IN ('failed','interrupted')：非白名单状态影响行数 0（调用方 409）
        Ok(if self.update_if_guarded(id, |r| matches!(r.status.as_str(), "failed" | "interrupted"), |r| {
            r.status = "pending".into();
            r.error = None;
            r.gate = None;
            r.session_id = None;
        }) { 1 } else { 0 })
    }

    async fn reset_for_remediate(&self, id: &str) -> Result<u64, ApiError> {
        // WHERE status = 'rejected'（只认 rejected：用户打回走复制新任务，审查打回走本通道）
        Ok(if self.update_if_guarded(id, |r| r.status == "rejected", |r| {
            r.status = "running".into();
            r.error = None;
            r.gate = Some("p:implement".into());
            r.session_id = None;
        }) { 1 } else { 0 })
    }

    async fn try_rewind(&self, id: &str, target_gate: &str) -> Result<u64, ApiError> {
        // WHERE status IN ('awaiting_approval','failed','interrupted','rejected','done')
        Ok(if self.update_if_guarded(
            id,
            |r| matches!(r.status.as_str(), "awaiting_approval" | "failed" | "interrupted" | "rejected" | "done"),
            |r| {
                r.status = "awaiting_approval".into();
                r.error = None;
                r.gate = Some(target_gate.to_string());
                r.session_id = None;
            },
        ) { 1 } else { 0 })
    }

    async fn set_session(&self, id: &str, session_id: &str) -> Result<(), ApiError> {
        self.update_plain(id, |r| r.session_id = Some(session_id.to_string()));
        Ok(())
    }

    async fn set_base_head(&self, id: &str, head: &str) -> Result<(), ApiError> {
        self.update_plain(id, |r| r.base_head = Some(head.to_string()));
        Ok(())
    }

    async fn set_result(&self, id: &str, result_json: &str) -> Result<(), ApiError> {
        self.update_plain(id, |r| r.result = Some(result_json.to_string()));
        Ok(())
    }
}

#[cfg(test)]
impl InMemoryTaskStore {
    /// 条件写（WHERE 谓词由 `guard` 给出）：命中即写入并刷 `updated_at`，返回是否命中。
    fn update_if_guarded<G: FnOnce(&TaskRecord) -> bool, F: FnOnce(&mut TaskRecord)>(
        &self,
        id: &str,
        guard: G,
        f: F,
    ) -> bool {
        let mut rows = self.rows.lock().unwrap();
        let Some(r) = rows.iter_mut().find(|r| r.id == id) else { return false };
        if !guard(r) {
            return false;
        }
        f(r);
        r.updated_at = now_ms();
        true
    }
}

/// 内存审批端口实现（仅测试编译）：`list_by_task` 按插入序返回（= 持久层 `ORDER BY decided_at`，
/// 同秒内保持 rowid 升序），使生产侧 `.last()` 恒为最近一条。
#[cfg(test)]
pub struct InMemoryApprovalStore {
    rows: std::sync::Mutex<Vec<ApprovalRecord>>,
}

#[cfg(test)]
impl InMemoryApprovalStore {
    pub fn new() -> Self {
        Self { rows: std::sync::Mutex::new(Vec::new()) }
    }
}

#[cfg(test)]
impl Default for InMemoryApprovalStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[async_trait::async_trait]
impl ApprovalStore for InMemoryApprovalStore {
    async fn record(&self, approval: &ApprovalRecord) -> Result<(), ApiError> {
        self.rows.lock().unwrap().push(approval.clone());
        Ok(())
    }

    async fn list_by_task(&self, task_id: &str) -> Result<Vec<ApprovalRecord>, ApiError> {
        Ok(self.rows.lock().unwrap().iter().filter(|a| a.task_id == task_id).cloned().collect())
    }
}
