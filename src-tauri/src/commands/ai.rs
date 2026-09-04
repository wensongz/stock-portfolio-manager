use crate::db::Database;
use crate::models::ai_config::{AiConfig, AiModelInfo, ChatMessage, DEFAULT_SYSTEM_PROMPT};
use crate::services::ai_chat_service::{
    self, validated_portfolio_scope, ChatParams, PrefilledToolContext,
};
use crate::services::ai_config_service;
use crate::services::ai_models_service::{self, FetchModelsParams};
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_service::{QuoteCache, QuoteServiceState};
use serde::Deserialize;
use tauri::{AppHandle, State};

#[tauri::command(rename_all = "camelCase")]
pub async fn get_ai_config(db: State<'_, Database>) -> Result<AiConfig, String> {
    ai_config_service::get_ai_config(&db)
}

/// Return the built-in default system prompt. The UI uses this to offer a
/// "restore defaults" action without having to duplicate the prompt text.
#[tauri::command(rename_all = "camelCase")]
pub async fn get_default_system_prompt() -> Result<String, String> {
    Ok(DEFAULT_SYSTEM_PROMPT.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn update_ai_config(db: State<'_, Database>, config: AiConfig) -> Result<bool, String> {
    ai_config_service::update_ai_config(&db, &config)
}

/// Request body for `fetch_ai_models`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchModelsRequest {
    pub provider: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
}

/// Fetch the list of models available for a provider using the supplied
/// credentials. Returns an error string when the API is unreachable or the
/// credentials are invalid; the UI falls back to manual entry in that case.
#[tauri::command(rename_all = "camelCase")]
pub async fn fetch_ai_models(req: FetchModelsRequest) -> Result<Vec<AiModelInfo>, String> {
    ai_models_service::fetch_models(FetchModelsParams {
        provider: req.provider,
        api_key: req.api_key,
        base_url: req.base_url,
    })
    .await
}

/// Request body for `chat_with_ai`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatRequest {
    /// Conversation history (excluding the system prompt and the portfolio
    /// context, which are added on the backend).
    pub messages: Vec<ChatMessage>,
    /// Whether to inject the live portfolio snapshot as extra context.
    #[serde(default)]
    pub include_context: bool,
    /// Skill ids the user explicitly activated (via `/` / `@` or a quick
    /// chip). Empty means "auto-activate based on the latest user message".
    #[serde(default)]
    pub active_skills: Vec<String>,
    /// Exact read-only tool scope staged by a trusted app navigation. It is
    /// consumed by this single command invocation and never inferred from text.
    #[serde(default)]
    pub tool_context: Option<PrefilledToolContext>,
}

/// Stream an AI chat completion to the frontend via Tauri events
/// (`ai-chat-delta` / `ai-chat-done` / `ai-chat-error`). Returns `Ok(())` once
/// the stream finishes successfully; the actual content is delivered as
/// events, not through this return value.
#[tauri::command(rename_all = "camelCase")]
pub async fn chat_with_ai(
    app: AppHandle,
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    quote_state: State<'_, QuoteServiceState>,
    req: ChatRequest,
) -> Result<(), String> {
    let portfolio_scope = validated_portfolio_scope(req.tool_context.as_ref())?;
    ai_chat_service::chat_stream(
        app,
        &db,
        &cache,
        &quote_cache,
        &quote_state,
        ChatParams {
            messages: req.messages,
            include_context: req.include_context,
            active_skills: req.active_skills,
            tool_context: req.tool_context,
            portfolio_scope,
        },
    )
    .await
}

/// Abort any in-flight `chat_with_ai` stream. The streaming loop polls a
/// global stop flag between chunks; on the next iteration it will emit
/// `ai-chat-done` (with whatever tokens and usage were collected so far) and
/// return, cutting off further completion-token billing.
#[tauri::command(rename_all = "camelCase")]
pub async fn stop_ai_chat() -> Result<(), String> {
    ai_chat_service::stop_chat();
    Ok(())
}
