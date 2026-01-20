use async_trait::async_trait;
use anyhow::{Result, anyhow, Context};
use serde::{Deserialize, Serialize};
use crate::config::Config;

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn chat(&self, system: &str, user: &str) -> Result<String>;
}

pub fn create_llm(config: &Config) -> Result<Box<dyn LlmClient>> {
    match config.llm.provider.as_str() {
        "gemini" => {
            let cfg = config.llm.gemini.as_ref().context("Gemini config missing")?;
            Ok(Box::new(GeminiClient::new(&cfg.api_key, &cfg.model)))
        },
        "ollama" => {
            let cfg = config.llm.ollama.as_ref().context("Ollama config missing")?;
            Ok(Box::new(OllamaClient::new(&cfg.base_url, &cfg.model)))
        },
        _ => Err(anyhow!("Unknown LLM provider: {}", config.llm.provider))
    }
}

// --- Gemini ---

struct GeminiClient {
    api_key: String,
    model: String,
    client: reqwest::Client,
}

impl GeminiClient {
    fn new(api_key: &str, model: &str) -> Self {
        Self {
            api_key: api_key.to_string(),
            model: model.to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiSystemInstruction>,
}

#[derive(Serialize)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
struct GeminiSystemInstruction {
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
    error: Option<GeminiError>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiContentResponse,
}

#[derive(Deserialize)]
struct GeminiContentResponse {
    parts: Vec<GeminiPartResponse>,
}

#[derive(Deserialize)]
struct GeminiPartResponse {
    text: String,
}

#[derive(Deserialize, Debug)]
struct GeminiError {
    message: String,
}

#[async_trait]
impl LlmClient for GeminiClient {
    async fn chat(&self, system: &str, user: &str) -> Result<String> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let request_body = GeminiRequest {
            contents: vec![GeminiContent {
                role: "user".to_string(),
                parts: vec![GeminiPart { text: user.to_string() }],
            }],
            system_instruction: Some(GeminiSystemInstruction {
                parts: vec![GeminiPart { text: system.to_string() }],
            }),
        };

        let resp = self.client.post(&url)
            .json(&request_body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow!("Gemini API error: {}", error_text));
        }

        let result: GeminiResponse = resp.json().await?;

        if let Some(err) = result.error {
            return Err(anyhow!("Gemini API returned error: {}", err.message));
        }

        if let Some(candidates) = result.candidates {
            if let Some(first) = candidates.first() {
                if let Some(part) = first.content.parts.first() {
                    return Ok(part.text.clone());
                }
            }
        }

        Err(anyhow!("Gemini response format unexpected or empty"))
    }
}

// --- Ollama ---

struct OllamaClient {
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaClient {
    fn new(base_url: &str, model: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            model: model.to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
}

#[derive(Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OllamaResponse {
    message: OllamaMessageResponse,
}

#[derive(Deserialize)]
struct OllamaMessageResponse {
    content: String,
}

#[async_trait]
impl LlmClient for OllamaClient {
    async fn chat(&self, system: &str, user: &str) -> Result<String> {
        let url = format!("{}/api/chat", self.base_url);

        let request_body = OllamaRequest {
            model: self.model.clone(),
            messages: vec![
                OllamaMessage { role: "system".to_string(), content: system.to_string() },
                OllamaMessage { role: "user".to_string(), content: user.to_string() },
            ],
            stream: false,
        };

        let resp = self.client.post(&url)
            .json(&request_body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow!("Ollama API error: {}", error_text));
        }

        let result: OllamaResponse = resp.json().await?;
        Ok(result.message.content)
    }
}
