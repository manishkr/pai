mod anthropic;
mod auth;
mod config;
mod documents;
mod error;
mod models;
mod routes;
mod schema;
mod state;

use anyhow::{Context, Result};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::AppConfig;
use crate::routes::router;
use crate::state::AppState;

// =============================================================================
// Server Bootstrap
// =============================================================================
//
// The server owns all external integrations: HTTP, Postgres, Redis, and the
// Anthropic streaming client. The browser UI is served as a plain HTML/CSS/JS
// shell so local development works with `cargo run -p server` alone.

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = AppConfig::from_env().context("while attempting to load configuration")?;
    let bind_address = config.bind_address;
    let state = AppState::build(config)
        .await
        .context("while attempting to initialize application state")?;

    let listener = tokio::net::TcpListener::bind(bind_address)
        .await
        .context("while attempting to bind the server socket")?;
    let app = router(state);

    info!(%bind_address, "server listening");
    axum::serve(listener, app)
        .await
        .context("while attempting to serve HTTP traffic")?;

    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("server=info,tower_http=info,axum::rejection=trace"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .json()
        .init();
}

pub fn app_shell(public_origin: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en" class="h-full">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Share Doc Chat</title>
    <link rel="stylesheet" href="/style.css" />
  </head>
  <body data-public-origin="{public_origin}">
    <div class="shell">
      <section class="login-screen hidden" data-login-screen>
        <div class="login-card">
          <div class="login-eyebrow">Workspace only</div>
          <h1 class="login-title">Share Doc Chat</h1>
          <p class="login-copy">
            Conversations and grounded documents require company sign-in. Continue with your Google Workspace account to open the chat workspace.
          </p>
          <a class="button login-button" href="/auth/google/start">Continue with Google</a>
        </div>
      </section>

      <div class="layout hidden" data-app-layout>
        <aside class="sidebar">
          <button class="button" type="button" data-new-chat>+ New chat</button>
          <div class="sidebar-label">Recent</div>
          <div class="conversation-list" data-conversation-list></div>
          <div class="sidebar-footer">
            <div class="account-card">
              <div class="account-avatar" data-user-avatar>SD</div>
              <div class="account-copy">
                <div class="account-name" data-user-name>Loading user...</div>
                <div class="account-email" data-user-email></div>
              </div>
              <button class="settings-item logout-button" type="button" data-logout>Sign out</button>
            </div>
          </div>
        </aside>

        <main class="thread">
          <div class="thread-header">
            <h1 class="thread-title">Share Doc Chat</h1>
            <div class="thread-actions">
              <div class="model-chip">claude-sonnet-4</div>
              <button class="menu-button" type="button" aria-label="More options">&hellip;</button>
            </div>
          </div>

          <section class="messages" data-messages>
            <div class="message-empty" data-empty-state>
              Start a conversation on the left or send a prompt below. Replies stream live into the thread and finished turns are saved automatically.
            </div>
          </section>

          <div class="composer">
            <div class="error hidden" data-error></div>
            <div class="selected-docs hidden" data-selected-docs></div>
            <form data-composer>
              <div class="composer-shell">
                <div class="composer-main">
                  <div class="mention-suggestions hidden" data-mention-suggestions></div>
                  <textarea
                    class="composer-input"
                    id="prompt"
                    data-prompt
                    placeholder="Sure! Here's how you'd set up streaming..."
                  ></textarea>
                </div>
                <div class="composer-side">
                  <div class="status" data-status>Loading conversations...</div>
                  <button class="button send-button" type="submit" data-send aria-label="Send">&#8593;</button>
                </div>
              </div>
            </form>
          </div>
        </main>

        <aside class="document-rail">
          <div class="document-rail-header">
            <div>
              <div class="document-rail-title">Documents</div>
              <div class="document-rail-copy">Authorized SharePoint documents only. Select one or more to ground the reply.</div>
            </div>
          </div>
          <div class="document-list" data-document-list></div>
        </aside>
      </div>
    </div>
    <script src="/app.js"></script>
  </body>
</html>"#
    )
}
