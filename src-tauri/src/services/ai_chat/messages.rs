//! Preserve reasoning in the conversation; adapt only the outbound API shape.
use super::{strip_tools_from_prompt, HOST_PREFILLED_TOOL_CALL_ID};
use crate::models::ai_config::{AiConfig, ChatMessage};
use serde_json::{json, Value};

fn is_deepseek(cfg: &AiConfig) -> bool {
    cfg.provider.eq_ignore_ascii_case("deepseek")
        || (matches!(cfg.provider.as_str(), "openai" | "openrouter")
            && (cfg.model.to_ascii_lowercase().starts_with("deepseek")
                || cfg.base_url.as_deref().is_some_and(|base| {
                    url::Url::parse(base)
                        .ok()
                        .and_then(|url| url.host_str().map(str::to_string))
                        .is_some_and(|host| host == "api.deepseek.com")
                })))
}

fn accepts_reasoning_content(cfg: &AiConfig) -> bool {
    // The display history is provider-independent. Send its reasoning only
    // to interfaces that accept this extension; switching to native OpenAI,
    // Gemini, Ollama, or Anthropic must not send foreign reasoning fields.
    is_deepseek(cfg)
        || matches!(
            cfg.provider.as_str(),
            "kimi" | "glm" | "mimo" | "qwen" | "openrouter"
        )
}

pub(super) fn history_message(message: &ChatMessage, cfg: &AiConfig) -> Value {
    let mut wire = json!({ "role": message.role, "content": message.content });
    if message.role == "assistant" && accepts_reasoning_content(cfg) {
        if let Some(reasoning) = &message.reasoning_content {
            wire["reasoning_content"] = json!(reasoning);
        }
    }
    wire
}

pub(super) fn assistant_message(
    content: &str,
    reasoning: Option<&str>,
    tool_calls: Vec<Value>,
) -> Value {
    let mut message = json!({ "role": "assistant", "content": content });
    // Any compatible provider that returned this field gets its exact value
    // back within the tool loop. Keep absent and explicitly empty distinct.
    if let Some(reasoning) = reasoning {
        message["reasoning_content"] = json!(reasoning);
    }
    if !tool_calls.is_empty() {
        message["tool_calls"] = json!(tool_calls);
    }
    message
}

pub(super) fn build_request_body(
    cfg: &AiConfig,
    messages: &[Value],
    tools: &[Value],
    with_tools: bool,
) -> Value {
    let with_tools = with_tools && cfg.tools_enabled;
    let deepseek = is_deepseek(cfg);
    let accepts_reasoning = accepts_reasoning_content(cfg);
    let messages: Vec<Value> = messages
        .iter()
        .enumerate()
        .map(|(index, message)| {
            let mut message = message.clone();
            // Host-prefilled calls have no model reasoning. Legacy history
            // may also lack it. Supply an explicit empty value for DeepSeek's
            // required field, without replacing any available reasoning.
            // Kimi/MiMo and other reasoning-compatible prefill requests also
            // need an explicit value for the host-generated assistant call.
            let host_prefill = message["tool_calls"].as_array().is_some_and(|calls| {
                calls
                    .iter()
                    .any(|call| call["id"] == HOST_PREFILLED_TOOL_CALL_ID)
            });
            if (deepseek || (accepts_reasoning && host_prefill))
                && message["role"] == "assistant"
                && message.get("reasoning_content").is_none_or(Value::is_null)
            {
                message["reasoning_content"] = json!("");
            }
            if !with_tools && index == 0 && message["role"] == "system" {
                if let Some(content) = message["content"].as_str() {
                    message["content"] = json!(strip_tools_from_prompt(content));
                }
            }
            message
        })
        .collect();
    let mut body = json!({
        "model": cfg.model,
        "messages": messages,
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if with_tools {
        body["tools"] = json!(tools);
        body["tool_choice"] = json!("auto");
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(provider: &str) -> AiConfig {
        AiConfig {
            provider: provider.into(),
            model: "test-model".into(),
            ..AiConfig::default()
        }
    }

    #[test]
    fn reasoning_history_is_adapted_to_the_target_interface_without_mutation() {
        let message: ChatMessage = serde_json::from_value(json!({
            "role": "assistant", "content": "建议", "reasoning_content": "检查\n 配置。 "
        }))
        .unwrap();
        for provider in ["deepseek", "kimi", "glm", "mimo", "qwen", "openrouter"] {
            assert_eq!(
                history_message(&message, &config(provider))["reasoning_content"],
                "检查\n 配置。 "
            );
        }
        for provider in ["openai", "gemini", "grok", "ollama", "anthropic"] {
            assert!(history_message(&message, &config(provider))
                .get("reasoning_content")
                .is_none());
        }
        assert_eq!(message.reasoning_content.as_deref(), Some("检查\n 配置。 "));
        for role in ["user", "system"] {
            let message = ChatMessage {
                role: role.into(),
                ..message.clone()
            };
            assert!(history_message(&message, &config("deepseek"))
                .get("reasoning_content")
                .is_none());
        }
    }

    #[test]
    fn reasoning_prefill_and_legacy_history_have_deepseek_required_fields() {
        let messages = vec![
            json!({"role": "user", "content": "调仓建议"}),
            json!({"role": "assistant", "content": "旧回复"}),
            assistant_message(
                "",
                None,
                vec![super::super::prefilled_tool_call_message(
                    "deepseek",
                    "get_rebalance_context",
                    "{\"config_id\":\"c1\"}",
                )],
            ),
            json!({"role": "tool", "tool_call_id": super::super::HOST_PREFILLED_TOOL_CALL_ID, "content": "工具上下文"}),
        ];
        for (provider, model, base_url) in [
            ("deepseek", "deepseek-flash", None),
            ("openrouter", "deepseek/deepseek-v4-flash", None),
            ("openai", "deepseek-flash", Some("https://proxy.example/v1")),
            (
                "openai",
                "custom-alias",
                Some("https://api.deepseek.com/v1"),
            ),
        ] {
            let cfg = AiConfig {
                provider: provider.into(),
                model: model.into(),
                base_url: base_url.map(str::to_string),
                ..AiConfig::default()
            };
            let body = build_request_body(&cfg, &messages, &[], true);
            let encoded = serde_json::to_string(&body).unwrap();
            let body: Value = serde_json::from_str(&encoded).unwrap();
            assert_eq!(body["messages"][1]["reasoning_content"], "");
            assert_eq!(body["messages"][2]["reasoning_content"], "");
            assert_eq!(body["messages"][2]["content"], "");
            assert_eq!(
                body["messages"][2]["tool_calls"][0]["id"],
                body["messages"][3]["tool_call_id"]
            );
            assert!(body["messages"][0].get("reasoning_content").is_none());
            assert!(body["messages"][3].get("reasoning_content").is_none());
        }
        assert!(messages[2].get("reasoning_content").is_none());
        let other = build_request_body(&config("openai"), &messages, &[], true);
        assert!(other["messages"][2].get("reasoning_content").is_none());
        for provider in ["kimi", "glm", "mimo", "qwen", "openrouter"] {
            let body = build_request_body(&config(provider), &messages, &[], true);
            assert_eq!(
                body["messages"][2]["reasoning_content"], "",
                "{provider} prefill"
            );
        }
    }

    #[test]
    fn reasoning_survives_request_fallback_and_repeated_serialization() {
        let messages = vec![
            json!({"role": "system", "content": "助手\n\n# 你可用的工具\n工具说明\n\n# 回答原则\n中文"}),
            assistant_message(
                "正在检查",
                Some("思考\n 原样 "),
                vec![json!({"id": "call1"})],
            ),
            json!({"role": "tool", "tool_call_id": "call1", "content": "result"}),
        ];
        for provider in ["deepseek", "openai", "glm"] {
            for with_tools in [true, false] {
                let body = build_request_body(
                    &config(provider),
                    &messages,
                    &[json!({"type": "function"})],
                    with_tools,
                );
                assert_eq!(body["messages"][1]["reasoning_content"], "思考\n 原样 ");
                assert_eq!(body["messages"][1]["content"], "正在检查");
                assert_eq!(body["messages"][2], messages[2]);
                assert_eq!(body.get("tools").is_some(), with_tools);
                assert_eq!(
                    body["messages"][0]["content"]
                        .as_str()
                        .unwrap()
                        .contains("工具说明"),
                    with_tools
                );
            }
        }
    }
}
