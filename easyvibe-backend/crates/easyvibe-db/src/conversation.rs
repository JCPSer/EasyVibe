//! conversation 聚合域：会话+消息持久化/分页回放/压缩水位/token 累计。
use easyvibe_common::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::core::{db_err, now_secs};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationRow {
    pub id: String,
    pub repo: String,
    pub summary: Option<String>,
    pub compacted_before: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub created_at: String,
    pub updated_at: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessageRow {
    pub id: i64,
    pub conversation_id: String,
    pub role: String, // user / assistant / system
    pub content: String,
    pub compacted: bool,
    pub tokens: i64,
    pub created_at: String,
}

pub trait ConversationRepository: Send + Sync {
    /// 每仓库一个会话（入口对话），不存在则创建
    fn get_or_create(&self, repo: &str) -> impl std::future::Future<Output = Result<ConversationRow, ApiError>> + Send;
    /// 追加一条消息，返回 rowid（用于压缩水位）
    fn append_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        tokens: i64,
    ) -> impl std::future::Future<Output = Result<i64, ApiError>> + Send;
    /// 回放从库读（R1 分页：before_id 之前的一页，升序返回；None=最新一页）
    fn list_messages(
        &self,
        conversation_id: &str,
        before_id: Option<i64>,
        limit: i64,
    ) -> impl std::future::Future<Output = Result<Vec<ConversationMessageRow>, ApiError>> + Send;
    /// 运行态上下文：仅未压缩消息（近期窗口原文保留）
    fn list_uncompacted(
        &self,
        conversation_id: &str,
    ) -> impl std::future::Future<Output = Result<Vec<ConversationMessageRow>, ApiError>> + Send;
    /// 压缩执行：水位之前的消息标 compacted（原文保留），摘要与累计 token 更新
    fn apply_compaction(
        &self,
        conversation_id: &str,
        before_id: i64,
        summary: &str,
        prompt_tokens_add: i64,
        completion_tokens_add: i64,
    ) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// token 用量累计（§10 #4 成本护栏第一步）
    fn add_tokens(
        &self,
        conversation_id: &str,
        prompt_tokens: i64,
        completion_tokens: i64,
    ) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// 清空会话（保留会话行，消息与摘要重置——"新对话"按钮的原料）
    fn reset(&self, conversation_id: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    // ---- M4-2 多会话 ----
    /// 该仓库全部会话（最近活跃在前）
    fn list_by_repo(&self, repo: &str) -> impl std::future::Future<Output = Result<Vec<ConversationRow>, ApiError>> + Send;
    /// 新建命名会话（id 由调用方生成）
    fn create(&self, id: &str, repo: &str, title: Option<&str>) -> impl std::future::Future<Output = Result<ConversationRow, ApiError>> + Send;
    fn rename(&self, conversation_id: &str, title: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    /// 删除会话（先删消息，FK 无级联）
    fn delete(&self, conversation_id: &str) -> impl std::future::Future<Output = Result<(), ApiError>> + Send;
    fn count_messages(&self, conversation_id: &str) -> impl std::future::Future<Output = Result<i64, ApiError>> + Send;
}

pub struct SqliteConversationRepository {
    pool: SqlitePool,
}

impl SqliteConversationRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct ConversationRowSql {
    id: String, repo: String, summary: Option<String>, compacted_before: i64,
    prompt_tokens: i64, completion_tokens: i64, created_at: String, updated_at: String,
    title: Option<String>,   // M4-2 多会话：用户可命名
}

impl From<ConversationRowSql> for ConversationRow {
    fn from(r: ConversationRowSql) -> Self {
        Self {
            id: r.id, repo: r.repo, summary: r.summary, compacted_before: r.compacted_before,
            prompt_tokens: r.prompt_tokens, completion_tokens: r.completion_tokens,
            created_at: r.created_at, updated_at: r.updated_at,
            title: r.title,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ConversationMessageRowSql {
    id: i64, conversation_id: String, role: String, content: String,
    compacted: i64, tokens: i64, created_at: String,
}

impl From<ConversationMessageRowSql> for ConversationMessageRow {
    fn from(r: ConversationMessageRowSql) -> Self {
        Self {
            id: r.id, conversation_id: r.conversation_id, role: r.role, content: r.content,
            compacted: r.compacted != 0, tokens: r.tokens, created_at: r.created_at,
        }
    }
}

impl ConversationRepository for SqliteConversationRepository {
    async fn get_or_create(&self, repo: &str) -> Result<ConversationRow, ApiError> {
        // M4-2 多会话：默认会话 = 该仓库最近活跃的会话（旧行为一仓一会话时即唯一会话），无则建 "chat:{repo}"
        if let Some(row) = sqlx::query_as::<_, ConversationRowSql>(
            "SELECT * FROM conversations WHERE repo = ? ORDER BY updated_at DESC LIMIT 1",
        ).bind(repo).fetch_optional(&self.pool).await.map_err(db_err)? {
            return Ok(row.into());
        }
        let id = format!("chat:{repo}");
        let now = now_secs();
        sqlx::query("INSERT INTO conversations (id, repo, created_at, updated_at) VALUES (?, ?, ?, ?)")
            .bind(&id).bind(repo).bind(&now).bind(&now)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(sqlx::query_as::<_, ConversationRowSql>("SELECT * FROM conversations WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool).await.map_err(db_err)?.into())
    }

    async fn list_by_repo(&self, repo: &str) -> Result<Vec<ConversationRow>, ApiError> {
        Ok(sqlx::query_as::<_, ConversationRowSql>("SELECT * FROM conversations WHERE repo = ? ORDER BY updated_at DESC")
            .bind(repo).fetch_all(&self.pool).await.map_err(db_err)?.into_iter().map(Into::into).collect())
    }

    async fn create(&self, id: &str, repo: &str, title: Option<&str>) -> Result<ConversationRow, ApiError> {
        let now = now_secs();
        sqlx::query("INSERT INTO conversations (id, repo, title, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
            .bind(id).bind(repo).bind(title).bind(&now).bind(&now)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(sqlx::query_as::<_, ConversationRowSql>("SELECT * FROM conversations WHERE id = ?")
            .bind(id).fetch_one(&self.pool).await.map_err(db_err)?.into())
    }

    async fn rename(&self, conversation_id: &str, title: &str) -> Result<(), ApiError> {
        sqlx::query("UPDATE conversations SET title = ?, updated_at = ? WHERE id = ?")
            .bind(title).bind(now_secs()).bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn delete(&self, conversation_id: &str) -> Result<(), ApiError> {
        sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = ?").bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        sqlx::query("DELETE FROM conversations WHERE id = ?").bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn count_messages(&self, conversation_id: &str) -> Result<i64, ApiError> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM conversation_messages WHERE conversation_id = ?")
            .bind(conversation_id).fetch_one(&self.pool).await.map_err(db_err)?;
        Ok(row.0)
    }

    async fn append_message(&self, conversation_id: &str, role: &str, content: &str, tokens: i64) -> Result<i64, ApiError> {
        let res = sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, tokens, created_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(conversation_id).bind(role).bind(content).bind(tokens).bind(now_secs())
        .execute(&self.pool).await.map_err(db_err)?;
        sqlx::query("UPDATE conversations SET updated_at = ? WHERE id = ?")
            .bind(now_secs()).bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(res.last_insert_rowid())
    }

    async fn list_messages(&self, conversation_id: &str, before_id: Option<i64>, limit: i64) -> Result<Vec<ConversationMessageRow>, ApiError> {
        // R1 清债：倒序取一页再翻回升序（before_id=None 取最新一页）
        let rows = match before_id {
            Some(b) => sqlx::query_as::<_, ConversationMessageRowSql>(
                "SELECT * FROM conversation_messages WHERE conversation_id = ? AND id < ? ORDER BY id DESC LIMIT ?",
            )
            .bind(conversation_id).bind(b).bind(limit)
            .fetch_all(&self.pool).await.map_err(db_err)?,
            None => sqlx::query_as::<_, ConversationMessageRowSql>(
                "SELECT * FROM conversation_messages WHERE conversation_id = ? ORDER BY id DESC LIMIT ?",
            )
            .bind(conversation_id).bind(limit)
            .fetch_all(&self.pool).await.map_err(db_err)?,
        };
        let mut rows: Vec<ConversationMessageRow> = rows.into_iter().map(Into::into).collect();
        rows.reverse();
        let _ = limit;
        Ok(rows)
    }

    async fn list_uncompacted(&self, conversation_id: &str) -> Result<Vec<ConversationMessageRow>, ApiError> {
        let rows = sqlx::query_as::<_, ConversationMessageRowSql>(
            "SELECT * FROM conversation_messages WHERE conversation_id = ? AND compacted = 0 ORDER BY id",
        )
        .bind(conversation_id)
        .fetch_all(&self.pool).await.map_err(db_err)?;
        Ok(rows.into_iter().map(Into::into).collect())
    }

    async fn apply_compaction(&self, conversation_id: &str, before_id: i64, summary: &str, pt_add: i64, ct_add: i64) -> Result<(), ApiError> {
        sqlx::query("UPDATE conversation_messages SET compacted = 1 WHERE conversation_id = ? AND id <= ?")
            .bind(conversation_id).bind(before_id)
            .execute(&self.pool).await.map_err(db_err)?;
        sqlx::query(
            "UPDATE conversations SET summary = ?, compacted_before = ?, prompt_tokens = prompt_tokens + ?, completion_tokens = completion_tokens + ?, updated_at = ? WHERE id = ?",
        )
        .bind(summary).bind(before_id).bind(pt_add).bind(ct_add).bind(now_secs()).bind(conversation_id)
        .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn add_tokens(&self, conversation_id: &str, prompt_tokens: i64, completion_tokens: i64) -> Result<(), ApiError> {
        sqlx::query(
            "UPDATE conversations SET prompt_tokens = prompt_tokens + ?, completion_tokens = completion_tokens + ?, updated_at = ? WHERE id = ?",
        )
        .bind(prompt_tokens).bind(completion_tokens).bind(now_secs()).bind(conversation_id)
        .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }

    async fn reset(&self, conversation_id: &str) -> Result<(), ApiError> {
        sqlx::query("DELETE FROM conversation_messages WHERE conversation_id = ?")
            .bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        sqlx::query("UPDATE conversations SET summary = NULL, compacted_before = 0, prompt_tokens = 0, completion_tokens = 0, updated_at = ? WHERE id = ?")
            .bind(now_secs()).bind(conversation_id)
            .execute(&self.pool).await.map_err(db_err)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    #[tokio::test]
    async fn conversation_messages_paginate() {
        // R1 清债：回放分页（先爆热区第一名的回归网）
        let db = Database::connect_memory().await.unwrap();
        let repo = SqliteConversationRepository::new(db.pool().clone());
        let conv = repo.get_or_create("demo").await.unwrap();
        for i in 0..60 {
            repo.append_message(&conv.id, "user", &format!("msg-{i}"), 1).await.unwrap();
        }
        let page1 = repo.list_messages(&conv.id, None, 50).await.unwrap();
        assert_eq!(page1.len(), 50);
        assert_eq!(page1[0].content, "msg-10", "最旧的在 60-50=10");
        let page2 = repo.list_messages(&conv.id, Some(page1[0].id), 50).await.unwrap();
        assert_eq!(page2.len(), 10, "第二页应 10 条");
        assert_eq!(page2[0].content, "msg-0");
        // 衔接：page2 最后一条 id < page1 第一条 id
        assert!(page2.last().unwrap().id < page1.first().unwrap().id);
    }

    #[tokio::test]
    async fn conversation_roundtrip_and_compaction() {
        let db = Database::connect_memory().await.unwrap();
        let repo = SqliteConversationRepository::new(db.pool().clone());
        let conv = repo.get_or_create("demo").await.unwrap();
        assert_eq!(conv.id, "chat:demo");

        let m1 = repo.append_message(&conv.id, "user", "问题一", 10).await.unwrap();
        let _m2 = repo.append_message(&conv.id, "assistant", "回答一", 20).await.unwrap();
        let m3 = repo.append_message(&conv.id, "user", "问题二", 10).await.unwrap();
        let _m4 = repo.append_message(&conv.id, "assistant", "回答二", 20).await.unwrap();

        // 未压缩水位前：运行态仅见未压缩消息
        repo.apply_compaction(&conv.id, m3 - 1, "摘要：决策 X；未决问题 Y", 100, 50).await.unwrap();
        let all = repo.list_messages(&conv.id, None, 50).await.unwrap();
        assert_eq!(all.len(), 4, "原文全部保留（留痕可回放）");
        assert!(all.iter().take(2).all(|m| m.compacted));
        assert!(!all[2].compacted, "近期窗口原文保留");
        let fresh = repo.list_uncompacted(&conv.id).await.unwrap();
        assert_eq!(fresh.len(), 2);

        // token 累计
        repo.add_tokens(&conv.id, 30, 15).await.unwrap();
        let after = repo.get_or_create("demo").await.unwrap();
        assert_eq!(after.prompt_tokens, 130);
        assert_eq!(after.completion_tokens, 65);
        assert!(after.summary.as_deref().unwrap().contains("未决问题 Y"), "摘要须携带会话状态（§11 🟡6）");

        // 重置（新对话）
        repo.reset(&conv.id).await.unwrap();
        assert!(repo.list_messages(&conv.id, None, 50).await.unwrap().is_empty());
        let clean = repo.get_or_create("demo").await.unwrap();
        assert_eq!(clean.prompt_tokens, 0);
        assert!(clean.summary.is_none());
    }
}
