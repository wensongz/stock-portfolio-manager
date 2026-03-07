use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub base_url: Option<String>,
    pub system_prompt: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        AiConfig {
            provider: "openai".to_string(),
            api_key: String::new(),
            model: "gpt-4".to_string(),
            base_url: None,
            system_prompt: "你是一位专业的投资顾问，帮助用户分析股票投资组合。".to_string(),
        }
    }
}
