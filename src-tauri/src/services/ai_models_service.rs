//! Fetch the list of available models from an AI provider.
//!
//! All supported providers expose an OpenAI-compatible `GET /models` endpoint,
//! so the request logic is uniform: Bearer auth (optional for local Ollama)
//! against `{base}/models`, parsed as `{ data: [{ id, owned_by }] }`.

use crate::models::ai_config::AiModelInfo;
use crate::services::http_client;
use serde::Deserialize;

/// Parameters used to look up models for a given provider.
#[derive(Debug, Clone)]
pub struct FetchModelsParams {
    pub provider: String,
    pub api_key: String,
    pub base_url: Option<String>,
}

/// OpenAI-style `/models` response.
#[derive(Debug, Deserialize)]
struct OpenAiModelsResponse {
    #[serde(default)]
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    id: String,
    #[serde(default)]
    owned_by: Option<String>,
}

/// Resolve the absolute base URL for a provider's `/models` endpoint.
///
/// `user_base` overrides everything when provided. Otherwise we fall back to
/// per-provider well-known defaults. Trailing slashes are stripped.
pub(crate) fn resolve_base_url(provider: &str, user_base: Option<&str>) -> Result<String, String> {
    if let Some(b) = user_base.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(b.trim_end_matches('/').to_string());
    }
    match provider {
        "openai" => Ok("https://api.openai.com/v1".to_string()),
        "ollama" => Ok("http://localhost:11434/v1".to_string()),
        "openrouter" => Ok("https://openrouter.ai/api/v1".to_string()),
        "kimi" | "moonshot" => Ok("https://api.moonshot.ai/v1".to_string()),
        "glm" | "zhipu" => Ok("https://open.bigmodel.cn/api/paas/v4".to_string()),
        "mimo" | "xiaomi" => Ok("https://api.xiaomimimo.com/v1".to_string()),
        "deepseek" => Ok("https://api.deepseek.com".to_string()),
        "qwen" => Ok("https://dashscope.aliyuncs.com/compatible-mode/v1".to_string()),
        "gemini" => Ok("https://generativelanguage.googleapis.com/v1beta/openai".to_string()),
        "grok" => Ok("https://api.x.ai/v1".to_string()),
        "anthropic" => Ok("https://api.anthropic.com".to_string()),
        other => Err(format!("未知的服务商：{other}，请填写 Base URL")),
    }
}

/// Fetch models from a provider via its OpenAI-compatible `/models` endpoint.
/// Anthropic uses `/v1/models` with `x-api-key` auth instead of Bearer.
pub async fn fetch_models(params: FetchModelsParams) -> Result<Vec<AiModelInfo>, String> {
    let base = resolve_base_url(&params.provider, params.base_url.as_deref())?;
    let is_anthropic = params.provider.eq_ignore_ascii_case("anthropic");
    let url = if is_anthropic {
        // Anthropic's models endpoint lives under /v1; base is api.anthropic.com
        format!("{base}/v1/models")
    } else {
        format!("{base}/models")
    };

    let client = http_client::general_client();
    let mut req = client.get(&url);
    if !params.api_key.is_empty() {
        if is_anthropic {
            req = req.header("x-api-key", &params.api_key);
            req = req.header("anthropic-version", "2023-06-01");
        } else {
            req = req.bearer_auth(&params.api_key);
        }
    }

    let resp = req.send().await.map_err(|e| format!("请求失败：{e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("获取模型失败 (HTTP {status})：{body}"));
    }

    let parsed: OpenAiModelsResponse = resp
        .json()
        .await
        .map_err(|e| format!("解析模型列表失败：{e}"))?;

    let mut models: Vec<AiModelInfo> = parsed
        .data
        .into_iter()
        .map(|m| AiModelInfo {
            id: m.id,
            name: None,
            owned_by: m.owned_by,
        })
        .collect();
    // Stable, readable ordering: alphabetical by id.
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}
