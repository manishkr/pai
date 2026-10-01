use diesel::prelude::*;
use shared::{ConversationSummary, MessageRecord, MessageRole};
use uuid::Uuid;

use crate::schema::{auth_sessions, conversations, messages, users};

#[derive(Debug, Insertable)]
#[diesel(table_name = users)]
pub struct NewUser<'a> {
    pub id: Uuid,
    pub google_sub: &'a str,
    pub email: &'a str,
    pub name: &'a str,
    pub avatar_url: Option<&'a str>,
    pub hosted_domain: &'a str,
    pub created_at: i64,
    pub last_login_at: i64,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = auth_sessions)]
pub struct NewAuthSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub expires_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = conversations)]
pub struct ConversationRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = conversations)]
pub struct NewConversation<'a> {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: &'a str,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = messages)]
pub struct MessageRow {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub role: String,
    pub content: String,
    pub created_at: i64,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = messages)]
pub struct NewMessage<'a> {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub role: &'a str,
    pub content: &'a str,
    pub created_at: i64,
}

impl From<ConversationRow> for ConversationSummary {
    fn from(value: ConversationRow) -> Self {
        Self {
            id: value.id,
            title: value.title,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl TryFrom<MessageRow> for MessageRecord {
    type Error = String;

    fn try_from(value: MessageRow) -> Result<Self, Self::Error> {
        let role = MessageRole::try_from(value.role.as_str())?;
        Ok(Self {
            id: value.id,
            conversation_id: value.conversation_id,
            role,
            content: value.content,
            created_at: value.created_at,
        })
    }
}
