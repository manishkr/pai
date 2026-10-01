use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::{FromRef, FromRequestParts};
use axum::http::header::COOKIE;
use axum::http::{HeaderMap, HeaderValue};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cookie::{Cookie, SameSite};
use diesel::prelude::*;
use hmac::{Hmac, KeyInit, Mac};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use shared::AuthenticatedUser;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::error::AppError;
use crate::models::{NewAuthSession, NewUser};
use crate::schema::{auth_sessions, users};
use crate::state::{AUTH_SESSION_TTL_SECS, AppState, now_epoch_seconds};

type HmacSha256 = Hmac<Sha256>;

const APP_SESSION_COOKIE_NAME: &str = "share_doc_auth_session";
const OAUTH_STATE_COOKIE_NAME: &str = "share_doc_oauth_state";
const OAUTH_VERIFIER_COOKIE_NAME: &str = "share_doc_oauth_verifier";
const OAUTH_TRANSIENT_MAX_AGE_SECS: i64 = 10 * 60;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthContext {
    pub auth_session_id: Uuid,
    pub user_id: Uuid,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GoogleCallbackQuery {
    pub code: String,
    pub state: String,
}

#[derive(Debug, Clone)]
pub struct OAuthStart {
    pub authorization_url: String,
    pub set_cookies: Vec<HeaderValue>,
}

#[derive(Debug, Clone)]
pub struct CompletedLogin {
    pub auth_session_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct GoogleTokenResponse {
    access_token: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct GoogleUserInfo {
    pub sub: String,
    pub email: String,
    #[serde(default, alias = "email_verified")]
    pub verified_email: bool,
    #[serde(default)]
    pub name: String,
    pub picture: Option<String>,
    pub hd: Option<String>,
}

impl GoogleUserInfo {
    fn display_name(&self) -> &str {
        if self.name.trim().is_empty() {
            &self.email
        } else {
            &self.name
        }
    }
}

impl AuthContext {
    pub fn to_shared_user(&self) -> AuthenticatedUser {
        AuthenticatedUser {
            email: self.email.clone(),
            name: self.name.clone(),
            avatar_url: self.avatar_url.clone(),
        }
    }
}

impl<S> FromRequestParts<S> for AuthContext
where
    S: Send + Sync,
    Arc<AppState>: FromRef<S>,
{
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> impl std::future::Future<Output = Result<Self, Self::Rejection>> + Send {
        let headers = parts.headers.clone();
        let state = Arc::<AppState>::from_ref(state);

        async move { authenticate_request(state, &headers).await }
    }
}

pub async fn authenticate_request(
    state: Arc<AppState>,
    headers: &HeaderMap,
) -> Result<AuthContext, AppError> {
    let Some(raw_cookie_value) = read_cookie_value(headers, APP_SESSION_COOKIE_NAME) else {
        return Err(AppError::Unauthorized("authentication required"));
    };

    let Some(session_id_value) =
        verify_signed_value(&state.config.cookie_secret, &raw_cookie_value)
    else {
        return Err(AppError::Unauthorized("invalid session cookie"));
    };
    let auth_session_id = Uuid::parse_str(&session_id_value)
        .map_err(|_| AppError::Unauthorized("invalid session cookie"))?;

    let now = now_epoch_seconds();
    state
        .run_db("load authenticated session", move |connection| {
            auth_sessions::table
                .inner_join(users::table.on(auth_sessions::user_id.eq(users::id)))
                .filter(auth_sessions::id.eq(auth_session_id))
                .filter(auth_sessions::expires_at.gt(now))
                .select((
                    auth_sessions::id,
                    users::id,
                    users::email,
                    users::name,
                    users::avatar_url,
                ))
                .first::<(Uuid, Uuid, String, String, Option<String>)>(connection)
                .optional()
                .map_err(|error| {
                    AppError::internal(format!("load authenticated session: {error}"))
                })?
                .map(
                    |(auth_session_id, user_id, email, name, avatar_url)| AuthContext {
                        auth_session_id,
                        user_id,
                        email,
                        name,
                        avatar_url,
                    },
                )
                .ok_or(AppError::Unauthorized("authentication required"))
        })
        .await
}

pub fn begin_google_login(config: &AppConfig) -> Result<OAuthStart, AppError> {
    let oauth_state = generate_token();
    let code_verifier = generate_token();
    let code_challenge = pkce_challenge(&code_verifier);

    let set_cookies = vec![
        signed_cookie(
            config,
            OAUTH_STATE_COOKIE_NAME,
            &oauth_state,
            OAUTH_TRANSIENT_MAX_AGE_SECS,
            false,
        )?,
        signed_cookie(
            config,
            OAUTH_VERIFIER_COOKIE_NAME,
            &code_verifier,
            OAUTH_TRANSIENT_MAX_AGE_SECS,
            false,
        )?,
    ];

    let authorization_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256&prompt=select_account&hd={}",
        urlencoding::encode(&config.google_client_id),
        urlencoding::encode(&config.google_redirect_uri),
        urlencoding::encode("openid email profile"),
        urlencoding::encode(&oauth_state),
        urlencoding::encode(&code_challenge),
        urlencoding::encode(&config.google_hosted_domain),
    );

    Ok(OAuthStart {
        authorization_url,
        set_cookies,
    })
}

pub async fn complete_google_login(
    state: Arc<AppState>,
    headers: &HeaderMap,
    query: GoogleCallbackQuery,
) -> Result<CompletedLogin, AppError> {
    let expected_state = read_signed_cookie_value(
        headers,
        OAUTH_STATE_COOKIE_NAME,
        &state.config.cookie_secret,
    )
    .ok_or(AppError::Unauthorized("missing oauth state"))?;
    let code_verifier = read_signed_cookie_value(
        headers,
        OAUTH_VERIFIER_COOKIE_NAME,
        &state.config.cookie_secret,
    )
    .ok_or(AppError::Unauthorized("missing oauth verifier"))?;

    if expected_state != query.state {
        return Err(AppError::Unauthorized("oauth state mismatch"));
    }

    let profile = fetch_google_user_info(&state.config, &query.code, &code_verifier).await?;
    validate_google_user_info(&profile, &state.config.google_hosted_domain)?;

    let now = now_epoch_seconds();
    let session_expires_at =
        now + i64::try_from(AUTH_SESSION_TTL_SECS).expect("session TTL should fit i64");
    let profile_for_db = profile.clone();
    let auth_session_id = Uuid::new_v4();

    state
        .run_db("upsert authenticated user", move |connection| {
            // We keep the user upsert inline so the entire login write path stays
            // transactional from the application's perspective.
            let existing_user_id = users::table
                .filter(users::google_sub.eq(&profile_for_db.sub))
                .select(users::id)
                .first::<Uuid>(connection)
                .optional()
                .map_err(|error| AppError::internal(format!("lookup existing user: {error}")))?;

            let user_id = if let Some(existing_user_id) = existing_user_id {
                diesel::update(users::table.find(existing_user_id))
                    .set((
                        users::email.eq(&profile_for_db.email),
                        users::name.eq(profile_for_db.display_name()),
                        users::avatar_url.eq(profile_for_db.picture.as_deref()),
                        users::hosted_domain.eq(profile_for_db.hd.as_deref().unwrap_or_default()),
                        users::last_login_at.eq(now),
                    ))
                    .execute(connection)
                    .map_err(|error| {
                        AppError::internal(format!("update existing user: {error}"))
                    })?;
                existing_user_id
            } else {
                let user_id = Uuid::new_v4();
                let new_user = NewUser {
                    id: user_id,
                    google_sub: &profile_for_db.sub,
                    email: &profile_for_db.email,
                    name: profile_for_db.display_name(),
                    avatar_url: profile_for_db.picture.as_deref(),
                    hosted_domain: profile_for_db.hd.as_deref().unwrap_or_default(),
                    created_at: now,
                    last_login_at: now,
                };

                diesel::insert_into(users::table)
                    .values(&new_user)
                    .execute(connection)
                    .map_err(|error| AppError::internal(format!("insert user: {error}")))?;
                user_id
            };

            let auth_session = NewAuthSession {
                id: auth_session_id,
                user_id,
                expires_at: session_expires_at,
                created_at: now,
            };
            diesel::insert_into(auth_sessions::table)
                .values(&auth_session)
                .execute(connection)
                .map_err(|error| AppError::internal(format!("create auth session: {error}")))?;

            Ok(())
        })
        .await?;

    Ok(CompletedLogin { auth_session_id })
}

pub async fn logout_request(state: Arc<AppState>, headers: &HeaderMap) -> Result<(), AppError> {
    let Some(raw_cookie_value) = read_cookie_value(headers, APP_SESSION_COOKIE_NAME) else {
        return Ok(());
    };
    let Some(session_id_value) =
        verify_signed_value(&state.config.cookie_secret, &raw_cookie_value)
    else {
        return Ok(());
    };
    let Ok(auth_session_id) = Uuid::parse_str(&session_id_value) else {
        return Ok(());
    };

    state
        .run_db("delete auth session", move |connection| {
            diesel::delete(auth_sessions::table.find(auth_session_id))
                .execute(connection)
                .map_err(|error| AppError::internal(format!("delete auth session: {error}")))?;
            Ok(())
        })
        .await
}

pub fn session_cookie(config: &AppConfig, auth_session_id: Uuid) -> Result<HeaderValue, AppError> {
    signed_cookie(
        config,
        APP_SESSION_COOKIE_NAME,
        &auth_session_id.to_string(),
        i64::try_from(AUTH_SESSION_TTL_SECS).expect("session TTL should fit i64"),
        true,
    )
}

pub fn clear_auth_cookies(config: &AppConfig) -> Result<Vec<HeaderValue>, AppError> {
    Ok(vec![
        expired_cookie(config, APP_SESSION_COOKIE_NAME, true)?,
        expired_cookie(config, OAUTH_STATE_COOKIE_NAME, false)?,
        expired_cookie(config, OAUTH_VERIFIER_COOKIE_NAME, false)?,
    ])
}

pub fn clear_oauth_cookies(config: &AppConfig) -> Result<Vec<HeaderValue>, AppError> {
    Ok(vec![
        expired_cookie(config, OAUTH_STATE_COOKIE_NAME, false)?,
        expired_cookie(config, OAUTH_VERIFIER_COOKIE_NAME, false)?,
    ])
}

async fn fetch_google_user_info(
    config: &AppConfig,
    code: &str,
    code_verifier: &str,
) -> Result<GoogleUserInfo, AppError> {
    let client = reqwest::Client::new();
    let token = client
        .post("https://oauth2.googleapis.com/token")
        .form(&[
            ("client_id", config.google_client_id.as_str()),
            ("client_secret", config.google_client_secret.as_str()),
            ("code", code),
            ("code_verifier", code_verifier),
            ("grant_type", "authorization_code"),
            ("redirect_uri", config.google_redirect_uri.as_str()),
        ])
        .send()
        .await
        .context("while attempting to exchange the Google authorization code")
        .map_err(AppError::internal)?
        .error_for_status()
        .context("while attempting to validate the Google token response")
        .map_err(AppError::internal)?
        .json::<GoogleTokenResponse>()
        .await
        .context("while attempting to decode the Google token response")
        .map_err(AppError::internal)?;

    let profile_response = client
        .get("https://openidconnect.googleapis.com/v1/userinfo")
        .bearer_auth(token.access_token)
        .send()
        .await
        .context("while attempting to fetch the Google user profile")
        .map_err(AppError::internal)?
        .error_for_status()
        .context("while attempting to validate the Google user profile response")
        .map_err(AppError::internal)?;
    let profile_payload = profile_response
        .text()
        .await
        .context("while attempting to read the Google user profile body")
        .map_err(AppError::internal)?;

    serde_json::from_str::<GoogleUserInfo>(&profile_payload)
        .with_context(|| {
            format!("while attempting to decode the Google user profile: {profile_payload}")
        })
        .map_err(AppError::internal)
}

fn validate_google_user_info(
    profile: &GoogleUserInfo,
    expected_hosted_domain: &str,
) -> Result<(), AppError> {
    if !profile.verified_email {
        return Err(AppError::Forbidden("google email must be verified"));
    }

    let Some((_, email_domain)) = profile.email.rsplit_once('@') else {
        return Err(AppError::Forbidden(
            "google account must provide an email address",
        ));
    };

    let Some(hosted_domain) = profile.hd.as_deref() else {
        return Err(AppError::Forbidden(
            "google account must belong to the workspace domain",
        ));
    };

    if hosted_domain != expected_hosted_domain || email_domain != expected_hosted_domain {
        return Err(AppError::Forbidden(
            "google account is outside the allowed workspace",
        ));
    }

    Ok(())
}

fn signed_cookie(
    config: &AppConfig,
    name: &str,
    value: &str,
    max_age_seconds: i64,
    http_only: bool,
) -> Result<HeaderValue, AppError> {
    let signed_value = sign_value(&config.cookie_secret, value)?;
    cookie_header(config, name, &signed_value, max_age_seconds, http_only)
}

fn expired_cookie(
    config: &AppConfig,
    name: &str,
    http_only: bool,
) -> Result<HeaderValue, AppError> {
    cookie_header(config, name, "", 0, http_only)
}

fn cookie_header(
    config: &AppConfig,
    name: &str,
    value: &str,
    max_age_seconds: i64,
    http_only: bool,
) -> Result<HeaderValue, AppError> {
    let mut builder = Cookie::build((name, value.to_string()))
        .path("/")
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::seconds(max_age_seconds));

    if http_only {
        builder = builder.http_only(true);
    }
    if should_use_secure_cookies(config) {
        builder = builder.secure(true);
    }

    HeaderValue::from_str(&builder.build().to_string())
        .map_err(|_| AppError::internal("build auth cookie header"))
}

fn should_use_secure_cookies(config: &AppConfig) -> bool {
    config.public_origin.starts_with("https://")
}

fn read_signed_cookie_value(headers: &HeaderMap, name: &str, secret: &str) -> Option<String> {
    let raw_value = read_cookie_value(headers, name)?;
    verify_signed_value(secret, &raw_value)
}

fn read_cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let header = headers.get(COOKIE)?.to_str().ok()?;
    for part in header.split(';') {
        let cookie = Cookie::parse(part.trim()).ok()?;
        if cookie.name() == name {
            return Some(cookie.value().to_string());
        }
    }

    None
}

fn sign_value(secret: &str, value: &str) -> Result<String, AppError> {
    let encoded_value = URL_SAFE_NO_PAD.encode(value.as_bytes());
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| AppError::internal("create signing key"))?;
    mac.update(encoded_value.as_bytes());
    let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
    Ok(format!("{encoded_value}.{signature}"))
}

fn verify_signed_value(secret: &str, signed_value: &str) -> Option<String> {
    let (encoded_value, encoded_signature) = signed_value.rsplit_once('.')?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(encoded_value.as_bytes());
    let expected_signature = URL_SAFE_NO_PAD.decode(encoded_signature.as_bytes()).ok()?;
    mac.verify_slice(&expected_signature).ok()?;

    let decoded = URL_SAFE_NO_PAD.decode(encoded_value.as_bytes()).ok()?;
    String::from_utf8(decoded).ok()
}

fn generate_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

#[cfg(test)]
mod tests {
    use super::{
        GoogleUserInfo, pkce_challenge, sign_value, validate_google_user_info, verify_signed_value,
    };

    #[test]
    fn signed_values_round_trip() {
        let signed = sign_value("secret", "session-id").expect("value should sign");

        let verified = verify_signed_value("secret", &signed).expect("value should verify");

        assert_eq!(verified, "session-id");
    }

    #[test]
    fn tampered_signed_values_fail_verification() {
        let signed = sign_value("secret", "session-id").expect("value should sign");
        let tampered = format!("{signed}oops");

        assert!(verify_signed_value("secret", &tampered).is_none());
    }

    #[test]
    fn pkce_challenge_is_url_safe() {
        let challenge = pkce_challenge("verifier-value");

        assert!(!challenge.contains('='));
        assert!(!challenge.contains('+'));
        assert!(!challenge.contains('/'));
    }

    #[test]
    fn workspace_validation_rejects_wrong_domain() {
        let error = validate_google_user_info(
            &GoogleUserInfo {
                sub: "123".to_string(),
                email: "alice@outside.com".to_string(),
                verified_email: true,
                name: "Alice".to_string(),
                picture: None,
                hd: Some("outside.com".to_string()),
            },
            "company.com",
        )
        .expect_err("wrong domain must fail");

        assert!(error.to_string().contains("workspace"));
    }

    #[test]
    fn workspace_validation_accepts_expected_domain() {
        validate_google_user_info(
            &GoogleUserInfo {
                sub: "123".to_string(),
                email: "alice@company.com".to_string(),
                verified_email: true,
                name: "Alice".to_string(),
                picture: Some("https://example.com/avatar.png".to_string()),
                hd: Some("company.com".to_string()),
            },
            "company.com",
        )
        .expect("expected domain should pass");
    }
}
