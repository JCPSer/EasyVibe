//! 会话域端口适配（c-arch-13 R1）。

use super::dto::{Conversation, Message};
use easyvibe_common::ApiError;

#[async_trait::async_trait]
pub(crate) trait ConversationPort {
    async fn get_or_create(&self, repo: &str) -> Result<Conversation, ApiError>;
    async fn list_by_repo(&self, repo: &str) -> Result<Vec<Conversation>, ApiError>;
    async fn create(&self, id: &str, repo: &str, title: Option<&str>) -> Result<Conversation, ApiError>;
    async fn rename(&self, conversation_id: &str, title: &str) -> Result<(), ApiError>;
    async fn delete(&self, conversation_id: &str) -> Result<(), ApiError>;
    async fn reset(&self, conversation_id: &str) -> Result<(), ApiError>;
    async fn list_messages(&self, conversation_id: &str, before_id: Option<i64>, limit: i64) -> Result<Vec<Message>, ApiError>;
    async fn count_messages(&self, conversation_id: &str) -> Result<i64, ApiError>;
    async fn list_uncompacted(&self, conversation_id: &str) -> Result<Vec<Message>, ApiError>;
    async fn append_message(&self, conversation_id: &str, role: &str, content: &str, tokens: i64) -> Result<i64, ApiError>;
    async fn add_tokens(&self, conversation_id: &str, prompt_tokens: i64, completion_tokens: i64) -> Result<(), ApiError>;
    async fn apply_compaction(&self, conversation_id: &str, before_id: i64, summary: &str, prompt_tokens: i64, completion_tokens: i64) -> Result<(), ApiError>;
}

fn into_conversation(r: easyvibe_db::ConversationRow) -> Conversation {
    Conversation {
        id: r.id, repo: r.repo, summary: r.summary, prompt_tokens: r.prompt_tokens,
        completion_tokens: r.completion_tokens, created_at: r.created_at, updated_at: r.updated_at, title: r.title,
    }
}

fn into_message(r: easyvibe_db::ConversationMessageRow) -> Message {
    Message {
        id: r.id, conversation_id: r.conversation_id, role: r.role, content: r.content,
        compacted: r.compacted, tokens: r.tokens, created_at: r.created_at,
    }
}

#[async_trait::async_trait]
impl ConversationPort for easyvibe_db::SqliteConversationRepository {
    async fn get_or_create(&self, repo: &str) -> Result<Conversation, ApiError> {
        Ok(into_conversation(easyvibe_db::ConversationRepository::get_or_create(self, repo).await?))
    }
    async fn list_by_repo(&self, repo: &str) -> Result<Vec<Conversation>, ApiError> {
        Ok(easyvibe_db::ConversationRepository::list_by_repo(self, repo).await?.into_iter().map(into_conversation).collect())
    }
    async fn create(&self, id: &str, repo: &str, title: Option<&str>) -> Result<Conversation, ApiError> {
        Ok(into_conversation(easyvibe_db::ConversationRepository::create(self, id, repo, title).await?))
    }
    async fn rename(&self, conversation_id: &str, title: &str) -> Result<(), ApiError> {
        easyvibe_db::ConversationRepository::rename(self, conversation_id, title).await
    }
    async fn delete(&self, conversation_id: &str) -> Result<(), ApiError> {
        easyvibe_db::ConversationRepository::delete(self, conversation_id).await
    }
    async fn reset(&self, conversation_id: &str) -> Result<(), ApiError> {
        easyvibe_db::ConversationRepository::reset(self, conversation_id).await
    }
    async fn list_messages(&self, conversation_id: &str, before_id: Option<i64>, limit: i64) -> Result<Vec<Message>, ApiError> {
        Ok(easyvibe_db::ConversationRepository::list_messages(self, conversation_id, before_id, limit).await?.into_iter().map(into_message).collect())
    }
    async fn count_messages(&self, conversation_id: &str) -> Result<i64, ApiError> {
        easyvibe_db::ConversationRepository::count_messages(self, conversation_id).await
    }
    async fn list_uncompacted(&self, conversation_id: &str) -> Result<Vec<Message>, ApiError> {
        Ok(easyvibe_db::ConversationRepository::list_uncompacted(self, conversation_id).await?.into_iter().map(into_message).collect())
    }
    async fn append_message(&self, conversation_id: &str, role: &str, content: &str, tokens: i64) -> Result<i64, ApiError> {
        easyvibe_db::ConversationRepository::append_message(self, conversation_id, role, content, tokens).await
    }
    async fn add_tokens(&self, conversation_id: &str, prompt_tokens: i64, completion_tokens: i64) -> Result<(), ApiError> {
        easyvibe_db::ConversationRepository::add_tokens(self, conversation_id, prompt_tokens, completion_tokens).await
    }
    async fn apply_compaction(&self, conversation_id: &str, before_id: i64, summary: &str, prompt_tokens: i64, completion_tokens: i64) -> Result<(), ApiError> {
        easyvibe_db::ConversationRepository::apply_compaction(self, conversation_id, before_id, summary, prompt_tokens, completion_tokens).await
    }
}
