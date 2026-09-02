use super::*;

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
