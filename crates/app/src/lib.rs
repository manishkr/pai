use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_meta::provide_meta_context;
use shared::{
    ChatStreamCompleted, ChatStreamEvent, ChatStreamStarted, ChatStreamToken,
    ConversationMessagesResponse, ConversationSummary, CreateConversationResponse, MessageRecord,
    MessageRole,
};
use uuid::Uuid;

// =============================================================================
// Browser App
// =============================================================================
//
// This UI intentionally keeps the first version focused on the core chat loop:
// list conversations, load a thread, send a prompt, and stream the assistant's
// reply token-by-token.

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    let conversations = RwSignal::new(Vec::<ConversationSummary>::new());
    let messages = RwSignal::new(Vec::<MessageRecord>::new());
    let selected_conversation_id = RwSignal::new(None::<Uuid>);
    let prompt = RwSignal::new(String::new());
    let streaming_text = RwSignal::new(String::new());
    let is_streaming = RwSignal::new(false);
    let error_message = RwSignal::new(None::<String>);

    Effect::new(move |_| {
        spawn_local(async move {
            match fetch_conversations().await {
                Ok(items) => conversations.set(items),
                Err(error) => error_message.set(Some(error)),
            }
        });
    });

    let create_chat = move || {
        let conversations = conversations;
        let selected_conversation_id = selected_conversation_id;
        let messages = messages;
        let error_message = error_message;

        spawn_local(async move {
            match create_conversation().await {
                Ok(conversation) => {
                    selected_conversation_id.set(Some(conversation.id));
                    messages.set(Vec::new());
                    conversations.update(|items| items.insert(0, conversation));
                }
                Err(error) => error_message.set(Some(error)),
            }
        });
    };

    let select_conversation = move |conversation_id: Uuid| {
        selected_conversation_id.set(Some(conversation_id));
        let messages = messages;
        let error_message = error_message;

        spawn_local(async move {
            match fetch_messages(conversation_id).await {
                Ok(items) => messages.set(items),
                Err(error) => error_message.set(Some(error)),
            }
        });
    };

    let submit_prompt = move |event: SubmitEvent| {
        event.prevent_default();

        let prompt_value = prompt.get();
        if prompt_value.trim().is_empty() || is_streaming.get() {
            return;
        }

        let conversations = conversations;
        let messages = messages;
        let prompt_signal = prompt;
        let selected_conversation_id = selected_conversation_id;
        let streaming_text = streaming_text;
        let is_streaming = is_streaming;
        let error_message = error_message;

        spawn_local(async move {
            let conversation_id = match selected_conversation_id.get() {
                Some(existing) => existing,
                None => match create_conversation().await {
                    Ok(conversation) => {
                        selected_conversation_id.set(Some(conversation.id));
                        conversations.update(|items| items.insert(0, conversation.clone()));
                        conversation.id
                    }
                    Err(error) => {
                        error_message.set(Some(error));
                        return;
                    }
                },
            };

            let optimistic_message = MessageRecord {
                id: Uuid::new_v4(),
                conversation_id,
                role: MessageRole::User,
                content: prompt_value.clone(),
                created_at: 0,
            };
            messages.update(|items| items.push(optimistic_message));

            prompt_signal.set(String::new());
            streaming_text.set(String::new());
            is_streaming.set(true);

            if let Err(error) = stream_reply(
                conversation_id,
                prompt_value,
                messages,
                conversations,
                selected_conversation_id,
                streaming_text,
                is_streaming,
            )
            .await
            {
                error_message.set(Some(error));
                is_streaming.set(false);
            }
        });
    };

    view! {
        <div class="min-h-screen bg-slate-950 text-slate-100">
            <div class="mx-auto grid min-h-screen max-w-7xl grid-cols-1 gap-6 px-4 py-6 lg:grid-cols-[280px_minmax(0,1fr)]">
                <aside class="rounded-3xl border border-white/10 bg-slate-900/70 p-4 shadow-2xl backdrop-blur">
                    <div class="mb-4 flex items-center justify-between">
                        <h1 class="text-lg font-semibold tracking-wide">"Share Doc Chat"</h1>
                        <button class="rounded-full bg-emerald-400 px-3 py-2 text-sm font-semibold text-slate-950"
                            on:click=move |_| create_chat()>
                            "New chat"
                        </button>
                    </div>

                    <div class="space-y-2">
                        <For
                            each=move || conversations.get()
                            key=|conversation| conversation.id
                            children=move |conversation| {
                                let conversation_id = conversation.id;
                                view! {
                                    <button
                                        class="w-full rounded-2xl border border-white/10 bg-slate-800/80 px-3 py-3 text-left transition hover:border-emerald-300/60 hover:bg-slate-800"
                                        on:click=move |_| select_conversation(conversation_id)
                                    >
                                        <div class="truncate text-sm font-medium">{conversation.title.clone()}</div>
                                        <div class="mt-1 text-xs text-slate-400">{format_epoch(conversation.updated_at)}</div>
                                    </button>
                                }
                            }
                        />
                    </div>
                </aside>

                <main class="flex min-h-[80vh] flex-col rounded-[2rem] border border-white/10 bg-slate-900/60 shadow-2xl backdrop-blur">
                    <div class="border-b border-white/10 px-6 py-5">
                        <h2 class="text-xl font-semibold">"Anonymous chat MVP"</h2>
                        <p class="mt-1 text-sm text-slate-400">
                            "Streaming replies over SSE, Redis-backed hot state, and Diesel-backed persisted history."
                        </p>
                    </div>

                    <div class="flex-1 space-y-4 overflow-y-auto px-6 py-6">
                        <For
                            each=move || messages.get()
                            key=|message| message.id
                            children=move |message| {
                                let class = if message.role == MessageRole::User {
                                    "ml-auto max-w-3xl rounded-3xl rounded-br-md bg-emerald-400 px-4 py-3 text-slate-950"
                                } else {
                                    "mr-auto max-w-3xl whitespace-pre-wrap rounded-3xl rounded-bl-md bg-slate-800 px-4 py-3 text-slate-100"
                                };

                                view! {
                                    <div class=class>{message.content}</div>
                                }
                            }
                        />

                        <Show when=move || !streaming_text.get().is_empty() fallback=|| ()>
                            <div class="mr-auto max-w-3xl whitespace-pre-wrap rounded-3xl rounded-bl-md bg-slate-800 px-4 py-3 text-slate-100">
                                {move || streaming_text.get()}
                            </div>
                        </Show>

                        <Show when=move || error_message.get().is_some() fallback=|| ()>
                            <div class="rounded-2xl border border-rose-400/40 bg-rose-500/10 px-4 py-3 text-sm text-rose-200">
                                {move || error_message.get().unwrap_or_default()}
                            </div>
                        </Show>
                    </div>

                    <form class="border-t border-white/10 px-6 py-5" on:submit=submit_prompt>
                        <label class="mb-2 block text-sm text-slate-400" for="prompt">
                            "Ask something"
                        </label>
                        <textarea
                            id="prompt"
                            class="h-32 w-full rounded-3xl border border-white/10 bg-slate-950/70 px-4 py-4 text-sm text-slate-100 outline-none transition focus:border-emerald-300/80"
                            prop:value=move || prompt.get()
                            on:input=move |event| prompt.set(event_target_value(&event))
                            placeholder="Type a message and watch the assistant stream back..."
                        />
                        <div class="mt-4 flex items-center justify-between">
                            <div class="text-xs text-slate-500">
                                {move || if is_streaming.get() { "Assistant is streaming..." } else { "Ready" }}
                            </div>
                            <button
                                class="rounded-full bg-emerald-400 px-5 py-3 text-sm font-semibold text-slate-950 disabled:cursor-not-allowed disabled:opacity-50"
                                prop:disabled=move || is_streaming.get()
                                type="submit"
                            >
                                "Send"
                            </button>
                        </div>
                    </form>
                </main>
            </div>
        </div>
    }
}

async fn stream_reply(
    conversation_id: Uuid,
    prompt: String,
    messages: RwSignal<Vec<MessageRecord>>,
    conversations: RwSignal<Vec<ConversationSummary>>,
    selected_conversation_id: RwSignal<Option<Uuid>>,
    streaming_text: RwSignal<String>,
    is_streaming: RwSignal<bool>,
) -> Result<(), String> {
    let url = format!(
        "/api/chat/stream?conversation_id={conversation_id}&message={}",
        urlencoding::encode(&prompt)
    );
    let event_source = browser::event_source(&url)?;

    browser::attach_stream_handlers(
        event_source,
        move |event| match event {
            ChatStreamEvent::Started(ChatStreamStarted { .. }) => {}
            ChatStreamEvent::Token(ChatStreamToken { text, .. }) => {
                streaming_text.update(|value| value.push_str(&text));
            }
            ChatStreamEvent::Completed(ChatStreamCompleted { .. }) => {
                let streaming = streaming_text.get();
                if !streaming.is_empty() {
                    messages.update(|items| {
                        items.push(MessageRecord {
                            id: Uuid::new_v4(),
                            conversation_id,
                            role: MessageRole::Assistant,
                            content: streaming.clone(),
                            created_at: 0,
                        });
                    });
                }

                streaming_text.set(String::new());
                is_streaming.set(false);
                selected_conversation_id.set(Some(conversation_id));

                spawn_local(async move {
                    if let Ok(updated) = fetch_conversations().await {
                        conversations.set(updated);
                    }
                    if let Ok(updated) = fetch_messages(conversation_id).await {
                        messages.set(updated);
                    }
                });
            }
            ChatStreamEvent::Error(error) => {
                is_streaming.set(false);
                streaming_text.set(format!("Streaming failed: {}", error.message));
            }
        },
        move || {
            is_streaming.set(false);
        },
    )?;

    Ok(())
}

async fn create_conversation() -> Result<ConversationSummary, String> {
    browser::post_json::<_, CreateConversationResponse>(
        "/api/conversations",
        &serde_json::json!({ "title": null }),
    )
    .await
    .map(|payload| payload.conversation)
}

async fn fetch_conversations() -> Result<Vec<ConversationSummary>, String> {
    browser::get_json("/api/conversations").await
}

async fn fetch_messages(conversation_id: Uuid) -> Result<Vec<MessageRecord>, String> {
    let response: ConversationMessagesResponse =
        browser::get_json(&format!("/api/conversations/{conversation_id}/messages")).await?;
    Ok(response.messages)
}

fn format_epoch(epoch: i64) -> String {
    if epoch <= 0 {
        return "just now".to_string();
    }

    #[cfg(target_arch = "wasm32")]
    {
        let millis = (epoch as f64) * 1000.0;
        let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(millis));
        return date.to_locale_string("en-US");
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        "recent".to_string()
    }
}

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    leptos::mount::hydrate_body(App);
}

#[cfg(not(feature = "hydrate"))]
pub fn hydrate() {}

#[cfg(target_arch = "wasm32")]
mod browser {
    use gloo_net::http::Request;
    use serde::Serialize;
    use wasm_bindgen::closure::Closure;
    use wasm_bindgen::{JsCast, JsValue};
    use web_sys::{Event, EventSource, MessageEvent};

    use super::ChatStreamEvent;

    pub async fn get_json<T>(url: &str) -> Result<T, String>
    where
        T: serde::de::DeserializeOwned,
    {
        Request::get(url)
            .send()
            .await
            .map_err(|error| error.to_string())?
            .json::<T>()
            .await
            .map_err(|error| error.to_string())
    }

    pub async fn post_json<B, T>(url: &str, body: &B) -> Result<T, String>
    where
        B: Serialize + ?Sized,
        T: serde::de::DeserializeOwned,
    {
        Request::post(url)
            .json(body)
            .map_err(|error| error.to_string())?
            .send()
            .await
            .map_err(|error| error.to_string())?
            .json::<T>()
            .await
            .map_err(|error| error.to_string())
    }

    pub fn event_source(url: &str) -> Result<EventSource, String> {
        EventSource::new(url).map_err(|error| format!("{error:?}"))
    }

    pub fn attach_stream_handlers<F, G>(
        event_source: EventSource,
        mut on_event: F,
        mut on_close: G,
    ) -> Result<(), String>
    where
        F: FnMut(ChatStreamEvent) + 'static,
        G: FnMut() + 'static,
    {
        let source_for_message = event_source.clone();
        let message_callback =
            Closure::<dyn FnMut(MessageEvent)>::wrap(Box::new(move |event: MessageEvent| {
                if let Some(text) = event.data().as_string() {
                    if let Ok(payload) = serde_json::from_str::<ChatStreamEvent>(&text) {
                        let should_close = matches!(
                            payload,
                            ChatStreamEvent::Completed(_) | ChatStreamEvent::Error(_)
                        );
                        on_event(payload);
                        if should_close {
                            source_for_message.close();
                            on_close();
                        }
                    }
                }
            }));
        event_source.set_onmessage(Some(message_callback.as_ref().unchecked_ref()));
        message_callback.forget();

        let source_for_error = event_source.clone();
        let error_callback = Closure::<dyn FnMut(Event)>::wrap(Box::new(move |_event: Event| {
            source_for_error.close();
            on_close();
        }));
        event_source.set_onerror(Some(error_callback.as_ref().unchecked_ref()));
        error_callback.forget();

        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod browser {
    use serde::Serialize;

    use super::ChatStreamEvent;

    pub async fn get_json<T>(_url: &str) -> Result<T, String>
    where
        T: serde::de::DeserializeOwned,
    {
        Err("browser APIs are only available in the hydrated client".to_string())
    }

    pub async fn post_json<B, T>(_url: &str, _body: &B) -> Result<T, String>
    where
        B: Serialize + ?Sized,
        T: serde::de::DeserializeOwned,
    {
        Err("browser APIs are only available in the hydrated client".to_string())
    }

    pub fn event_source(_url: &str) -> Result<(), String> {
        Err("browser APIs are only available in the hydrated client".to_string())
    }

    pub fn attach_stream_handlers<F, G>(
        _event_source: (),
        _on_event: F,
        _on_close: G,
    ) -> Result<(), String>
    where
        F: FnMut(ChatStreamEvent) + 'static,
        G: FnMut() + 'static,
    {
        Err("browser APIs are only available in the hydrated client".to_string())
    }
}
