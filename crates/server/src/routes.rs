use std::convert::Infallible;
use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use diesel::prelude::*;
use futures_util::StreamExt;
use shared::{
    ChatStreamCompleted, ChatStreamError, ChatStreamEvent, ChatStreamStarted, ChatStreamToken,
    ConversationMessagesResponse, ConversationSummary, CreateConversationResponse, MeResponse,
    MessageRecord, MessageRole,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tracing::{error, instrument};
use uuid::Uuid;

use crate::anthropic::AnthropicStreamEvent;
use crate::app_shell;
use crate::auth::{
    AuthContext, GoogleCallbackQuery, begin_google_login, clear_auth_cookies, clear_oauth_cookies,
    complete_google_login, logout_request, session_cookie,
};
use crate::documents::{allowed_document_id_set, inject_documents_into_prompt};
use crate::error::AppError;
use crate::models::{ConversationRow, MessageRow, NewConversation, NewMessage};
use crate::schema::{conversations, messages};
use crate::state::{AppState, json_event, now_epoch_seconds};

#[derive(Debug, serde::Deserialize)]
pub struct CreateConversationRequest {
    pub title: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct ChatStreamQuery {
    pub conversation_id: Uuid,
    pub message: String,
    pub document_ids: Option<String>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/style.css", get(style_css))
        .route("/app.js", get(app_js))
        .route("/auth/google/start", get(auth_google_start))
        .route("/auth/google/callback", get(auth_google_callback))
        .route("/auth/logout", post(auth_logout))
        .route("/api/me", get(get_me))
        .route("/api/documents", get(list_documents))
        .route(
            "/api/conversations",
            post(create_conversation).get(list_conversations),
        )
        .route("/api/conversations/{id}/messages", get(get_messages))
        .route("/api/chat/stream", get(chat_stream))
        .with_state(state)
}

async fn index(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Html(app_shell(&state.config.public_origin))
}

async fn style_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../../../style.css"),
    )
}

async fn app_js() -> impl IntoResponse {
    (
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../../../app.js"),
    )
}

async fn auth_google_start(State(state): State<Arc<AppState>>) -> Result<Response, AppError> {
    let start = begin_google_login(&state.config)?;

    let mut response = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, start.authorization_url)
        .body(axum::body::Body::empty())
        .map_err(AppError::internal)?;

    for cookie in start.set_cookies {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }

    Ok(response)
}

async fn auth_google_callback(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<GoogleCallbackQuery>,
) -> Result<Response, AppError> {
    let completed = complete_google_login(state.clone(), &headers, query).await?;
    let session_cookie = session_cookie(&state.config, completed.auth_session_id)?;
    let clear_cookies = clear_oauth_cookies(&state.config)?;

    let mut response = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/")
        .body(axum::body::Body::empty())
        .map_err(AppError::internal)?;

    response
        .headers_mut()
        .append(header::SET_COOKIE, session_cookie);
    for cookie in clear_cookies {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }

    Ok(response)
}

async fn auth_logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    logout_request(state.clone(), &headers).await?;

    let mut response = StatusCode::NO_CONTENT.into_response();
    for cookie in clear_auth_cookies(&state.config)? {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }

    Ok(response)
}

#[instrument(skip_all, fields(user_id = %auth.user_id))]
async fn get_me(
    State(state): State<Arc<AppState>>,
    auth: AuthContext,
) -> Result<Json<MeResponse>, AppError> {
    state
        .touch_session(&auth.auth_session_id, &auth.user_id)
        .await
        .map_err(AppError::internal)?;

    let catalog = state
        .documents
        .load_catalog()
        .await
        .map_err(AppError::internal)?;
    let document_access = state
        .config
        .document_access_rules
        .resolve_for_email(&catalog, &auth.email);

    Ok(Json(MeResponse {
        user: auth.to_shared_user(),
        document_access,
    }))
}

#[instrument(skip_all, fields(user_id = %auth.user_id))]
async fn list_documents(
    State(state): State<Arc<AppState>>,
    auth: AuthContext,
) -> Result<Json<Vec<crate::documents::DocumentTreeNode>>, AppError> {
    state
        .touch_session(&auth.auth_session_id, &auth.user_id)
        .await
        .map_err(AppError::internal)?;

    let catalog = state
        .documents
        .load_catalog()
        .await
        .map_err(AppError::internal)?;
    let access = state
        .config
        .document_access_rules
        .resolve_for_email(&catalog, &auth.email);
    let allowed_document_ids = allowed_document_id_set(&access);
    Ok(Json(catalog.filter_document_tree(&allowed_document_ids)))
}

#[instrument(skip_all, fields(user_id = %auth.user_id))]
async fn create_conversation(
    State(state): State<Arc<AppState>>,
    auth: AuthContext,
    Json(payload): Json<CreateConversationRequest>,
) -> Result<Json<CreateConversationResponse>, AppError> {
    state
        .touch_session(&auth.auth_session_id, &auth.user_id)
        .await
        .map_err(AppError::internal)?;

    let title = payload
        .title
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| "New chat".to_string());

    let created = state
        .run_db("create conversation", {
            let user_id = auth.user_id;
            let title = title.clone();
            move |connection| {
                let now = now_epoch_seconds();
                let conversation = NewConversation {
                    id: Uuid::new_v4(),
                    user_id,
                    title: &title,
                    created_at: now,
                    updated_at: now,
                };

                diesel::insert_into(conversations::table)
                    .values(&conversation)
                    .returning(ConversationRow::as_returning())
                    .get_result::<ConversationRow>(connection)
                    .map(ConversationSummary::from)
                    .map_err(|error| AppError::internal(format!("create conversation: {error}")))
            }
        })
        .await?;

    Ok(Json(CreateConversationResponse {
        conversation: created,
    }))
}

#[instrument(skip_all, fields(user_id = %auth.user_id))]
async fn list_conversations(
    State(state): State<Arc<AppState>>,
    auth: AuthContext,
) -> Result<Json<Vec<ConversationSummary>>, AppError> {
    state
        .touch_session(&auth.auth_session_id, &auth.user_id)
        .await
        .map_err(AppError::internal)?;

    let list = state
        .run_db("list conversations", {
            let user_id = auth.user_id;
            move |connection| {
                conversations::table
                    .filter(conversations::user_id.eq(user_id))
                    .order(conversations::updated_at.desc())
                    .select(ConversationRow::as_select())
                    .load::<ConversationRow>(connection)
                    .map(|rows| {
                        rows.into_iter()
                            .map(ConversationSummary::from)
                            .collect::<Vec<_>>()
                    })
                    .map_err(|error| AppError::internal(format!("list conversations: {error}")))
            }
        })
        .await?;

    Ok(Json(list))
}

#[instrument(skip_all, fields(user_id = %auth.user_id, conversation_id = %id))]
async fn get_messages(
    State(state): State<Arc<AppState>>,
    auth: AuthContext,
    Path(id): Path<Uuid>,
) -> Result<Json<ConversationMessagesResponse>, AppError> {
    state
        .touch_session(&auth.auth_session_id, &auth.user_id)
        .await
        .map_err(AppError::internal)?;

    if let Some(cached) = state
        .load_cached_messages(&auth.user_id, id)
        .await
        .map_err(AppError::internal)?
    {
        return Ok(Json(ConversationMessagesResponse { messages: cached }));
    }

    let messages = load_owned_messages(state.clone(), auth.user_id, id).await?;
    state
        .cache_messages(&auth.user_id, id, &messages)
        .await
        .map_err(AppError::internal)?;

    Ok(Json(ConversationMessagesResponse { messages }))
}

#[instrument(skip_all, fields(user_id = %auth.user_id, conversation_id = %query.conversation_id))]
async fn chat_stream(
    State(state): State<Arc<AppState>>,
    auth: AuthContext,
    Query(query): Query<ChatStreamQuery>,
) -> Result<Response, AppError> {
    let prompt = query.message.trim().to_string();
    if prompt.is_empty() {
        return Err(AppError::Validation("message cannot be empty".to_string()));
    }

    state
        .touch_session(&auth.auth_session_id, &auth.user_id)
        .await
        .map_err(AppError::internal)?;

    ensure_conversation_belongs_to_user(state.clone(), auth.user_id, query.conversation_id).await?;

    let catalog = state
        .documents
        .load_catalog()
        .await
        .map_err(AppError::internal)?;
    let access = state
        .config
        .document_access_rules
        .resolve_for_email(&catalog, &auth.email);
    let allowed_document_ids = allowed_document_id_set(&access);
    let selected_document_ids = parse_document_ids(query.document_ids.as_deref());
    let unauthorized_document_requested = selected_document_ids
        .iter()
        .any(|document_id| !allowed_document_ids.contains(document_id));
    if unauthorized_document_requested {
        return Err(AppError::Forbidden("document access is not allowed"));
    }

    let assistant_message_id = Uuid::new_v4();
    let conversation_id = query.conversation_id;
    let user_id = auth.user_id;
    let prompt_for_db = prompt.clone();
    let prompt_for_title = prompt.clone();
    let selected_documents = catalog.find_documents(&selected_document_ids, &allowed_document_ids);

    state
        .run_db("persist user message", move |connection| {
            let now = now_epoch_seconds();
            let user_message = NewMessage {
                id: Uuid::new_v4(),
                conversation_id,
                role: MessageRole::User.as_str(),
                content: &prompt_for_db,
                created_at: now,
            };

            diesel::insert_into(messages::table)
                .values(&user_message)
                .execute(connection)
                .map_err(|error| AppError::internal(format!("persist user message: {error}")))?;

            let conversation = conversations::table
                .find(conversation_id)
                .select(ConversationRow::as_select())
                .get_result::<ConversationRow>(connection)
                .map_err(|error| {
                    AppError::internal(format!("load conversation for title update: {error}"))
                })?;

            let title = if conversation.title == "New chat" {
                shared::derive_conversation_title(&prompt_for_title)
            } else {
                conversation.title
            };

            diesel::update(conversations::table.find(conversation_id))
                .set((
                    conversations::title.eq(title),
                    conversations::updated_at.eq(now),
                ))
                .execute(connection)
                .map_err(|error| {
                    AppError::internal(format!("update conversation metadata: {error}"))
                })?;

            Ok(())
        })
        .await?;

    let history = load_owned_messages(state.clone(), user_id, conversation_id).await?;
    let llm_history = contextualize_history_with_documents(history.clone(), &selected_documents);
    state
        .cache_messages(&user_id, conversation_id, &history)
        .await
        .map_err(AppError::internal)?;

    let active_key = state
        .begin_active_stream(&user_id, conversation_id, assistant_message_id)
        .await
        .map_err(AppError::internal)?;

    let (tx, rx) = mpsc::channel::<Result<axum::response::sse::Event, Infallible>>(32);
    let state_for_task = state.clone();

    tokio::spawn(async move {
        let started = ChatStreamEvent::Started(ChatStreamStarted {
            conversation_id,
            assistant_message_id,
        });

        if tx
            .send(Ok(
                json_event(&started).expect("stream start event should serialize")
            ))
            .await
            .is_err()
        {
            return;
        }

        let stream_result: anyhow::Result<()> = state_for_task
            .anthropic
            .stream_messages(llm_history, |event| {
                Box::pin({
                    let state_for_task = state_for_task.clone();
                    let active_key = active_key.clone();
                    let tx = tx.clone();
                    async move {
                        match event {
                            AnthropicStreamEvent::Started => Ok(()),
                            AnthropicStreamEvent::TextDelta(delta) => {
                                state_for_task
                                    .append_stream_chunk(&active_key, &delta)
                                    .await?;

                                let payload = ChatStreamEvent::Token(ChatStreamToken {
                                    conversation_id,
                                    assistant_message_id,
                                    text: delta,
                                });
                                let event = json_event(&payload)?;
                                tx.send(Ok(event))
                                    .await
                                    .map_err(|error| anyhow::anyhow!(error))
                                    .context("while attempting to forward a token event")?;
                                Ok(())
                            }
                            AnthropicStreamEvent::Completed => Ok(()),
                        }
                    }
                })
            })
            .await;

        match stream_result {
            Ok(()) => {
                let finalize = async {
                    let assistant_text = state_for_task.finish_active_stream(&active_key).await?;

                    state_for_task
                        .run_db("persist assistant message", move |connection| {
                            let now = now_epoch_seconds();
                            let assistant_message = NewMessage {
                                id: assistant_message_id,
                                conversation_id,
                                role: MessageRole::Assistant.as_str(),
                                content: &assistant_text,
                                created_at: now,
                            };

                            diesel::insert_into(messages::table)
                                .values(&assistant_message)
                                .execute(connection)
                                .map_err(|error| {
                                    AppError::internal(format!(
                                        "persist assistant message: {error}"
                                    ))
                                })?;

                            diesel::update(conversations::table.find(conversation_id))
                                .set(conversations::updated_at.eq(now))
                                .execute(connection)
                                .map_err(|error| {
                                    AppError::internal(format!(
                                        "touch conversation after assistant reply: {error}"
                                    ))
                                })?;

                            Ok(())
                        })
                        .await?;

                    let messages =
                        load_owned_messages(state_for_task.clone(), user_id, conversation_id)
                            .await?;
                    state_for_task
                        .cache_messages(&user_id, conversation_id, &messages)
                        .await
                        .map_err(AppError::internal)?;

                    let payload = ChatStreamEvent::Completed(ChatStreamCompleted {
                        conversation_id,
                        assistant_message_id,
                    });
                    let event = json_event(&payload)?;
                    tx.send(Ok(event)).await.map_err(|error| {
                        AppError::internal(format!("forward completion event: {error}"))
                    })?;

                    Result::<(), AppError>::Ok(())
                }
                .await;

                if let Err(error) = finalize {
                    error!(?error, "assistant stream finalize failed");
                    let payload = ChatStreamEvent::Error(ChatStreamError {
                        message: error.to_string(),
                    });
                    let _ = tx
                        .send(Ok(
                            json_event(&payload).expect("error event should serialize")
                        ))
                        .await;
                }
            }
            Err(error) => {
                error!(?error, "anthropic stream failed");
                let _ = state_for_task.abandon_active_stream(&active_key).await;
                let payload = ChatStreamEvent::Error(ChatStreamError {
                    message: error.to_string(),
                });
                let _ = tx
                    .send(Ok(
                        json_event(&payload).expect("error event should serialize")
                    ))
                    .await;
            }
        }
    });

    let stream = ReceiverStream::new(rx).map(|event| event);
    let sse = Sse::new(stream).keep_alive(KeepAlive::default());
    Ok(sse.into_response())
}

fn parse_document_ids(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

fn contextualize_history_with_documents(
    mut history: Vec<MessageRecord>,
    documents: &[crate::documents::DocumentRecord],
) -> Vec<MessageRecord> {
    if documents.is_empty() {
        return history;
    }

    if let Some(last_user_message) = history
        .iter_mut()
        .rev()
        .find(|message| matches!(message.role, MessageRole::User))
    {
        last_user_message.content =
            inject_documents_into_prompt(&last_user_message.content, documents);
    }

    history
}

async fn ensure_conversation_belongs_to_user(
    state: Arc<AppState>,
    user_id: Uuid,
    conversation_id: Uuid,
) -> Result<(), AppError> {
    state
        .run_db("authorize conversation", move |connection| {
            let exists = conversations::table
                .filter(conversations::id.eq(conversation_id))
                .filter(conversations::user_id.eq(user_id))
                .select(ConversationRow::as_select())
                .first::<ConversationRow>(connection)
                .optional()
                .map_err(|error| AppError::internal(format!("authorize conversation: {error}")))?;

            exists.map(|_| ()).ok_or(AppError::NotFound("conversation"))
        })
        .await
}

async fn load_owned_messages(
    state: Arc<AppState>,
    user_id: Uuid,
    conversation_id: Uuid,
) -> Result<Vec<MessageRecord>, AppError> {
    ensure_conversation_belongs_to_user(state.clone(), user_id, conversation_id).await?;

    state
        .run_db("load messages", move |connection| {
            messages::table
                .inner_join(
                    conversations::table.on(messages::conversation_id.eq(conversations::id)),
                )
                .filter(messages::conversation_id.eq(conversation_id))
                .filter(conversations::user_id.eq(user_id))
                .select(MessageRow::as_select())
                .order(messages::created_at.asc())
                .load::<MessageRow>(connection)
                .map_err(|error| AppError::internal(format!("load messages: {error}")))?
                .into_iter()
                .map(|row| MessageRecord::try_from(row).map_err(AppError::Validation))
                .collect()
        })
        .await
}
