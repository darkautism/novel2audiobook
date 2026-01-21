use async_trait::async_trait;
use anyhow::{Result, anyhow, Context};
use serde::{Deserialize, Serialize};
use crate::config::Config;
use std::fmt::Debug;


#[async_trait]
pub trait LlmClient: Send + Sync + Debug {
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
        "openai" => {
            let cfg = config.llm.openai.as_ref().context("OpenAI config missing")?;
            Ok(Box::new(OpenAIClient::new(&cfg.api_key, &cfg.model, cfg.base_url.as_deref())))
        },
        _ => Err(anyhow!("Unknown LLM provider: {}", config.llm.provider))
    }
}

// --- Gemini ---
#[derive(Debug)]
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
    content: Option<GeminiContentResponse>,
    #[serde(rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct GeminiContentResponse {
    #[serde(default)]
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

        // Get text to debug JSON issues if needed
        let response_text = resp.text().await?;
        let result: GeminiResponse = match serde_json::from_str(&response_text) {
            Ok(r) => r,
            Err(e) => return Err(anyhow!("Failed to parse Gemini response: {}. Body: {}", e, response_text)),
        };

        if let Some(err) = result.error {
            return Err(anyhow!("Gemini API returned error: {}", err.message));
        }

        if let Some(candidates) = result.candidates {
            if let Some(first) = candidates.first() {
                if let Some(content) = &first.content {
                    if let Some(part) = content.parts.first() {
                        return Ok(part.text.clone());
                    }
                }
                
                // If we get here, content or parts are missing
                let reason = first.finish_reason.as_deref().unwrap_or("UNKNOWN");
                return Err(anyhow!("Gemini response empty. Finish reason: {}", reason));
            }
        }

        Err(anyhow!("Gemini response format unexpected or empty. Body: {}", response_text))
    }
}

// --- Ollama ---
#[derive(Debug)]
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

// --- OpenAI ---

#[derive(Debug)]
struct OpenAIClient {
    api_key: String,
    model: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAIClient {
    fn new(api_key: &str, model: &str, base_url: Option<&str>) -> Self {
        Self {
            api_key: api_key.to_string(),
            model: model.to_string(),
            base_url: base_url.unwrap_or("https://api.openai.com/v1").trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
}

#[derive(Serialize)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessageResponse,
}

#[derive(Deserialize)]
struct OpenAIMessageResponse {
    content: Option<String>,
}

#[async_trait]
impl LlmClient for OpenAIClient {
    async fn chat(&self, system: &str, user: &str) -> Result<String> {
        let url = format!("{}/chat/completions", self.base_url);

        let request_body = OpenAIRequest {
            model: self.model.clone(),
            messages: vec![
                OpenAIMessage { role: "system".to_string(), content: system.to_string() },
                OpenAIMessage { role: "user".to_string(), content: user.to_string() },
            ],
        };

        let resp = self.client.post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&request_body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let error_text = resp.text().await?;
            return Err(anyhow!("OpenAI API error: {}", error_text));
        }

        let result: OpenAIResponse = resp.json().await?;
        if let Some(choice) = result.choices.first() {
             if let Some(content) = &choice.message.content {
                 return Ok(content.clone());
             }
        }
        
        Err(anyhow!("OpenAI response empty or missing content"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gemini_response_parsing_safety_block() {
        // Simulating a response where content is blocked (safety)
        // Usually content is missing or parts missing.
        let json = r#"{
            "candidates": [
                {
                    "finishReason": "SAFETY",
                    "index": 0
                }
            ]
        }"#;

        let result: GeminiResponse = serde_json::from_str(json).unwrap();
        let candidate = &result.candidates.as_ref().unwrap()[0];
        
        assert!(candidate.content.is_none());
        assert_eq!(candidate.finish_reason.as_deref(), Some("SAFETY"));
    }

    #[test]
    fn test_gemini_response_parsing_empty_content() {
        // Simulating a response where parts might be missing
        let json = r#"{
            "candidates": [
                {
                    "content": { "role": "model" },
                    "finishReason": "STOP",
                    "index": 0
                }
            ]
        }"#;

        let result: GeminiResponse = serde_json::from_str(json).unwrap();
        let candidate = &result.candidates.as_ref().unwrap()[0];
        
        // Content exists but parts are empty (default)
        assert!(candidate.content.is_some());
        assert!(candidate.content.as_ref().unwrap().parts.is_empty());
    }
    
    #[test]
    fn test_gemini_response_parsing_success() {
        let json = r#"{
            "candidates": [
                {
                    "content": {
                        "parts": [
                            { "text": "Hello world" }
                        ],
                        "role": "model"
                    },
                    "finishReason": "STOP",
                    "index": 0
                }
            ]
        }"#;
        
        let result: GeminiResponse = serde_json::from_str(json).unwrap();
        let candidate = &result.candidates.as_ref().unwrap()[0];
        
        assert_eq!(candidate.content.as_ref().unwrap().parts[0].text, "Hello world");
    }

    #[test]
    fn test_openai_response_parsing_success() {
        let json = r#"{
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1677652288,
            "model": "gpt-3.5-turbo-0613",
            "system_fingerprint": "fp_44709d6fcb",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello there, how may I assist you today?"
                },
                "logprobs": null,
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 9,
                "completion_tokens": 12,
                "total_tokens": 21
            }
        }"#;

        let result: OpenAIResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            result.choices[0].message.content.as_deref(),
            Some("Hello there, how may I assist you today?")
        );
    }
}
