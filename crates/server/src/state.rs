use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use bb8::Pool;
use bb8_redis::RedisConnectionManager;
use diesel::pg::PgConnection;
use diesel::r2d2::{ConnectionManager, Pool as DieselPool};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness, embed_migrations};
use redis::AsyncCommands;
use shared::MessageRecord;
use tokio::task;
use tracing::info;

use crate::anthropic::AnthropicClient;
use crate::config::AppConfig;
use crate::documents::DocumentService;
use crate::error::AppError;

pub const MIGRATIONS: EmbeddedMigrations = embed_migrations!("../../migrations");
pub const AUTH_SESSION_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub postgres: DieselPool<ConnectionManager<PgConnection>>,
    pub redis: Pool<RedisConnectionManager>,
    pub anthropic: AnthropicClient,
    pub documents: DocumentService,
}

impl AppState {
    pub async fn build(config: AppConfig) -> Result<Arc<Self>> {
        let postgres = build_postgres_pool(&config)?;
        run_migrations(postgres.clone()).await?;

        let redis_manager = RedisConnectionManager::new(config.redis_url.clone())
            .context("while attempting to construct the Redis manager")?;
        let redis = Pool::builder()
            .build(redis_manager)
            .await
            .context("while attempting to build the Redis pool")?;

        let state = Arc::new(Self {
            anthropic: AnthropicClient::new(&config),
            documents: DocumentService::new(config.sharepoint_graph.clone()),
            config,
            postgres,
            redis,
        });

        info!("application state initialized");
        Ok(state)
    }

    pub async fn touch_session(
        &self,
        auth_session_id: &uuid::Uuid,
        user_id: &uuid::Uuid,
    ) -> Result<()> {
        let mut connection = self
            .redis
            .get()
            .await
            .context("while attempting to checkout a Redis connection")?;
        let key = format!("auth-session:{auth_session_id}:user:{user_id}:metadata");
        let payload = serde_json::json!({
            "auth_session_id": auth_session_id,
            "user_id": user_id,
            "touched_at": now_epoch_seconds(),
        })
        .to_string();

        let _: () = connection
            .set_ex(key, payload, AUTH_SESSION_TTL_SECS)
            .await
            .context("while attempting to store session metadata in Redis")?;
        Ok(())
    }

    pub async fn cache_messages(
        &self,
        user_id: &uuid::Uuid,
        conversation_id: uuid::Uuid,
        messages: &[MessageRecord],
    ) -> Result<()> {
        let mut connection = self
            .redis
            .get()
            .await
            .context("while attempting to checkout a Redis connection")?;
        let key = format!("user:{user_id}:conversation:{conversation_id}:messages");
        let payload =
            serde_json::to_string(messages).context("while attempting to serialize messages")?;

        let _: () = connection
            .set_ex(key, payload, AUTH_SESSION_TTL_SECS)
            .await
            .context("while attempting to cache conversation history in Redis")?;
        Ok(())
    }

    pub async fn load_cached_messages(
        &self,
        user_id: &uuid::Uuid,
        conversation_id: uuid::Uuid,
    ) -> Result<Option<Vec<MessageRecord>>> {
        let mut connection = self
            .redis
            .get()
            .await
            .context("while attempting to checkout a Redis connection")?;
        let key = format!("user:{user_id}:conversation:{conversation_id}:messages");
        let payload: Option<String> = connection
            .get(key)
            .await
            .context("while attempting to load cached conversation history")?;

        payload
            .map(|value| {
                serde_json::from_str(&value)
                    .context("while attempting to deserialize cached messages")
            })
            .transpose()
    }

    pub async fn begin_active_stream(
        &self,
        user_id: &uuid::Uuid,
        conversation_id: uuid::Uuid,
        assistant_message_id: uuid::Uuid,
    ) -> Result<String> {
        let key =
            format!("user:{user_id}:conversation:{conversation_id}:active:{assistant_message_id}");
        let mut connection = self
            .redis
            .get()
            .await
            .context("while attempting to checkout a Redis connection")?;

        let _: () = connection
            .set_ex(&key, "", AUTH_SESSION_TTL_SECS)
            .await
            .context("while attempting to initialize active stream state in Redis")?;
        Ok(key)
    }

    pub async fn append_stream_chunk(&self, key: &str, chunk: &str) -> Result<()> {
        let mut connection = self
            .redis
            .get()
            .await
            .context("while attempting to checkout a Redis connection")?;

        let _: usize = connection
            .append(key, chunk)
            .await
            .context("while attempting to append a streaming chunk in Redis")?;
        let _: bool = connection
            .expire(
                key,
                i64::try_from(AUTH_SESSION_TTL_SECS).expect("TTL fits in i64"),
            )
            .await
            .context("while attempting to refresh the streaming TTL")?;
        Ok(())
    }

    pub async fn finish_active_stream(&self, key: &str) -> Result<String> {
        let mut connection = self
            .redis
            .get()
            .await
            .context("while attempting to checkout a Redis connection")?;
        let text: Option<String> = connection
            .get(key)
            .await
            .context("while attempting to read the completed stream from Redis")?;
        let _: usize = connection
            .del(key)
            .await
            .context("while attempting to delete the active stream key")?;

        Ok(text.unwrap_or_default())
    }

    pub async fn abandon_active_stream(&self, key: &str) -> Result<()> {
        let mut connection = self
            .redis
            .get()
            .await
            .context("while attempting to checkout a Redis connection")?;
        let _: usize = connection
            .del(key)
            .await
            .context("while attempting to delete the failed active stream key")?;
        Ok(())
    }

    pub async fn run_db<F, T>(&self, task_name: &'static str, operation: F) -> Result<T, AppError>
    where
        F: FnOnce(&mut PgConnection) -> Result<T, AppError> + Send + 'static,
        T: Send + 'static,
    {
        let pool = self.postgres.clone();
        task::spawn_blocking(move || {
            let mut connection = pool
                .get()
                .map_err(|error| AppError::internal(format!("{task_name}: {error}")))?;

            operation(&mut connection)
        })
        .await
        .map_err(|error| AppError::internal(format!("{task_name}: {error}")))?
    }
}

fn build_postgres_pool(config: &AppConfig) -> Result<DieselPool<ConnectionManager<PgConnection>>> {
    let manager = ConnectionManager::<PgConnection>::new(config.database_url.clone());
    DieselPool::builder()
        .build(manager)
        .context("while attempting to build the Postgres pool")
}

async fn run_migrations(pool: DieselPool<ConnectionManager<PgConnection>>) -> Result<()> {
    task::spawn_blocking(move || {
        let mut connection = pool
            .get()
            .context("while attempting to checkout a Postgres connection for migrations")?;
        connection
            .run_pending_migrations(MIGRATIONS)
            .map_err(|error| {
                anyhow::anyhow!("while attempting to run Diesel migrations: {error}")
            })?;
        Result::<_, anyhow::Error>::Ok(())
    })
    .await
    .context("while attempting to await the migration task")??;

    Ok(())
}

pub fn now_epoch_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_secs() as i64
}

pub fn json_event<T: serde::Serialize>(
    payload: &T,
) -> Result<axum::response::sse::Event, AppError> {
    let data = serde_json::to_string(payload).map_err(AppError::internal)?;
    Ok(axum::response::sse::Event::default().data(data))
}
