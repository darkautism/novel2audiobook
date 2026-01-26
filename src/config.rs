use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_input")]
    pub input_folder: String,

    #[serde(default = "default_output")]
    pub output_folder: String,

    #[serde(default = "default_build")]
    pub build_folder: String,

    #[serde(default)]
    pub unattended: bool,

    pub llm: LlmConfig,

    #[serde(default)]
    pub audio: AudioConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlmConfig {
    pub provider: String, // "gemini", "ollama" or "openai"
    #[serde(default = "default_retry_count")]
    pub retry_count: usize,
    #[serde(default = "default_retry_delay")]
    pub retry_delay_seconds: u64,
    pub gemini: Option<GeminiConfig>,
    pub ollama: Option<OllamaConfig>,
    pub openai: Option<OpenAIConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OpenAIConfig {
    pub api_key: String,
    pub model: String,
    pub base_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiConfig {
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OllamaConfig {
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AudioConfig {
    #[serde(default = "default_tts_provider")]
    pub provider: String,
    #[serde(default = "default_language")]
    pub language: String,

    #[serde(default = "default_exclude_locales")]
    pub exclude_locales: Vec<String>,

    #[serde(rename = "edge-tts")]
    pub edge_tts: Option<EdgeTtsConfig>,
    pub gpt_sovits: Option<GptSovitsConfig>,
    pub qwen3_tts: Option<Qwen3TtsConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Qwen3TtsConfig {
    #[serde(default)]
    pub self_host: bool,
    #[serde(default = "default_qwen3_base_url")]
    pub base_url: String,
    pub narrator_voice: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct EdgeTtsConfig {
    pub narrator_voice: Option<String>,
    pub default_male_voice: Option<String>,
    pub default_female_voice: Option<String>,
    #[serde(default)]
    pub style: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GptSovitsConfig {
    pub token: String,

    #[serde(default)]
    pub retry: i32,

    #[serde(default = "default_gpt_sovits_base_url")]
    pub base_url: String,

    #[serde(default = "default_gpt_sovits_top_k")]
    pub top_k: i32,
    #[serde(default = "default_gpt_sovits_top_p")]
    pub top_p: u8,
    #[serde(default = "default_gpt_sovits_temperature")]
    pub temperature: u8,
    #[serde(default = "default_gpt_sovits_speed_factor")]
    pub speed_factor: u8,
    #[serde(default = "default_gpt_sovits_repetition_penalty")]
    pub repetition_penalty: f64,

    pub narrator_voice: Option<String>,

    #[serde(default)]
    pub autofix: bool,
}

fn default_input() -> String {
    "input".to_string()
}
fn default_output() -> String {
    "output".to_string()
}
fn default_build() -> String {
    "build".to_string()
}
fn default_language() -> String {
    "zh".to_string()
}
fn default_exclude_locales() -> Vec<String> {
    vec![]
}
fn default_retry_count() -> usize {
    3
}
fn default_retry_delay() -> u64 {
    10
}
fn default_tts_provider() -> String {
    "edge-tts".to_string()
}

fn default_gpt_sovits_base_url() -> String {
    "https://gsv2p.acgnai.top/".to_string()
}

fn default_gpt_sovits_top_k() -> i32 {
    10
}
fn default_gpt_sovits_top_p() -> u8 {
    1
}
fn default_gpt_sovits_temperature() -> u8 {
    1
}
fn default_gpt_sovits_speed_factor() -> u8 {
    1
}
fn default_gpt_sovits_repetition_penalty() -> f64 {
    1.35
}

fn default_qwen3_base_url() -> String {
    "http://127.0.0.1:8000".to_string()
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Path::new("config.yml");
        if !path.exists() {
            anyhow::bail!("config.yml not found. Please create one.");
        }

        let content = fs::read_to_string(path).context("Failed to read config.yml")?;
        let config: Config =
            serde_norway::from_str(&content).context("Failed to parse config.yml")?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let content = serde_norway::to_string(self)?;
        fs::write("config.yml", content).context("Failed to write config.yml")?;
        Ok(())
    }

    pub fn ensure_directories(&self) -> Result<()> {
        fs::create_dir_all(&self.input_folder)?;
        fs::create_dir_all(&self.output_folder)?;
        fs::create_dir_all(&self.build_folder)?;
        Ok(())
    }
}
