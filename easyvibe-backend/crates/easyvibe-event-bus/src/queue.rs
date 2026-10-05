//! 运行会话队列（需求方案「运行会话气泡 + 单会话排队 v1」§4.1/§5/§8）。
//!
//! 纯数据（`JobKind` / `QueuedJob` / `QueueChange`）+ 单槽状态机（`QueueState`）。
//! 原 `easyvibe-app/src/session_queue.rs` 的队列核心部分迁移而来，行为语义零改动。
//!
//! 执行入口经 [`QueueHost`] trait 回调注入（由 server-api 实现）：队列核心不再
//! `crate::AppState` / `crate::start_*_inner`，故 application 层可单向依赖本模块。

use chrono::{DateTime, Utc};
use easyvibe_common::ApiError;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use crate::bus::BusEvent;

/// 排队任务类型（patrol|reinduce|submap；任务槽位不参与排队——任务退回 pending 有自己的机制）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Patrol,
    Reinduce,
    Submap,
}

impl JobKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobKind::Patrol => "patrol",
            JobKind::Reinduce => "reinduce",
            JobKind::Submap => "submap",
        }
    }
}

#[derive(Debug, Clone)]
pub struct QueuedJob {
    pub kind: JobKind,
    /// Submap 必填（模块 id）；其余类型为 None
    pub module_id: Option<String>,
    /// 展示用：「巡检」「归纳」「分析模块 <id>」
    pub label: String,
    pub enqueued_at: DateTime<Utc>,
}

/// 队列变更（每次变更都经 queue.changed 广播，payload 见 ws_handler 的翻译）
#[derive(Debug, Clone)]
pub enum QueueChange {
    Enqueued { job: QueuedJob },
    /// S3：替换时携带被替换项 label，前端 toast「已替换之前的排队：{旧label}」
    Replaced { job: QueuedJob, replaced_label: String },
    Cancelled { job: QueuedJob },
    /// I3：排空携带任务信息与 started 标记
    Drained { job: QueuedJob, started: bool },
    /// B2：执行撞 Conflict（TOCTOU），项放回原槽位（未覆盖期间用户新排队）
    Requeued { job: QueuedJob },
    /// B2：确定性失败，丢弃 + 广播失败原因（不留死信，避免无限重试）
    Failed { job: QueuedJob, error: String },
}

impl QueueChange {
    fn job(&self) -> &QueuedJob {
        match self {
            QueueChange::Enqueued { job }
            | QueueChange::Replaced { job, .. }
            | QueueChange::Cancelled { job }
            | QueueChange::Drained { job, .. }
            | QueueChange::Requeued { job }
            | QueueChange::Failed { job, .. } => job,
        }
    }

    pub fn type_str(&self) -> &'static str {
        match self {
            QueueChange::Enqueued { .. } => "enqueued",
            QueueChange::Replaced { .. } => "replaced",
            QueueChange::Cancelled { .. } => "cancelled",
            QueueChange::Drained { .. } => "drained",
            QueueChange::Requeued { .. } => "requeued",
            QueueChange::Failed { .. } => "failed",
        }
    }

    /// WS payload：repo/type/job + 各类型附加字段（replacedLabel/started/error）
    pub fn to_payload(&self, repo: &str) -> serde_json::Value {
        let job = self.job();
        let mut data = serde_json::json!({
            "repo": repo,
            "type": self.type_str(),
            "job": {
                "kind": job.kind.as_str(),
                "label": job.label,
                "moduleId": job.module_id,
                "enqueuedAt": job.enqueued_at,
            },
        });
        match self {
            QueueChange::Replaced { replaced_label, .. } => data["replacedLabel"] = replaced_label.clone().into(),
            QueueChange::Drained { started, .. } => data["started"] = (*started).into(),
            QueueChange::Failed { error, .. } => data["error"] = error.clone().into(),
            _ => {}
        }
        data
    }
}

/// 队列宿主：由服务面（server-api）实现，向状态机注入三项能力——发事件、查活动会话、执行任务。
/// 执行入口以回调注入，队列核心因此不反向依赖服务面。
/// 用 `#[async_trait]` 保证返回 future 为 `Send`（`drain` 内 `tokio::spawn` 需要）。
#[async_trait::async_trait]
pub trait QueueHost: Clone + Send + Sync + 'static {
    fn publish_event(&self, ev: BusEvent);
    /// Starting | Running 视为活动
    async fn has_active_session(&self, repo: &str) -> bool;
    async fn run_job(&self, repo: &str, job: &QueuedJob) -> Result<(), ApiError>;
}

/// 单槽队列状态机：每仓库同时最多排 1 个（防链式雪崩，需求 §4.3 明确不做多槽）。
/// 内存态，重启即失（S4：重启后首帧 GET 自然清态，无需额外机制）。
#[derive(Default)]
pub struct QueueState {
    inner: Mutex<HashMap<String, QueuedJob>>,
}

impl QueueState {
    pub fn new() -> Self {
        Self::default()
    }

    /// POST 入队/替换：返回被替换项 label（首次入队为 None）。广播 enqueued/replaced。
    pub async fn enqueue_or_replace<H: QueueHost>(&self, host: &H, repo: &str, job: QueuedJob) -> Option<String> {
        let replaced = self.inner.lock().await.insert(repo.to_string(), job.clone());
        match replaced {
            Some(old) => {
                let label = old.label.clone();
                host.publish_event(BusEvent::QueueChanged {
                    repo: repo.to_string(),
                    change: QueueChange::Replaced { job, replaced_label: label.clone() },
                });
                Some(label)
            }
            None => {
                host.publish_event(BusEvent::QueueChanged {
                    repo: repo.to_string(),
                    change: QueueChange::Enqueued { job },
                });
                None
            }
        }
    }

    /// 单仓库排队项（GET handler 读）。
    pub async fn peek(&self, repo: &str) -> Option<QueuedJob> {
        self.inner.lock().await.get(repo).cloned()
    }

    /// 全部排队项（overview 读）。
    pub async fn all(&self) -> Vec<(String, QueuedJob)> {
        self.inner.lock().await.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.is_empty()
    }

    /// DELETE 取消：移除并广播 cancelled；无排队返回 None。
    pub async fn cancel<H: QueueHost>(&self, host: &H, repo: &str) -> Option<QueuedJob> {
        let removed = self.inner.lock().await.remove(repo);
        if let Some(job) = &removed {
            host.publish_event(BusEvent::QueueChanged {
                repo: repo.to_string(),
                change: QueueChange::Cancelled { job: job.clone() },
            });
        }
        removed
    }

    /// drain（§4.1）：「有排队 && 无活动会话」→ pop + tokio::spawn 执行（I2：本函数只做
    /// 判断 + pop，IO 不阻塞调用方——事件循环与清扫器共用）。返回 true 表示发生了 drain。
    pub async fn drain<H: QueueHost>(self: &Arc<Self>, host: &H, repo: &str) -> bool {
        // 复查活动会话（防御：终态事件后可能已有新会话抢注，如任务槽——活动则跳过等下一终态）
        if host.has_active_session(repo).await {
            return false;
        }
        let job = match self.inner.lock().await.remove(repo) {
            Some(j) => j,
            None => return false,
        };
        host.publish_event(BusEvent::QueueChanged {
            repo: repo.to_string(),
            change: QueueChange::Drained { job: job.clone(), started: true },
        });
        let queue = self.clone();
        let host = host.clone();
        let repo = repo.to_string();
        tokio::spawn(async move {
            if let Err(e) = host.run_job(&repo, &job).await {
                queue.handle_failure(&host, &repo, job, e).await;
            }
        });
        true
    }

    /// drain 失败分级（B2）：
    /// - Conflict（TOCTOU：终态后任务槽等抢注了活动会话）→ 放回原槽位，**不覆盖**期间用户新排的队，等下一触发
    /// - 其他确定性失败 → 丢弃 + 广播 queue.changed{type:"failed", error}（不留死信，禁止只留 warn 日志）
    pub async fn handle_failure<H: QueueHost>(&self, host: &H, repo: &str, job: QueuedJob, err: ApiError) {
        match err {
            ApiError::Conflict(_) => {
                tracing::warn!("[queue] {} 排队任务「{}」执行撞 Conflict（TOCTOU），放回原槽位等下一触发", repo, job.label);
                self.inner.lock().await.entry(repo.to_string()).or_insert(job.clone());
                host.publish_event(BusEvent::QueueChanged {
                    repo: repo.to_string(),
                    change: QueueChange::Requeued { job },
                });
            }
            other => {
                tracing::warn!("[queue] {} 排队任务「{}」启动失败，丢弃: {}", repo, job.label, other);
                host.publish_event(BusEvent::QueueChanged {
                    repo: repo.to_string(),
                    change: QueueChange::Failed { job, error: other.to_string() },
                });
            }
        }
    }

    /// 周期清扫器（B3）：12s 一拍，凡「有排队 && 无活动会话」的仓库触发 drain——
    /// 事件驱动降级为「事件加速 + 周期校对」，覆盖 try_send 背压丢弃/任务槽终态/超时 kill/spawn 失败等漏事件路径
    pub async fn sweep_loop<H: QueueHost>(self: Arc<Self>, host: H) {
        let mut tick = tokio::time::interval(Duration::from_secs(12));
        loop {
            tick.tick().await;
            let repos: Vec<String> = self.inner.lock().await.keys().cloned().collect();
            for repo in repos {
                self.drain(&host, &repo).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_change_payload_carries_type_and_extras() {
        let job = QueuedJob { kind: JobKind::Reinduce, module_id: None, label: "归纳".into(), enqueued_at: Utc::now() };
        let p = QueueChange::Enqueued { job: job.clone() }.to_payload("r1");
        assert_eq!(p["type"], "enqueued");
        assert_eq!(p["job"]["kind"], "reinduce");
        assert!(p.get("started").is_none());
        let p = QueueChange::Drained { job: job.clone(), started: true }.to_payload("r1");
        assert_eq!(p["type"], "drained");
        assert_eq!(p["started"], true);
        let p = QueueChange::Replaced { job: job.clone(), replaced_label: "巡检".into() }.to_payload("r1");
        assert_eq!(p["replacedLabel"], "巡检");
        let p = QueueChange::Failed { job, error: "agent 缺失".into() }.to_payload("r1");
        assert_eq!(p["type"], "failed");
        assert_eq!(p["error"], "agent 缺失");
    }
}
