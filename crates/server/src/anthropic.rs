use anyhow::{Context, Result};
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use shared::{MessageRecord, MessageRole};
use std::future::Future;
use tokio::time::{Duration, sleep};

use crate::config::AppConfig;

// =============================================================================
// Anthropic Streaming
// =============================================================================
//
// Anthropic's stream arrives as SSE frames. We only care about text deltas for
// the MVP, so the parser intentionally ignores control frames like ping and
// non-text deltas.

#[derive(Clone)]
pub struct AnthropicClient {
    http: Client,
    api_key: Option<String>,
    model: String,
    mock: bool,
}

impl AnthropicClient {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            http: Client::new(),
            api_key: config.anthropic_api_key.clone(),
            model: config.anthropic_model.clone(),
            mock: config.anthropic_mock,
        }
    }

    pub async fn stream_messages<F>(
        &self,
        history: Vec<MessageRecord>,
        mut on_event: F,
    ) -> Result<()>
    where
        F: FnMut(AnthropicStreamEvent) -> PinBoxFuture + Send,
    {
        if self.mock {
            return self.stream_mock_messages(history, on_event).await;
        }

        let body = AnthropicRequest {
            model: self.model.clone(),
            max_tokens: 1024,
            stream: true,
            messages: history
                .into_iter()
                .map(|message| AnthropicMessage {
                    role: message.role.as_anthropic_role().to_string(),
                    content: message.content,
                })
                .collect(),
        };

        let response = self
            .http
            .post("https://api.anthropic.com/v1/messages")
            .header("anthropic-version", "2023-06-01")
            .header(
                "x-api-key",
                self.api_key
                    .as_deref()
                    .context("while attempting to read the configured Anthropic API key")?,
            )
            .json(&body)
            .send()
            .await
            .context("while attempting to send the Anthropic request")?
            .error_for_status()
            .context("while attempting to validate the Anthropic response")?;

        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("while attempting to read the Anthropic stream")?;
            buffer.push_str(
                std::str::from_utf8(&chunk)
                    .context("while attempting to decode Anthropic bytes")?,
            );

            while let Some(boundary) = buffer.find("\n\n") {
                let raw_event = buffer[..boundary].to_string();
                buffer.drain(..boundary + 2);

                if let Some(event) = parse_sse_block(&raw_event)? {
                    on_event(event).await?;
                }
            }
        }

        Ok(())
    }

    async fn stream_mock_messages<F>(
        &self,
        history: Vec<MessageRecord>,
        mut on_event: F,
    ) -> Result<()>
    where
        F: FnMut(AnthropicStreamEvent) -> PinBoxFuture + Send,
    {
        let prompt = history
            .iter()
            .rev()
            .find(|message| matches!(message.role, MessageRole::User))
            .map(|message| message.content.trim().to_string())
            .filter(|message| !message.is_empty())
            .unwrap_or_else(|| "Say hello".to_string());

        let chunks = mock_response_chunks(&prompt);
        on_event(AnthropicStreamEvent::Started).await?;

        for chunk in chunks {
            sleep(Duration::from_millis(80)).await;
            on_event(AnthropicStreamEvent::TextDelta(chunk)).await?;
        }

        sleep(Duration::from_millis(40)).await;
        on_event(AnthropicStreamEvent::Completed).await?;
        Ok(())
    }
}

type PinBoxFuture = std::pin::Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnthropicStreamEvent {
    Started,
    TextDelta(String),
    Completed,
}

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    stream: bool,
    messages: Vec<AnthropicMessage>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct RawAnthropicEvent {
    #[serde(rename = "type")]
    event_type: String,
    delta: Option<AnthropicDelta>,
}

#[derive(Debug, Deserialize)]
struct AnthropicDelta {
    #[serde(rename = "type")]
    delta_type: String,
    text: Option<String>,
}

pub fn parse_sse_block(block: &str) -> Result<Option<AnthropicStreamEvent>> {
    let mut event_name = None;
    let mut data = Vec::new();

    for line in block.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event_name = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.trim().to_string());
        }
    }

    let Some(event_name) = event_name else {
        return Ok(None);
    };

    if event_name == "ping" {
        return Ok(None);
    }

    let payload = data.join("\n");
    if payload == "[DONE]" {
        return Ok(Some(AnthropicStreamEvent::Completed));
    }

    let payload: RawAnthropicEvent =
        serde_json::from_str(&payload).context("while attempting to parse Anthropic SSE JSON")?;

    let event = match payload.event_type.as_str() {
        "message_start" => Some(AnthropicStreamEvent::Started),
        "message_stop" => Some(AnthropicStreamEvent::Completed),
        "content_block_delta" => payload
            .delta
            .filter(|delta| delta.delta_type == "text_delta")
            .and_then(|delta| delta.text)
            .map(AnthropicStreamEvent::TextDelta),
        _ => None,
    };

    Ok(event)
}

trait AnthropicRoleExt {
    fn as_anthropic_role(&self) -> &'static str;
}

impl AnthropicRoleExt for MessageRole {
    fn as_anthropic_role(&self) -> &'static str {
        match self {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        }
    }
}

fn mock_response_chunks(prompt: &str) -> Vec<String> {
    // We echo the user's intent back in a short, friendly answer so local
    // development exercises the same incremental rendering path as production.
    let sanitized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    let preview = sanitized.chars().take(80).collect::<String>();
    let response = format!(
        "Mock Anthropic reply for: \"{preview}\".\n\nThis is a simulated streaming response so you can test the full chat flow without a live Anthropic API key."
    );

    response
        .split_inclusive([' ', '\n'])
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{AnthropicStreamEvent, mock_response_chunks, parse_sse_block};

    #[test]
    fn parser_extracts_text_delta() {
        let raw = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}";
        let event = parse_sse_block(raw)
            .expect("valid SSE block should parse")
            .expect("block should produce an event");

        assert_eq!(event, AnthropicStreamEvent::TextDelta("hello".to_string()));
    }

    #[test]
    fn mock_response_mentions_prompt() {
        let chunks = mock_response_chunks("Write a greeting for new users");
        let combined = chunks.concat();

        assert!(combined.contains("Write a greeting for new users"));
        assert!(combined.contains("simulated streaming response"));
    }
}
