//! AI chat service: streams OpenAI-compatible `/chat/completions` responses.
//!
//! The service does two jobs:
//! 1. `build_portfolio_context` — assembles a structured Markdown snapshot of
//!    the user's current portfolio (overview, holdings, recent transactions,
//!    performance metrics) by reusing existing in-process services. It never
//!    triggers network quote fetches (`cache_only = true`).
//! 2. `chat_stream` — POSTs the conversation to the configured LLM provider
//!    with `stream: true`, parses the Server-Sent-Events stream chunk by
//!    chunk, and emits incremental token deltas to the frontend via the
//!    `ai-chat-delta` Tauri event (`ai-chat-done` / `ai-chat-error` on
//!    completion / failure).
//!
//! All supported providers (OpenAI / Ollama / OpenRouter / Kimi / GLM / MiMo)
//! expose the same `/chat/completions` shape, so the request logic is uniform.

use crate::commands::dashboard::build_holding_details_pub;
use crate::db::Database;
use crate::models::ai_config::ChatMessage;
use crate::models::skill::Skill;
use crate::services::ai_config_service;
use crate::services::ai_models_service::resolve_base_url;
use crate::services::exchange_rate_service::{get_cached_rates, ExchangeRateCache};
use crate::services::http_client;
use crate::services::performance_service::{self, PerformanceFilter};
use crate::services::quote_service::QuoteCache;
use crate::services::skill_service::{self, build_skill_system_message};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter};
use tracing::{debug, info, warn};

/// Parameters for a chat turn.
#[derive(Debug, Clone)]
pub struct ChatParams {
    /// Conversation history sent by the frontend (system prompt is prepended
    /// here from the saved config; portfolio context is added as an extra
    /// `user` message when `include_context` is true).
    pub messages: Vec<ChatMessage>,
    /// Whether to inject the portfolio context snapshot before the user
    /// messages.
    pub include_context: bool,
    /// Skill ids the user explicitly activated for this turn (via `/` or `@`
    /// in the composer, or by clicking a quick chip). Takes priority over
    /// automatic trigger-based activation.
    #[allow(dead_code)] // read via resolve_active_skills
    pub active_skills: Vec<String>,
}

/// Token-usage accounting for a single chat completion. Emitted to the
/// frontend once per turn (in the final SSE chunk).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatUsage {
    /// Tokens consumed by the prompt (system prompt + context + history).
    pub prompt_tokens: u32,
    /// Tokens produced by the model in its reply.
    pub completion_tokens: u32,
    /// `prompt_tokens + completion_tokens`.
    pub total_tokens: u32,
    /// Portion of `prompt_tokens` that hit the provider's prompt cache and is
    /// billed at a discount (OpenAI/Anthropic) or free (DeepSeek). 0 when the
    /// provider does not report cache info.
    #[serde(default)]
    pub cached_tokens: u32,
}

// Global stop flag. Set by `stop_chat()` from a Tauri command; the streaming
// loop polls it between chunks and aborts the request as soon as it is set.
// A single global flag is sufficient because only one chat turn is in flight
// at a time (the UI disables the send button while `sending` is true).
static STOP_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Request that any in-flight chat stream terminate as soon as possible.
///
/// The streaming loop checks this flag after each chunk and, when set, emits
/// `ai-chat-done` (with whatever tokens were already streamed) and returns.
/// The flag is cleared automatically by `chat_stream` at the start of each
/// new turn.
pub fn stop_chat() {
    STOP_REQUESTED.store(true, Ordering::SeqCst);
}

// ─────────────────────────────────────────────────────────────────────────────
// Skill activation
// ─────────────────────────────────────────────────────────────────────────────

/// Resolve which skills are active for this turn.
///
/// 1. If the user passed explicit skill ids (`active_skills`), look each one
///    up by id and use it as-is (regardless of `enabled` — explicit wins).
///    Unknown ids are silently dropped.
/// 2. Otherwise, fall back to **auto-activation**: load every skill, then
///    match each one's `trigger` keywords against the latest user message
///    (case-insensitive). Disabled skills are skipped.
///
/// Errors loading the skill list are swallowed (best-effort — skills should
/// never block the chat), returning an empty vec.
fn resolve_active_skills(app: &AppHandle, params: &ChatParams) -> Vec<Skill> {
    // Explicit selection path.
    if !params.active_skills.is_empty() {
        let ids: std::collections::HashSet<&str> =
            params.active_skills.iter().map(|s| s.as_str()).collect();
        let selected: Vec<Skill> = skill_service::list_skills(app)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| ids.contains(s.id.as_str()))
            .collect();
        if !selected.is_empty() {
            return selected;
        }
        // Fall through to auto-activation if every id was unknown.
    }

    // Auto-activation path: scan the latest user message for trigger hits.
    let latest_user = latest_user_message(params);
    if latest_user.trim().is_empty() {
        return Vec::new();
    }
    let skills = skill_service::list_skills(app).unwrap_or_default();
    skill_service::match_triggers(&skills, latest_user)
}

fn latest_user_message(params: &ChatParams) -> &str {
    params
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message.content.as_str())
        .unwrap_or("")
}

// ─────────────────────────────────────────────────────────────────────────────
// Portfolio context
// ─────────────────────────────────────────────────────────────────────────────

/// Assemble a Markdown snapshot of the current portfolio for the LLM prompt.
///
/// Uses cache-only quotes (no network) and pulls the last year of performance
/// metrics. Every section is guarded so an empty portfolio still yields a
/// short, valid context string rather than an error.
pub async fn build_portfolio_context(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
) -> Result<String, String> {
    let details = build_holding_details_pub(db, quote_cache, true).await?;
    let rates =
        get_cached_rates(cache, db)
            .await
            .unwrap_or_else(|_| crate::models::quote::ExchangeRates {
                usd_cny: 7.2,
                usd_hkd: 7.8,
                cny_hkd: 7.8 / 7.2,
                updated_at: Utc::now().to_rfc3339(),
            });

    // Normalise every holding's market value to USD for cross-currency totals.
    let to_usd = |amount: f64, currency: &str| {
        crate::services::exchange_rate_service::convert_currency(amount, currency, "USD", &rates)
    };
    let total_market_value_usd: f64 = details
        .iter()
        .map(|d| to_usd(d.market_value, &d.currency))
        .sum();
    let total_cost_value_usd: f64 = details
        .iter()
        .map(|d| to_usd(d.cost_value, &d.currency))
        .sum();
    let total_daily_pnl_usd: f64 = details
        .iter()
        .map(|d| to_usd(d.daily_pnl, &d.currency))
        .sum();

    let mut out = String::new();
    out.push_str("# 当前投资组合快照\n\n");

    // ── Overview ───────────────────────────────────────────────────────────
    out.push_str("## 账户总览（单位：USD）\n");
    if details.is_empty() {
        out.push_str("（暂无持仓）\n\n");
    } else {
        let total_pnl = total_market_value_usd - total_cost_value_usd;
        let total_pnl_pct = if total_cost_value_usd > 0.0 {
            total_pnl / total_cost_value_usd * 100.0
        } else {
            0.0
        };
        out.push_str(&format!(
            "- 持仓数量：{}\n- 总市值：{:.2}\n- 总成本：{:.2}\n- 累计盈亏：{:.2} ({:.2}%)\n- 当日盈亏：{:.2}\n\n",
            details.len(),
            total_market_value_usd,
            total_cost_value_usd,
            total_pnl,
            total_pnl_pct,
            total_daily_pnl_usd,
        ));
    }

    // ── Holdings table ─────────────────────────────────────────────────────
    out.push_str("## 当前持仓\n");
    out.push_str("| 代码 | 名称 | 市场 | 账户 | 类别 | 持仓 | 均价 | 现价 | 市值(USD) | 盈亏% |\n");
    out.push_str("|------|------|------|------|------|------|------|------|-----------|-------|\n");
    let mut sorted = details.clone();
    sorted.sort_by(|a, b| {
        to_usd(b.market_value, &b.currency)
            .partial_cmp(&to_usd(a.market_value, &a.currency))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for d in &sorted {
        let pnl_pct = d.pnl_percent.unwrap_or(0.0);
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.4} | {:.4} | {:.4} | {:.2} | {:.2} |\n",
            d.symbol,
            d.name,
            d.market,
            d.account_name,
            d.category_name,
            d.shares,
            d.avg_cost,
            d.current_price,
            to_usd(d.market_value, &d.currency),
            pnl_pct,
        ));
    }
    out.push('\n');

    // ── Recent transactions ────────────────────────────────────────────────
    out.push_str("## 近期交易（最近 20 条）\n");
    match fetch_recent_transactions(db, 20) {
        Ok(txns) if !txns.is_empty() => {
            out.push_str("| 日期 | 代码 | 名称 | 类型 | 持仓 | 价格 | 金额 |\n");
            out.push_str("|------|------|------|------|------|------|------|\n");
            for t in &txns {
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {:.4} | {:.4} | {:.2} |\n",
                    t.traded_at,
                    t.symbol,
                    t.name,
                    t.transaction_type,
                    t.shares,
                    t.price,
                    t.total_amount
                ));
            }
            out.push('\n');
        }
        _ => out.push_str("（暂无交易记录）\n\n"),
    }

    // ── Performance metrics (last 1 year) ──────────────────────────────────
    out.push_str("## 绩效指标（近 1 年）\n");
    let end = Utc::now().date_naive();
    let start = end - Duration::days(365);
    let filter = PerformanceFilter {
        market: None,
        account_id: None,
    };
    match performance_service::get_performance_summary(db, start, end, &filter) {
        Ok(p) if p.end_value > 0.0 || !p.return_series.is_empty() => {
            let sharpe = p
                .sharpe_ratio
                .map(|value| format!("{:.2}", value))
                .unwrap_or_else(|| "—".to_string());
            out.push_str(&format!(
                "- 期初市值：{:.2}\n- 期末市值：{:.2}\n- 累计收益率：{:.2}%\n- 年化收益率：{:.2}%\n- 累计盈亏：{:.2}\n- 最大回撤：{:.2}%\n- 波动率：{:.2}%\n- 夏普比率：{}\n\n",
                p.start_value,
                p.end_value,
                p.total_return,
                p.annualized_return,
                p.total_pnl,
                p.max_drawdown,
                p.volatility,
                sharpe,
            ));
        }
        _ => out.push_str("（暂无足够的历史数据）\n\n"),
    }

    Ok(out.trim_end().to_string())
}

struct TxnRow {
    traded_at: String,
    symbol: String,
    name: String,
    transaction_type: String,
    shares: f64,
    price: f64,
    total_amount: f64,
}

fn fetch_recent_transactions(db: &Database, limit: usize) -> Result<Vec<TxnRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT traded_at, symbol, name, transaction_type, shares, price, total_amount
             FROM transactions
             ORDER BY traded_at DESC
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![limit as i64], |row| {
            Ok(TxnRow {
                traded_at: row.get(0)?,
                symbol: row.get(1)?,
                name: row.get(2)?,
                transaction_type: row.get(3)?,
                shares: row.get(4)?,
                price: row.get(5)?,
                total_amount: row.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

// ─────────────────────────────────────────────────────────────────────────────
// Chat streaming
// ─────────────────────────────────────────────────────────────────────────────

/// OpenAI-style streaming chunk.
#[derive(Debug, Deserialize)]
struct ChatStreamChunk {
    #[serde(default)]
    choices: Vec<ChatChoice>,
    /// Only present on the final chunk when `stream_options.include_usage`
    /// is set on the request.
    #[serde(default)]
    usage: Option<UsagePayload>,
    /// Some providers return an inline `error` object inside the SSE stream
    /// (instead of an HTTP error status). Without this field serde silently
    /// parses it as an empty-but-successful chunk, making it look like the
    /// model "returned nothing" with no explanation.
    #[serde(default)]
    error: Option<StreamError>,
}

/// Inline error payload some providers embed in SSE chunks.
#[derive(Debug, Deserialize)]
struct StreamError {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    r#type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    #[serde(default)]
    delta: Delta,
    /// `"stop"` (final text), `"tool_calls"` (wants to call tools), or `None`
    /// mid-stream. Only present on the chunk that ends a choice.
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    /// Reasoning-model chain-of-thought (DeepSeek-R1 `reasoning_content`,
    /// GLM-4.5 `reasoning_content`, OpenAI o-series would be separate). We do
    /// NOT stream this to the user (it's internal scratch), but we track whether
    /// any arrived so we can distinguish "model produced nothing" (real error)
    /// from "model only produced reasoning, no final answer" (a different fix).
    #[serde(default)]
    reasoning_content: Option<String>,
    /// Tool-call deltas. These arrive across multiple chunks; the model streams
    /// the `arguments` JSON in pieces, so we accumulate by `index`.
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Debug, Deserialize)]
struct ToolCallDelta {
    /// Positional index within the model's tool-call list. Used as the
    /// accumulation key (the same call's arguments arrive split across many
    /// chunks, all sharing this index).
    #[serde(default)]
    index: u32,
    /// Only present on the first chunk for this call.
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ToolCallFunctionDelta>,
}

#[derive(Debug, Default, Deserialize)]
struct ToolCallFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    /// A fragment of the JSON arguments string. Concatenated across chunks.
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsagePayload {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
    /// OpenAI / OpenRouter / Kimi style: nested under `prompt_tokens_details`.
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
    /// DeepSeek style: top-level `prompt_cache_hit_tokens`.
    #[serde(default)]
    prompt_cache_hit_tokens: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: u32,
}

impl UsagePayload {
    fn into_chat_usage(self) -> ChatUsage {
        // Take the larger of the two cache fields so we tolerate either schema
        // (and providers that only populate one). When neither is present the
        // value is 0, which the UI renders as "no cache".
        let cached = self
            .prompt_tokens_details
            .map(|d| d.cached_tokens)
            .unwrap_or(0)
            .max(self.prompt_cache_hit_tokens.unwrap_or(0));
        ChatUsage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens: self.total_tokens,
            cached_tokens: cached,
        }
    }
}

/// Strip the tools section from the system prompt when tools are disabled.
/// Removes the "# 你可用的工具" section so the model doesn't try to call
/// tools that aren't available. The section starts with that heading and ends
/// at the next `#` heading.
fn strip_tools_from_prompt(prompt: &str) -> String {
    let marker = "# 你可用的工具";
    if let Some(start) = prompt.find(marker) {
        // Find the next `# ` heading after the tools section.
        let after_marker = &prompt[start..];
        if let Some(next_heading) = after_marker[1..].find("\n# ") {
            let end = start + 1 + next_heading + 1;
            // Remove the tools section, keeping the rest intact.
            let before = &prompt[..start];
            let after = &prompt[end..];
            format!("{}{}", before.trim_end(), after)
        } else {
            // Tools section is the last section — just drop it.
            prompt[..start].trim_end().to_string()
        }
    } else {
        prompt.to_string()
    }
}

/// Validate that the saved config is usable, returning the config on success.
fn load_and_validate_config(db: &Database) -> Result<crate::models::ai_config::AiConfig, String> {
    let cfg = ai_config_service::get_ai_config(db)?;
    if cfg.model.trim().is_empty() {
        return Err("尚未配置 AI 模型，请先到「设置 → AI 配置」中选择模型".to_string());
    }
    // Local Ollama does not need an API key.
    if !cfg.provider.eq_ignore_ascii_case("ollama") && cfg.api_key.trim().is_empty() {
        return Err("尚未配置 API Key，请先到「设置 → AI 配置」中填写".to_string());
    }
    Ok(cfg)
}

/// A fully-assembled tool call, after streaming fragments are merged by index.
/// This is the shape we hand back to `execute_tool` and also serialise into
/// the `assistant` tool_calls message for the next round.
#[derive(Debug, Clone)]
struct AssembledToolCall {
    id: String,
    function_name: String,
    arguments: String,
}

/// Lifecycle event for a single tool invocation, sent to the frontend via the
/// `ai-chat-tool-call` Tauri event so the UI can render a Claude-style
/// expandable tool card with live status, arguments, and results.
///
/// Emitted twice per call: once with `Running` before `execute_tool` runs, and
/// again with `Success` (carrying `result`) or `Error` (carrying `error`) plus
/// `duration_ms` when it finishes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallEvent {
    /// Stable id (the model's `tool_call_id`). The frontend upserts cards by it.
    pub id: String,
    /// Function name, e.g. `get_stock_quote`.
    pub name: String,
    /// Raw JSON arguments string the model supplied (best-effort; may be empty).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    pub status: ToolCallStatus,
    /// Tool result JSON (success only). Truncated to keep IPC payloads small.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Error message (error only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Wall-clock execution time in milliseconds (success/error only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolCallStatus {
    Running,
    Success,
    Error,
}

/// Cap on the size of a tool `result` payload we ship to the UI, to keep IPC
/// payloads and React re-renders cheap. The model still receives the full
/// result; this only trims what is shown in the expandable card.
const TOOL_RESULT_DISPLAY_LIMIT: usize = 2000;

/// Truncate a tool result string for display, appending an ellipsis marker if
/// it was cut. Keeps complete JSON-ish content readable.
fn truncate_for_display(s: &str) -> String {
    if s.chars().count() <= TOOL_RESULT_DISPLAY_LIMIT {
        s.to_string()
    } else {
        let cut: String = s.chars().take(TOOL_RESULT_DISPLAY_LIMIT).collect();
        format!("{cut}\n…（结果已截断，完整数据已发送给模型）")
    }
}

/// Merge streaming tool-call deltas (keyed by `index`) into a list of complete
/// calls. Pure function so it can be unit-tested in isolation.
///
/// Providers split a single tool call across many chunks: the first chunk
/// carries `id` + `function.name`, and subsequent chunks carry fragments of
/// `function.arguments` that must be concatenated in order. We accumulate per
/// index using a `BTreeMap` so the output order matches the model's intent.
fn merge_tool_calls(deltas: &[ToolCallDelta]) -> Vec<AssembledToolCall> {
    let mut map: std::collections::BTreeMap<u32, AssembledToolCall> =
        std::collections::BTreeMap::new();
    for d in deltas {
        let entry = map.entry(d.index).or_insert_with(|| AssembledToolCall {
            id: String::new(),
            function_name: String::new(),
            arguments: String::new(),
        });
        if let Some(id) = &d.id {
            entry.id = id.clone();
        }
        if let Some(f) = &d.function {
            if let Some(name) = &f.name {
                entry.function_name = name.clone();
            }
            if let Some(args) = &f.arguments {
                entry.arguments.push_str(args);
            }
        }
    }
    map.into_values().collect()
}

/// Outcome of consuming one streaming request round. The loop in `chat_stream`
/// inspects `finish_reason` to decide whether to execute tools and run another
/// round, or finish.
struct RoundOutcome {
    /// `"tool_calls"` if the model wants to call tools, otherwise treated as a
    /// final answer.
    finish_reason: Option<String>,
    /// Assembled tool calls (empty unless `finish_reason == "tool_calls"`).
    tool_calls: Vec<AssembledToolCall>,
    /// Usage from the final chunk, if the provider sent one this round.
    usage: Option<ChatUsage>,
    /// True if the user pressed stop mid-round.
    stopped: bool,
    /// Whether ANY content delta was emitted to the frontend this round. Used
    /// by the loop to detect an empty final reply (model returned `stop` but
    /// produced zero visible text — usually a provider/model mismatch).
    emitted_any_content: bool,
    /// The full text emitted this round (all content deltas concatenated).
    /// Used to detect a truncated reply that looks like an unfulfilled tool
    /// intent (e.g. "我来" then stop with no tool calls).
    emitted_text: String,
    /// Whether the model emitted reasoning_content (chain-of-thought) this
    /// round without a final answer. Helps produce an accurate diagnostic.
    had_reasoning_only: bool,
    /// Total characters of reasoning_content emitted this round. Diagnostic
    /// only — helps distinguish "model reasoned briefly then stalled" from
    /// "model produced a huge chain-of-thought that exhausted the token budget".
    reasoning_chars: usize,
    /// The full reasoning_content text (chain-of-thought), used as a fallback
    /// answer when the model produces reasoning but no final `content`.
    reasoning_text: String,
    /// Total SSE data chunks processed this round. Very low counts (1-2)
    /// indicate the stream ended almost immediately — a provider/model issue.
    chunk_count: usize,
    /// Whether the stream ended cleanly via `[DONE]`. When false, the stream
    /// was truncated (provider error, token limit, or network issue) — the
    /// caller should treat the round as incomplete rather than a normal stop.
    stream_completed: bool,
}

/// Send one streaming request and drain its SSE stream.
///
/// Text deltas are emitted to the frontend as they arrive (`ai-chat-delta`).
/// Tool-call deltas and the finish reason are accumulated and returned so the
/// caller can decide on the next round. Network/parse errors are returned as
/// `Err` (the caller emits `ai-chat-error`).
async fn stream_one_round(
    app: &AppHandle,
    cfg: &crate::models::ai_config::AiConfig,
    url: &str,
    body: serde_json::Value,
) -> Result<RoundOutcome, String> {
    let client = http_client::ai_client();
    let mut req = client.post(url).json(&body);
    if !cfg.api_key.is_empty() {
        req = req.bearer_auth(&cfg.api_key);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("请求 AI 服务失败：{e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("AI 服务返回错误 (HTTP {status})：{body}"));
    }

    // Parse the SSE stream. Each chunk is a slice of bytes; we buffer into a
    // string and split on `\n`, processing one SSE `data:` line at a time.
    let mut stream = resp;
    let mut buf = String::new();
    let mut last_usage: Option<ChatUsage> = None;
    let mut pending_deltas: Vec<ToolCallDelta> = Vec::new();
    let mut finish_reason: Option<String> = None;
    let mut stopped = false;
    let mut emitted_any_content = false;
    let mut emitted_text = String::new();
    let mut had_reasoning = false;
    let mut reasoning_char_count: usize = 0;
    let mut reasoning_text = String::new();
    let mut chunk_count: usize = 0;
    // Track whether we received the explicit `[DONE]` SSE marker. This is the
    // authoritative signal that the model finished generating. `Ok(None)` from
    // `stream.chunk()` means the TCP connection closed — which is usually a
    // clean end but can also mean truncation. We only set `stream_completed`
    // when we see `[DONE]`; `Ok(None)` sets a separate `stream_eof` flag.
    let mut received_done = false;
    let mut stream_eof = false;

    loop {
        if STOP_REQUESTED.load(Ordering::SeqCst) {
            stopped = true;
            break;
        }

        match stream.chunk().await {
            Ok(Some(chunk)) => {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                let mut newline_idx;
                while {
                    newline_idx = buf.find('\n');
                    newline_idx.is_some()
                } {
                    let newline_idx = newline_idx.unwrap();
                    let line = buf[..newline_idx].trim_end_matches('\r').to_string();
                    buf.drain(..=newline_idx);

                    let Some(payload) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let payload = payload.trim();
                    if payload == "[DONE]" {
                        received_done = true;
                        break;
                    }
                    if payload.is_empty() {
                        continue;
                    }
                    match serde_json::from_str::<ChatStreamChunk>(payload) {
                        Ok(parsed) => {
                            if let Some(err) = parsed.error {
                                let msg = err.message.unwrap_or_else(|| "未知流式错误".to_string());
                                let code = err.code.unwrap_or_default();
                                let etype = err.r#type.unwrap_or_default();
                                warn!(
                                    target: "ai_chat",
                                    "provider stream error: {msg} (code={code}, type={etype})"
                                );
                                return Err(format!(
                                    "AI 服务流式返回错误：{msg}（{}{}）",
                                    code,
                                    if etype.is_empty() {
                                        String::new()
                                    } else {
                                        format!("，类型 {}", etype)
                                    },
                                ));
                            }
                            if let Some(u) = parsed.usage {
                                last_usage = Some(u.into_chat_usage());
                            }
                            chunk_count += 1;
                            for choice in parsed.choices {
                                if let Some(fr) = choice.finish_reason {
                                    finish_reason = Some(fr);
                                }
                                if let Some(content) = choice.delta.content {
                                    if !content.is_empty() {
                                        emitted_any_content = true;
                                        emitted_text.push_str(&content);
                                        let _ = app.emit("ai-chat-delta", content);
                                    }
                                }
                                if let Some(rc) = choice.delta.reasoning_content {
                                    if !rc.is_empty() {
                                        had_reasoning = true;
                                        reasoning_char_count += rc.chars().count();
                                        reasoning_text.push_str(&rc);
                                        // Stream the chain-of-thought to the frontend so the
                                        // UI can render a collapsible "思考过程" block live,
                                        // matching the Claude/ZCode reasoning experience.
                                        let _ = app.emit("ai-chat-reasoning", rc);
                                    }
                                }
                                if !choice.delta.tool_calls.is_empty() {
                                    pending_deltas.extend(choice.delta.tool_calls);
                                }
                            }
                        }
                        Err(e) => {
                            debug!(target: "ai_chat", "skip unparseable SSE line: {e} :: {payload}");
                        }
                    }
                }
                // Exit outer loop when [DONE] is received (inner loop broke
                // on it). We don't check `buf.is_empty()` here — the buffer
                // may still have trailing data from the same chunk, but [DONE]
                // is the authoritative end signal.
                if received_done {
                    break;
                }
            }
            Ok(None) => {
                // TCP connection closed. This is the normal end-of-stream for
                // providers that don't send [DONE] (e.g. DeepSeek). We treat
                // this as a valid stream end, NOT as truncation. The caller
                // checks `received_done || stream_eof` to decide.
                stream_eof = true;
                break;
            }
            Err(e) => {
                return Err(format!("读取 AI 响应流失败：{e}"));
            }
        }
    }

    // `stream_completed` is true when the stream ended cleanly — either via
    // `[DONE]` or via `Ok(None)` (TCP close). The caller uses this to
    // distinguish "model finished" from "stream was cut short by an error".
    // An actual error would have returned `Err` above, so reaching here means
    // the stream ended normally (even without `[DONE]`).
    let stream_completed = received_done || stream_eof;

    // Assemble tool calls from the accumulated deltas. We do NOT gate this on
    // `finish_reason == "tool_calls"` because providers are inconsistent: GLM,
    // DeepSeek, and others sometimes emit tool_calls with `finish_reason: "stop"`
    // or even `null`. If we only honored the exact "tool_calls" string, those
    // calls would be silently dropped and the model's reply (e.g. "我来查一下…")
    // would be treated as the final answer — the "truncated to two words" bug.
    // The downstream `wants_tools` check validates the assembled list is
    // non-empty, so assembling unconditionally is safe.
    let tool_calls = merge_tool_calls(&pending_deltas);

    Ok(RoundOutcome {
        finish_reason,
        tool_calls,
        usage: last_usage,
        stopped,
        emitted_any_content,
        emitted_text,
        had_reasoning_only: had_reasoning && !emitted_any_content,
        reasoning_chars: reasoning_char_count,
        reasoning_text,
        chunk_count,
        stream_completed,
    })
}

/// Stream a chat completion to the frontend via Tauri events.
///
/// This is an iterative tool-calling loop: each round sends the conversation
/// (plus the advertised `tools`), streams back text deltas live, and if the
/// model asks to call tools, executes them and sends another round with the
/// results. Up to [`MAX_TOOL_ROUNDS`] rounds run before we give up and return
/// whatever text we have.
///
/// Events emitted:
/// - `ai-chat-delta` (payload: `String`) — one token delta at a time
/// - `ai-chat-reasoning` (payload: `String`) — one chain-of-thought token
///   delta at a time (DeepSeek-R1 / GLM-4.5+ reasoning_content)
/// - `ai-chat-skill` (payload: `Vec<String>`) — activated skill names (once)
/// - `ai-chat-tool` (payload: `Vec<String>`) — tool names being run this round
/// - `ai-chat-tool-call` (payload: `ToolCallEvent`) — per-tool progress:
///   emitted with `status:"running"` before execution and again with
///   `status:"success"`/`"error"` + result/duration after. Lets the UI render
///   Claude-style expandable tool cards.
/// - `ai-chat-usage` (payload: `ChatUsage`) — token accounting (final round)
/// - `ai-chat-done` (payload: `()`) — successful completion
/// - `ai-chat-error` (payload: `String`) — any failure (also returned as `Err`)
pub async fn chat_stream(
    app: AppHandle,
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    params: ChatParams,
) -> Result<(), String> {
    // Anthropic uses the native Messages API, not OpenAI-compatible
    // /chat/completions — dispatch to the dedicated implementation.
    let cfg_provider = ai_config_service::get_ai_config(db)
        .map(|c| c.provider)
        .unwrap_or_default();
    if cfg_provider.eq_ignore_ascii_case("anthropic") {
        return chat_stream_anthropic(app, db, cache, quote_cache, params).await;
    }

    let emit_error = |app: &AppHandle, msg: String| {
        let _ = app.emit("ai-chat-error", msg);
    };
    let emit_usage = |app: &AppHandle, usage: &Option<ChatUsage>| {
        if let Some(u) = usage {
            let _ = app.emit("ai-chat-usage", u.clone());
        }
    };

    let cfg = match load_and_validate_config(db) {
        Ok(c) => c,
        Err(e) => {
            emit_error(&app, e.clone());
            return Err(e);
        }
    };
    let base = match resolve_base_url(&cfg.provider, cfg.base_url.as_deref()) {
        Ok(b) => b,
        Err(e) => {
            emit_error(&app, e.clone());
            return Err(e);
        }
    };
    let url = format!("{base}/chat/completions");

    // Resolve which skills apply to this turn: the user's explicit selection
    // takes priority; otherwise we auto-activate any skill whose trigger
    // keyword appears in the latest user message. We also tell the frontend
    // which skills ended up active so the UI can show a badge.
    let activated = resolve_active_skills(&app, &params);
    if !activated.is_empty() {
        let names: Vec<String> = activated.iter().map(|s| s.name.clone()).collect();
        let _ = app.emit("ai-chat-skill", names);
    }

    // Build the full message list: system prompt, optional context, then the
    // conversation history from the frontend. This list is mutated across
    // rounds (tool calls + results are appended), so it lives outside the loop.
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if !cfg.system_prompt.trim().is_empty() {
        // When tools are disabled, strip the tools section from the system
        // prompt so the model doesn't try to call tools that don't exist.
        let prompt = if cfg.tools_enabled {
            cfg.system_prompt.clone()
        } else {
            strip_tools_from_prompt(&cfg.system_prompt)
        };
        messages.push(json!({ "role": "system", "content": prompt }));
    }
    // Active skills are injected as an extra system message so the model
    // treats them as authoritative instructions on top of its base persona.
    let skill_block = build_skill_system_message(&activated);
    if !skill_block.is_empty() {
        messages.push(json!({ "role": "system", "content": skill_block }));
    }
    if params.include_context {
        match build_portfolio_context(db, cache, quote_cache).await {
            Ok(ctx) => {
                messages.push(json!({
                    "role": "system",
                    "content": format!(
                        "以下是用户的实时投资组合数据，请在回答时参考（金额单位均为 USD，数据可能略有延迟）：\n\n{ctx}"
                    ),
                }));
            }
            // Context is best-effort; a failure should not block the chat.
            Err(e) => {
                warn!(target: "ai_chat", "failed to build portfolio context: {e}");
            }
        }
    }
    for m in &params.messages {
        messages.push(json!({ "role": m.role, "content": m.content }));
    }

    // The tools we advertise to the model on every round. Built once.
    // Only included when the user has enabled tools in settings — some models
    // (DeepSeek-v4-flash, local Ollama) don't support function calling and
    // return empty replies when `tools` is present.
    let tools = if cfg.tools_enabled {
        crate::services::ai_tools::tool_definitions()
    } else {
        Vec::new()
    };

    // Start each turn with a clean stop flag so a stale request from a
    // previous turn doesn't immediately abort this one.
    STOP_REQUESTED.store(false, Ordering::SeqCst);

    let tool_ctx = crate::services::ai_tools::ToolCtx::for_untrusted_model_turn(
        db,
        cache,
        quote_cache,
        latest_user_message(&params),
    );

    // ── Agentic tool-calling loop ──────────────────────────────────────────
    //
    // This is a resilient loop modelled after Claude/Codex's agentic pattern:
    //
    //   1. Send the conversation (with tools) to the model, stream the response.
    //   2. If the stream is truncated mid-response → retry this round (the
    //      model may have been mid-tool-call; retrying lets it start fresh).
    //   3. If the model returned tool calls → execute them, append results,
    //      and loop back to step 1 so the model can reason over the data.
    //   4. If the model returned a final answer (no tool calls) → done.
    //   5. If the user pressed stop → emit whatever we have and stop.
    //
    // Key design choices (vs the old fragile pipeline):
    //   - Stream truncation is a RECOVERABLE error, not a fatal one. We retry
    //     the round up to `MAX_STREAM_RETRIES` times before giving up.
    //   - Network/HTTP errors abort the entire turn (non-recoverable).
    //   - Empty replies after a clean stream are reported as errors (the model
    //     chose to say nothing — likely a provider/model mismatch).
    //   - Tool calls are accumulated across rounds; each round's assistant
    //     message + tool results are appended to `messages` so the model has
    //     full context on the next round.

    /// Maximum retries for a single round when the stream is truncated. We
    /// allow a few retries because truncation is often transient (provider
    /// hiccup, temporary rate limit). After this many retries, we give up and
    /// report the error.
    const MAX_STREAM_RETRIES: usize = 2;

    let mut last_usage: Option<ChatUsage> = None;

    // Build the request body. `with_tools` controls whether the `tools` /
    // `tool_choice` fields are included. The first round always respects the
    // user's config; a fallback round (after an empty reply from a model that
    // likely doesn't support function calling) passes `with_tools=false`.
    //
    // When `with_tools=false` we ALSO strip the "# 你可用的工具" section from
    // the base system prompt — otherwise the model is told it has tools but
    // isn't given any, which is exactly the confusion that causes empty
    // replies. The first system message is the base prompt (added before the
    // skill block and context); we transform a copy, not the originals.
    //
    // This is a free function (not a closure) so it borrows `messages` only
    // for the duration of the call — the loop can then mutate `messages`
    // (e.g. append tool results) without a borrow conflict.
    let build_body = |msgs: &Vec<serde_json::Value>, with_tools: bool| -> serde_json::Value {
        if with_tools && cfg.tools_enabled {
            json!({
                "model": cfg.model,
                "messages": msgs,
                "stream": true,
                "tools": tools,
                "tool_choice": "auto",
                "stream_options": { "include_usage": true },
            })
        } else {
            // Strip the tools section from the base system prompt so the model
            // isn't told about tools it won't receive. Only the first system
            // message (the base persona prompt) is transformed; skill/context
            // messages pass through unchanged.
            let messages_for_body: Vec<serde_json::Value> = msgs
                .iter()
                .enumerate()
                .map(|(i, m)| {
                    if i == 0 && m.get("role").and_then(|r| r.as_str()) == Some("system") {
                        if let Some(content) = m.get("content").and_then(|c| c.as_str()) {
                            let mut copy = m.clone();
                            copy["content"] = json!(strip_tools_from_prompt(content));
                            return copy;
                        }
                    }
                    m.clone()
                })
                .collect();
            json!({
                "model": cfg.model,
                "messages": messages_for_body,
                "stream": true,
                "stream_options": { "include_usage": true },
            })
        }
    };

    // Tracks whether we've already attempted the tools-stripped fallback this
    // turn. We retry at most once: if the model still returns empty without
    // tools, the issue is elsewhere (model choice, content filter) and a hard
    // error with a clear diagnostic is more honest than silent looping.
    let mut tried_without_tools = false;

    for round in 0..crate::services::ai_tools::MAX_TOOL_ROUNDS {
        let body = build_body(&messages, !tried_without_tools);

        // ── Stream with retry on truncation ────────────────────────────────
        // If the stream is truncated (no `[DONE]`, no clean EOF), we retry
        // the same request. This handles transient provider issues without
        // losing the entire turn. Non-stream errors (HTTP, network) are
        // fatal and abort the turn immediately.
        let mut outcome = None;
        for attempt in 0..=MAX_STREAM_RETRIES {
            match stream_one_round(&app, &cfg, &url, body.clone()).await {
                Ok(o) => {
                    if o.stream_completed || o.stopped || !o.tool_calls.is_empty() {
                        // Clean end, user stop, or tool calls — no retry needed.
                        outcome = Some(o);
                        break;
                    }
                    // Stream ended but no content, no tool calls, and not stopped.
                    // This could be a truncation OR an empty reply. If we got
                    // some chunks but no finish_reason, it's likely truncation.
                    if o.chunk_count > 0 && o.finish_reason.is_none() {
                        warn!(
                            target: "ai_chat",
                            round, attempt,
                            chunks = o.chunk_count,
                            "stream ended without [DONE] or finish_reason — retrying"
                        );
                        // Don't emit any deltas from this failed attempt — the
                        // caller will see the retried version. But we can't
                        // "un-emit" deltas already sent. To avoid duplicate
                        // content on retry, we track that we should skip
                        // emitting on retry attempts. For simplicity, we just
                        // accept that the first attempt's partial content may
                        // have been emitted — the retry will produce the full
                        // response and the user will see the final version.
                        if attempt < MAX_STREAM_RETRIES {
                            continue;
                        }
                    }
                    // Clean end with no content — this is an empty reply, not
                    // truncation. Don't retry; report it below.
                    outcome = Some(o);
                    break;
                }
                Err(e) => {
                    // Network/HTTP errors are fatal — don't retry.
                    warn!(target: "ai_chat", round, "stream error: {e}");
                    emit_usage(&app, &last_usage);
                    emit_error(&app, e.clone());
                    return Err(e);
                }
            }
        }

        let outcome = match outcome {
            Some(o) => o,
            None => {
                // All retries exhausted — the stream kept getting truncated.
                let diag = "AI 响应在多次重试后仍然中断。最常见原因是当前模型不支持函数调用（function calling），请切换到其他模型重试。".to_string();
                warn!(target: "ai_chat", round, "all retries exhausted");
                emit_usage(&app, &last_usage);
                emit_error(&app, diag);
                return Err("AI 响应流持续截断".to_string());
            }
        };

        info!(
            target: "ai_chat",
            round,
            finish_reason = ?outcome.finish_reason,
            tool_calls = outcome.tool_calls.len(),
            emitted_content = outcome.emitted_any_content,
            reasoning_chars = outcome.reasoning_chars,
            chunks = outcome.chunk_count,
            stopped = outcome.stopped,
            stream_completed = outcome.stream_completed,
            "round done"
        );

        if let Some(u) = outcome.usage.clone() {
            last_usage = Some(u);
        }

        // ── User stop ──────────────────────────────────────────────────────
        if outcome.stopped {
            emit_usage(&app, &last_usage);
            let _ = app.emit("ai-chat-done", ());
            return Ok(());
        }

        // ── Model finished (no tool calls) → done ──────────────────────────
        let wants_tools = !outcome.tool_calls.is_empty();
        if !wants_tools {
            // Empty reply detection: the stream completed but produced nothing.
            if !outcome.emitted_any_content {
                // Reasoning-only fallback: the model produced chain-of-thought
                // but no final answer. Show the reasoning with a note.
                if outcome.had_reasoning_only && !outcome.reasoning_text.is_empty() {
                    let note = "\n\n---\n\n> ⚠️ 本次模型只输出了思考过程，未生成最终回答（可能因输出长度限制）。如需完整回答，请重试或切换到非推理模型。";
                    let fallback = format!(
                        "**思考过程：**\n\n{}{}",
                        outcome.reasoning_text.trim(),
                        note
                    );
                    let _ = app.emit("ai-chat-delta", fallback);
                    emit_usage(&app, &last_usage);
                    let _ = app.emit("ai-chat-done", ());
                    return Ok(());
                }
                // Auto-fallback: the most common cause of an empty reply is the
                // model silently ignoring the `tools` field (it doesn't support
                // function calling). A manual retry would re-send the same
                // `tools` and get the same empty result — so we transparently
                // retry once WITHOUT tools. This frequently succeeds and spares
                // the user a manual retry that wouldn't have helped.
                if cfg.tools_enabled && !tried_without_tools {
                    tried_without_tools = true;
                    warn!(
                        target: "ai_chat",
                        round,
                        "empty reply with tools — retrying without tools (likely unsupported function calling)"
                    );
                    continue;
                }
                let diag = explain_empty_reply(&outcome);
                warn!(target: "ai_chat", round, "empty reply — {diag}");
                emit_usage(&app, &last_usage);
                emit_error(&app, diag);
                return Err("AI 返回了空回复".to_string());
            }

            // Stalled tool intent: the model started to say "let me look that
            // up" but never emitted tool calls. This is a model capability
            // issue, not a transient error. Same auto-fallback as above: strip
            // tools and retry once so the model answers directly instead of
            // stalling on an intent it can't fulfill.
            if outcome.tool_calls.is_empty()
                && looks_like_unfulfilled_tool_intent(&outcome.emitted_text)
            {
                if cfg.tools_enabled && !tried_without_tools {
                    tried_without_tools = true;
                    warn!(
                        target: "ai_chat",
                        round,
                        "stalled tool intent — retrying without tools"
                    );
                    continue;
                }
                let diag = format!(
                    "模型输出了「{}」后停止，未发起工具调用。最常见原因是当前模型不支持函数调用（function calling）。",
                    outcome.emitted_text.trim(),
                );
                warn!(target: "ai_chat", round, "stalled tool intent — {diag}");
                emit_usage(&app, &last_usage);
                emit_error(&app, diag);
                return Err("AI 工具调用未完成".to_string());
            }

            // Normal completion — the model produced content and finished.
            emit_usage(&app, &last_usage);
            let _ = app.emit("ai-chat-done", ());
            return Ok(());
        }

        // ── Model wants to call tools → execute and continue ────────────────
        // Record the assistant message with tool_calls (required by the API
        // so the next round can reference them by id).
        let tool_calls_json: Vec<serde_json::Value> = outcome
            .tool_calls
            .iter()
            .map(|tc| {
                json!({
                    "id": tc.id,
                    "type": "function",
                    "function": {
                        "name": tc.function_name,
                        "arguments": tc.arguments,
                    }
                })
            })
            .collect();
        messages.push(json!({
            "role": "assistant",
            "tool_calls": tool_calls_json,
        }));

        // Tell the UI which tools are running.
        let tool_names: Vec<String> = outcome
            .tool_calls
            .iter()
            .map(|tc| tc.function_name.clone())
            .collect();
        let _ = app.emit("ai-chat-tool", tool_names);

        // Execute each tool and append results. Tool execution errors are
        // returned as error JSON in the tool result — the model sees them and
        // can decide how to handle (apologise, retry, etc.). We never abort
        // the turn because a tool failed.
        //
        // For each call we also emit a `ai-chat-tool-call` event before and
        // after execution so the UI can render a Claude-style expandable tool
        // card with live status, arguments, and results.
        for tc in &outcome.tool_calls {
            // Tell the UI this tool is starting (so a spinner can show).
            let args_for_ui = if tc.arguments.trim().is_empty() {
                None
            } else {
                Some(tc.arguments.clone())
            };
            let _ = app.emit(
                "ai-chat-tool-call",
                ToolCallEvent {
                    id: tc.id.clone(),
                    name: tc.function_name.clone(),
                    arguments: args_for_ui.clone(),
                    status: ToolCallStatus::Running,
                    result: None,
                    error: None,
                    duration_ms: None,
                },
            );

            let started = std::time::Instant::now();
            let result = crate::services::ai_tools::execute_tool(
                &tool_ctx,
                &tc.function_name,
                &tc.arguments,
            )
            .await;
            let duration_ms = started.elapsed().as_millis() as u64;

            messages.push(json!({
                "role": "tool",
                "tool_call_id": tc.id,
                "content": result.content,
            }));

            // Report the outcome to the UI. The model always gets the full
            // `content`; the event payload is truncated for display only.
            if result.ok {
                let _ = app.emit(
                    "ai-chat-tool-call",
                    ToolCallEvent {
                        id: tc.id.clone(),
                        name: tc.function_name.clone(),
                        arguments: args_for_ui.clone(),
                        status: ToolCallStatus::Success,
                        result: Some(truncate_for_display(&result.content)),
                        error: None,
                        duration_ms: Some(duration_ms),
                    },
                );
            } else {
                // Surface a short, human-readable error to the card.
                let err_msg = match serde_json::from_str::<serde_json::Value>(&result.content) {
                    Ok(v) => v
                        .get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or(&result.content)
                        .to_string(),
                    Err(_) => result.content.clone(),
                };
                let _ = app.emit(
                    "ai-chat-tool-call",
                    ToolCallEvent {
                        id: tc.id.clone(),
                        name: tc.function_name.clone(),
                        arguments: args_for_ui.clone(),
                        status: ToolCallStatus::Error,
                        result: None,
                        error: Some(truncate_for_display(&err_msg)),
                        duration_ms: Some(duration_ms),
                    },
                );
            }
        }
        // Loop continues → next round sends tool results back to the model.
    }

    // Exhausted the round budget without a final answer.
    warn!(target: "ai_chat", "hit MAX_TOOL_ROUNDS without a final answer");
    emit_usage(&app, &last_usage);
    let _ = app.emit("ai-chat-done", ());
    Ok(())
}

/// Analyse the round outcome and produce a root-cause explanation for the user.
/// The message should answer "why did this happen?" rather than just "what
/// happened?".
fn explain_empty_reply(outcome: &RoundOutcome) -> String {
    // Case 1: stream didn't complete — provider cut the connection.
    if !outcome.stream_completed {
        return format!(
            "响应流在第 {} 个数据块后中断（finish_reason={:?}）。可能是输出 token 限制、服务端异常或网络问题。",
            outcome.chunk_count, outcome.finish_reason,
        );
    }

    // Case 2: stream completed, had reasoning but no final answer — token
    // budget exhausted by chain-of-thought.
    if outcome.had_reasoning_only && outcome.reasoning_chars > 0 {
        return format!(
            "模型输出了 {} 字的思考过程后停止，未生成最终回答。可能是输出长度限制导致。",
            outcome.reasoning_chars,
        );
    }

    // Case 3: stream completed cleanly but zero content and zero tool calls.
    // The most common cause: the model doesn't support the `tools` field and
    // silently ignores it, returning an empty completion.
    if outcome.chunk_count <= 2 && outcome.tool_calls.is_empty() {
        return format!(
            "模型返回了空回复（仅 {} 个数据块，finish_reason={:?}）。最常见原因是当前模型不支持函数调用（function calling）——收到 tools 字段后静默返回空响应。",
            outcome.chunk_count, outcome.finish_reason,
        );
    }

    // Case 4: some chunks arrived but no content — possibly content filtered
    // or the model decided not to respond.
    format!(
        "模型返回了空回复（{} 个数据块，finish_reason={:?}，工具调用 {} 个）。可能是内容被安全过滤，或模型主动拒绝回答。",
        outcome.chunk_count, outcome.finish_reason, outcome.tool_calls.len(),
    )
}

/// Heuristic: does this short reply look like an *unfulfilled tool intent* —
/// i.e. the model started to say "let me look that up" but never actually
/// emitted a tool call? We check for a very short reply (under ~30 chars) that
/// contains a tooling-intent keyword. This catches the common "我来" / "Let me
/// check" stall where a model that can't actually do function calling announces
/// its intent and then stops.
///
/// False positives are harmless: if the reply genuinely is a 2-word complete
/// answer that happens to contain "我来", surfacing a "please retry / switch
/// model" hint is still a better UX than a silently truncated 2-char answer.
fn looks_like_unfulfilled_tool_intent(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.chars().count() > 30 {
        return false;
    }
    // CJK intent markers: 我来（我来查）、帮您查、让我看、查询一下, etc.
    let cjk_markers = [
        "我来",
        "让我",
        "帮您",
        "帮你",
        "查询",
        "查一下",
        "稍等",
        "正在查",
        "正在获取",
        "调取",
    ];
    // ASCII intent markers.
    let ascii_markers = [
        "let me",
        "i'll",
        "i will",
        "checking",
        "looking up",
        "fetching",
        "one moment",
        "just a",
    ];
    let lower = trimmed.to_lowercase();
    cjk_markers.iter().any(|m| trimmed.contains(m))
        || ascii_markers.iter().any(|m| lower.contains(m))
}

/// OpenAI-style non-streaming chat completion response (only what we need).
#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    #[serde(default)]
    choices: Vec<ChatCompletionChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChoice {
    #[serde(default)]
    message: Option<ChatCompletionMessage>,
}

#[derive(Debug, Default, Deserialize)]
struct ChatCompletionMessage {
    #[serde(default)]
    content: Option<String>,
}

/// Ask the configured LLM to produce a short Chinese title summarising the
/// user's first question in a new session. Used to auto-name sessions with
/// something meaningful (e.g. "持仓集中度分析") instead of "新会话 14:30".
///
/// Returns a plain title string on success. On any failure the caller falls
/// back to a truncated prefix of the user message, so this is best-effort.
pub async fn generate_title(db: &Database, user_message: &str) -> Result<String, String> {
    let cfg = load_and_validate_config(db)?;
    let base = resolve_base_url(&cfg.provider, cfg.base_url.as_deref())?;
    let url = format!("{base}/chat/completions");

    let body = json!({
        "model": cfg.model,
        "messages": [
            {
                "role": "system",
                "content": "你是一个标题生成器。根据用户的问题生成一个简短的中文标题。要求：1) 不超过12个字；2) 不要使用标点符号或引号；3) 直接输出标题文字，不要加\"标题:\"等前缀；4) 用主题词概括问题核心。"
            },
            { "role": "user", "content": user_message }
        ],
        // Title generation is cheap — keep the reply short and deterministic.
        "max_tokens": 30,
        "temperature": 0.3,
        "stream": false,
    });

    let client = http_client::ai_client();
    let mut req = client.post(&url).json(&body);
    if !cfg.api_key.is_empty() {
        req = req.bearer_auth(&cfg.api_key);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("标题生成请求失败：{e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("标题生成失败 (HTTP {status})：{body}"));
    }

    let parsed: ChatCompletionResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析标题响应失败：{e}"))?;

    let title = parsed
        .choices
        .into_iter()
        .next()
        .and_then(|c| c.message)
        .and_then(|m| m.content)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "标题响应为空".to_string())?;

    // Sanitise: strip wrapping quotes/backticks the model sometimes adds,
    // collapse internal whitespace, and clamp to a reasonable length so a
    // runaway model can't produce a paragraph.
    let cleaned = title
        .trim_matches(|c: char| {
            c == '"' || c == '\'' || c == '`' || c == '「' || c == '」' || c.is_whitespace()
        })
        .replace(['\n', '\r'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let clamped = if cleaned.chars().count() > 24 {
        cleaned.chars().take(24).collect::<String>()
    } else {
        cleaned
    };
    Ok(clamped)
}

// ─────────────────────────────────────────────────────────────────────────────
// Anthropic Messages API (native Claude protocol)
// ─────────────────────────────────────────────────────────────────────────────

/// Convert the OpenAI-style tool definitions (`{type:"function", function:
/// {name, description, parameters}}`) into Anthropic's `tools` shape
/// (`{name, description, input_schema}`). Pure function, unit-tested.
fn anthropic_tool_definitions(openai_tools: &[serde_json::Value]) -> Vec<serde_json::Value> {
    openai_tools
        .iter()
        .filter_map(|t| {
            let function = t.get("function")?;
            Some(json!({
                "name": function.get("name")?.as_str()?.to_string(),
                "description": function.get("description").and_then(|d| d.as_str()).unwrap_or(""),
                "input_schema": function.get("parameters").cloned().unwrap_or(json!({"type": "object"})),
            }))
        })
        .collect()
}

/// Anthropic SSE `content_block_start` / `content_block_delta` / `message_delta`
/// events. Only the fields we consume are modelled.
#[derive(Debug, Deserialize)]
struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    event_type: String,
    // content_block_start
    #[serde(default)]
    content_block: Option<AnthropicContentBlock>,
    // content_block_delta
    #[serde(default)]
    delta: Option<AnthropicDelta>,
    // message_delta
    #[serde(default)]
    usage: Option<AnthropicUsage>,
    // message_start
    #[serde(default)]
    message: Option<AnthropicMessageStart>,
    // error
    #[serde(default)]
    error: Option<AnthropicApiError>,
}

#[derive(Debug, Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicDelta {
    #[serde(rename = "type", default)]
    delta_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    partial_json: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct AnthropicMessageStart {
    #[serde(default)]
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
struct AnthropicApiError {
    #[serde(default)]
    message: Option<String>,
}

/// One assembled Anthropic tool use (`tool_use` content block).
#[derive(Debug, Clone)]
struct AnthropicToolUse {
    id: String,
    name: String,
    arguments: String,
}

/// Stream an Anthropic Messages API turn. Mirrors `chat_stream`'s agentic
/// tool-calling loop but with the native `/v1/messages` protocol: `x-api-key`
/// auth, `system` as a top-level field, `content` as typed blocks, and SSE
/// events (`content_block_start` / `content_block_delta` / `message_stop`).
async fn chat_stream_anthropic(
    app: AppHandle,
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    params: ChatParams,
) -> Result<(), String> {
    let emit_error = |app: &AppHandle, msg: String| {
        let _ = app.emit("ai-chat-error", msg);
    };

    let cfg = load_and_validate_config(db)?;
    let base = match resolve_base_url(&cfg.provider, cfg.base_url.as_deref()) {
        Ok(b) => b,
        Err(e) => return Err(e),
    };
    // Native Messages endpoint; user-provided base may already include /v1.
    let url = if base.ends_with("/v1") {
        format!("{base}/messages")
    } else {
        format!("{base}/v1/messages")
    };

    // Resolve skills (same as the OpenAI path).
    let activated = resolve_active_skills(&app, &params);
    if !activated.is_empty() {
        let names: Vec<String> = activated.iter().map(|s| s.name.clone()).collect();
        let _ = app.emit("ai-chat-skill", names);
    }

    // ── Build the conversation as Anthropic messages ────────────────────────
    // system is a top-level field; user/assistant content is a list of text
    // blocks. Tool results are appended as user messages with tool_result
    // blocks during the loop.
    let mut system_parts: Vec<String> = Vec::new();
    if !cfg.system_prompt.trim().is_empty() {
        let prompt = if cfg.tools_enabled {
            cfg.system_prompt.clone()
        } else {
            strip_tools_from_prompt(&cfg.system_prompt)
        };
        system_parts.push(prompt);
    }
    let skill_block = build_skill_system_message(&activated);
    if !skill_block.is_empty() {
        system_parts.push(skill_block);
    }
    if params.include_context {
        match build_portfolio_context(db, cache, quote_cache).await {
            Ok(ctx) => system_parts.push(format!(
                "以下是用户的实时投资组合数据，请在回答时参考（金额单位均为 USD，数据可能略有延迟）：\n\n{ctx}"
            )),
            Err(e) => warn!(target: "ai_chat", "failed to build portfolio context: {e}"),
        }
    }

    // messages: Vec<Value> where each is {role, content: [...]} (or a
    // tool_result user message). Mutated across rounds.
    let mut messages: Vec<serde_json::Value> = Vec::new();
    for m in &params.messages {
        messages.push(json!({
            "role": m.role,
            "content": [ { "type": "text", "text": m.content } ],
        }));
    }

    let tools = if cfg.tools_enabled {
        anthropic_tool_definitions(&crate::services::ai_tools::tool_definitions())
    } else {
        Vec::new()
    };

    let tool_ctx = crate::services::ai_tools::ToolCtx::for_untrusted_model_turn(
        db,
        cache,
        quote_cache,
        latest_user_message(&params),
    );

    STOP_REQUESTED.store(false, Ordering::SeqCst);
    let client = http_client::ai_client();
    let mut last_usage: Option<ChatUsage> = None;

    // ── Agentic loop (same resilience pattern as the OpenAI path) ──────────
    const MAX_STREAM_RETRIES: usize = 2;

    for round in 0..=MAX_STREAM_RETRIES {
        if STOP_REQUESTED.load(Ordering::SeqCst) {
            break;
        }

        // Build the request body.
        let mut body = json!({
            "model": cfg.model,
            "max_tokens": 4096,
            "stream": true,
            "messages": messages,
        });
        if !system_parts.is_empty() {
            body["system"] = json!(system_parts.join("\n\n"));
        }
        if cfg.tools_enabled && !tools.is_empty() {
            body["tools"] = json!(tools);
        }

        // Send the request with Anthropic auth headers.
        let mut req = client.post(&url).json(&body);
        if !cfg.api_key.is_empty() {
            req = req.header("x-api-key", &cfg.api_key);
            req = req.header("anthropic-version", "2023-06-01");
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("请求 AI 服务失败：{e}");
                emit_error(&app, msg.clone());
                return Err(msg);
            }
        };
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            let msg = format!("AI 服务返回错误 (HTTP {status})：{body}");
            emit_error(&app, msg.clone());
            return Err(msg);
        }

        // ── Parse the SSE stream ─────────────────────────────────────────────
        // Events are `event: <name>\ndata: <json>\n\n`. We only need the
        // `data:` payloads; event names are redundant with payload `type`.
        let mut stream = resp;
        let mut buf = String::new();
        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_uses: Vec<AnthropicToolUse> = Vec::new();
        let mut current_tool_index: Option<usize> = None;
        let mut emitted_any_content = false;
        let mut stopped = false;
        let mut stream_clean = false;
        let mut round_usage: Option<ChatUsage> = None;
        let mut retryable = false;

        'stream: loop {
            if STOP_REQUESTED.load(Ordering::SeqCst) {
                stopped = true;
                break 'stream;
            }
            match stream.chunk().await {
                Ok(Some(chunk)) => {
                    buf.push_str(&String::from_utf8_lossy(&chunk));
                    while let Some(nl) = buf.find('\n') {
                        let line = buf[..nl].trim_end_matches('\r').to_string();
                        buf.drain(..=nl);
                        let Some(payload) = line.strip_prefix("data:") else {
                            continue;
                        };
                        let payload = payload.trim();
                        if payload.is_empty() {
                            continue;
                        }
                        let ev: AnthropicStreamEvent = match serde_json::from_str(payload) {
                            Ok(e) => e,
                            Err(_) => continue, // ignore non-JSON SSE lines
                        };
                        match ev.event_type.as_str() {
                            "message_start" => {
                                if let Some(u) = ev.message.and_then(|m| m.usage) {
                                    round_usage = Some(ChatUsage {
                                        prompt_tokens: u.input_tokens,
                                        completion_tokens: u.output_tokens,
                                        total_tokens: u.input_tokens + u.output_tokens,
                                        cached_tokens: 0,
                                    });
                                }
                            }
                            "content_block_start" => {
                                if let Some(block) = ev.content_block {
                                    if block.block_type == "tool_use" {
                                        tool_uses.push(AnthropicToolUse {
                                            id: block.id.clone().unwrap_or_default(),
                                            name: block.name.clone().unwrap_or_default(),
                                            arguments: String::new(),
                                        });
                                        current_tool_index = Some(tool_uses.len() - 1);
                                    }
                                }
                            }
                            "content_block_delta" => {
                                if let Some(d) = ev.delta {
                                    match d.delta_type.as_str() {
                                        "text_delta" => {
                                            if let Some(t) = d.text {
                                                text_parts.push(t.clone());
                                                emitted_any_content = true;
                                                let _ = app.emit("ai-chat-delta", t);
                                            }
                                        }
                                        "input_json_delta" => {
                                            if let Some(j) = d.partial_json {
                                                if let Some(idx) = current_tool_index {
                                                    if let Some(tu) = tool_uses.get_mut(idx) {
                                                        tu.arguments.push_str(&j);
                                                    }
                                                }
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                            "content_block_stop" => {
                                current_tool_index = None;
                            }
                            "message_delta" => {
                                if let Some(u) = ev.usage {
                                    round_usage = Some(ChatUsage {
                                        prompt_tokens: u.input_tokens,
                                        completion_tokens: u.output_tokens,
                                        total_tokens: u.input_tokens + u.output_tokens,
                                        cached_tokens: 0,
                                    });
                                }
                            }
                            "message_stop" => {
                                stream_clean = true;
                                break 'stream;
                            }
                            "error" => {
                                if let Some(e) = ev.error {
                                    let msg = e
                                        .message
                                        .unwrap_or_else(|| "未知 Anthropic 错误".to_string());
                                    emit_error(&app, msg.clone());
                                    return Err(msg);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ok(None) => break 'stream, // EOF without message_stop
                Err(e) => {
                    retryable = true;
                    warn!(target: "ai_chat", "Anthropic stream error: {e}");
                    break 'stream;
                }
            }
        }

        if let Some(u) = round_usage {
            last_usage = Some(u.clone());
            let _ = app.emit("ai-chat-usage", u);
        }

        // A clean message_stop means the model finished; stream errors mid-way
        // are retryable.
        if !stream_clean && !stopped {
            if retryable && round < MAX_STREAM_RETRIES {
                continue;
            }
            let msg = "Anthropic 流式响应中断".to_string();
            emit_error(&app, msg.clone());
            return Err(msg);
        }

        if stopped {
            break;
        }

        // ── Tool calls: execute and continue the loop ─────────────────────────
        if !tool_uses.is_empty() {
            // Append the assistant message with tool_use content blocks.
            let blocks: Vec<serde_json::Value> = tool_uses
                .iter()
                .map(|tu| {
                    json!({
                        "type": "tool_use",
                        "id": tu.id,
                        "name": tu.name,
                        "input": serde_json::from_str::<serde_json::Value>(&tu.arguments)
                            .unwrap_or_else(|_| json!({})),
                    })
                })
                .collect();
            messages.push(json!({
                "role": "assistant",
                "content": blocks,
            }));

            let tool_names: Vec<String> = tool_uses.iter().map(|t| t.name.clone()).collect();
            let _ = app.emit("ai-chat-tool", tool_names);

            // Execute each tool, append tool_result user message.
            let mut results: Vec<serde_json::Value> = Vec::new();
            for tu in &tool_uses {
                let args_for_ui = if tu.arguments.trim().is_empty() {
                    None
                } else {
                    Some(tu.arguments.clone())
                };
                let _ = app.emit(
                    "ai-chat-tool-call",
                    ToolCallEvent {
                        id: tu.id.clone(),
                        name: tu.name.clone(),
                        arguments: args_for_ui.clone(),
                        status: ToolCallStatus::Running,
                        result: None,
                        error: None,
                        duration_ms: None,
                    },
                );
                let start = std::time::Instant::now();
                let result =
                    crate::services::ai_tools::execute_tool(&tool_ctx, &tu.name, &tu.arguments)
                        .await;
                let duration = start.elapsed().as_millis() as u64;
                // ToolResult carries `content` (JSON string) and `ok`. Errors
                // are returned as error-shaped JSON so the model can recover.
                let is_err = !result.ok;
                let _ = app.emit(
                    "ai-chat-tool-call",
                    ToolCallEvent {
                        id: tu.id.clone(),
                        name: tu.name.clone(),
                        arguments: args_for_ui.clone(),
                        status: if is_err {
                            ToolCallStatus::Error
                        } else {
                            ToolCallStatus::Success
                        },
                        result: if is_err {
                            None
                        } else {
                            Some(truncate_for_display(&result.content))
                        },
                        error: if is_err {
                            Some(result.content.clone())
                        } else {
                            None
                        },
                        duration_ms: Some(duration),
                    },
                );
                results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tu.id,
                    "content": result.content,
                    "is_error": is_err,
                }));
            }
            messages.push(json!({
                "role": "user",
                "content": results,
            }));
            continue; // next round
        }

        // ── Final answer ──────────────────────────────────────────────────────
        if !emitted_any_content {
            let msg = "模型未返回任何内容，请检查模型名称或稍后重试".to_string();
            emit_error(&app, msg.clone());
            return Err(msg);
        }
        let full_text = text_parts.concat();
        let _ = app.emit(
            "ai-chat-done",
            serde_json::json!({ "content": full_text, "usage": last_usage }),
        );
        return Ok(());
    }

    // Loop exhausted without a final answer (e.g. all rounds were stopped).
    let msg = "对话未完成，请重试".to_string();
    emit_error(&app, msg.clone());
    Err(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anthropic_tool_definitions_converts_shape() {
        let openai = vec![
            json!({
                "type": "function",
                "function": {
                    "name": "get_stock_quote",
                    "description": "获取股票行情",
                    "parameters": { "type": "object", "properties": { "symbol": { "type": "string" } } },
                }
            }),
            json!({
                "type": "function",
                "function": {
                    "name": "no_params_tool",
                    "description": "",
                    "parameters": { "type": "object", "properties": {} },
                }
            }),
        ];
        let converted = anthropic_tool_definitions(&openai);
        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["name"], "get_stock_quote");
        assert_eq!(converted[0]["description"], "获取股票行情");
        assert_eq!(converted[0]["input_schema"]["type"], "object");
        assert_eq!(
            converted[0]["input_schema"]["properties"]["symbol"]["type"],
            "string"
        );
        // Anthropic tools must NOT carry the OpenAI "function" wrapper.
        assert!(converted[0].get("function").is_none());
        assert!(converted[0].get("type").is_none());
    }

    #[test]
    fn anthropic_sse_event_parses_text_and_tool_deltas() {
        // content_block_start (text)
        let start: AnthropicStreamEvent = serde_json::from_str(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        )
        .unwrap();
        assert_eq!(start.event_type, "content_block_start");
        assert_eq!(start.content_block.as_ref().unwrap().block_type, "text");

        // content_block_delta (text_delta)
        let delta: AnthropicStreamEvent = serde_json::from_str(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"你好"}}"#,
        )
        .unwrap();
        assert_eq!(delta.delta.as_ref().unwrap().text.as_deref(), Some("你好"));

        // content_block_start (tool_use) + input_json_delta
        let tool_start: AnthropicStreamEvent = serde_json::from_str(
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"get_stock_quote"}}"#,
        )
        .unwrap();
        assert_eq!(
            tool_start.content_block.as_ref().unwrap().block_type,
            "tool_use"
        );

        let json_delta: AnthropicStreamEvent = serde_json::from_str(
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"symbol\":"}}"#,
        )
        .unwrap();
        assert!(json_delta.delta.as_ref().unwrap().partial_json.is_some());

        // message_delta with usage
        let usage: AnthropicStreamEvent = serde_json::from_str(
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#,
        )
        .unwrap();
        assert_eq!(usage.usage.as_ref().unwrap().output_tokens, 42);
    }

    fn delta(
        index: u32,
        id: Option<&str>,
        name: Option<&str>,
        args: Option<&str>,
    ) -> ToolCallDelta {
        ToolCallDelta {
            index,
            id: id.map(String::from),
            function: Some(ToolCallFunctionDelta {
                name: name.map(String::from),
                arguments: args.map(String::from),
            }),
        }
    }

    #[test]
    fn merge_tool_calls_assembles_fragmented_arguments() {
        // Simulate the typical split: first chunk has id+name, then arguments
        // arrive in two fragments for the same index.
        let deltas = vec![
            delta(0, Some("call_1"), Some("get_stock_quote"), Some("{\"sym")),
            delta(0, None, None, Some("bol\":\"AAPL\"}")),
        ];
        let merged = merge_tool_calls(&deltas);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "call_1");
        assert_eq!(merged[0].function_name, "get_stock_quote");
        assert_eq!(merged[0].arguments, "{\"symbol\":\"AAPL\"}");
    }

    #[test]
    fn merge_tool_calls_preserves_index_order() {
        // Two distinct tool calls interleaved across chunks.
        let deltas = vec![
            delta(0, Some("a"), Some("get_market_overview"), None),
            delta(1, Some("b"), Some("get_stock_quote"), Some("{\"symbol\":")),
            delta(1, None, None, Some("\"TSLA\"}")),
        ];
        let merged = merge_tool_calls(&deltas);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].function_name, "get_market_overview");
        assert_eq!(merged[1].function_name, "get_stock_quote");
        assert_eq!(merged[1].arguments, "{\"symbol\":\"TSLA\"}");
    }

    #[test]
    fn merge_tool_calls_empty() {
        assert!(merge_tool_calls(&[]).is_empty());
    }

    #[test]
    fn detects_unfulfilled_tool_intent_cjk() {
        // The exact symptom from the bug report: "我来" then stop.
        assert!(looks_like_unfulfilled_tool_intent("我来"));
        assert!(looks_like_unfulfilled_tool_intent("我来查一下"));
        assert!(looks_like_unfulfilled_tool_intent("让我看看"));
        assert!(looks_like_unfulfilled_tool_intent("稍等，我查一下"));
    }

    #[test]
    fn detects_unfulfilled_tool_intent_ascii() {
        assert!(looks_like_unfulfilled_tool_intent("Let me check"));
        assert!(looks_like_unfulfilled_tool_intent("I'll look that up"));
    }

    #[test]
    fn does_not_flag_long_or_normal_replies() {
        // A genuine complete answer (longer than 30 chars) is never flagged.
        let normal = "今天大盘整体上涨，标普500收于7509点，上涨0.89%。纳斯达克表现最好。";
        assert!(!looks_like_unfulfilled_tool_intent(normal));
        // Empty is handled elsewhere (the empty-reply path), not here.
        assert!(!looks_like_unfulfilled_tool_intent(""));
    }
}
