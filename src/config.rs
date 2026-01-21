use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use anyhow::{Context, Result};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    #[serde(default = "default_input")]
    pub input_folder: String,
    
    #[serde(default = "default_output")]
    pub output_folder: String,
    
    #[serde(default = "default_build")]
    pub build_folder: String,

    pub llm: LlmConfig,

    #[serde(default)]
    pub audio: AudioConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlmConfig {
    pub provider: String, // "gemini", "ollama" or "openai"
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
    
    #[serde(rename = "edge-tts")]
    pub edge_tts: Option<EdgeTtsConfig>,
    
    #[serde(rename = "sovits-offline")]
    pub sovits: Option<SovitsConfig>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct EdgeTtsConfig {
    pub narrator_voice: Option<String>,
    pub default_male_voice: Option<String>,
    pub default_female_voice: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SovitsConfig {
    #[serde(default = "default_sovits_base_url")]
    pub base_url: String,
    #[serde(default = "default_sovits_voice_map")]
    pub voice_map_path: String,
    
    pub narrator_voice: Option<String>,
    pub default_male_voice: Option<String>,
    pub default_female_voice: Option<String>,
}

fn default_input() -> String { "input".to_string() }
fn default_output() -> String { "output".to_string() }
fn default_build() -> String { "build".to_string() }
fn default_language() -> String { "zh".to_string() }
fn default_tts_provider() -> String { "edge-tts".to_string() }
fn default_sovits_base_url() -> String { "http://127.0.0.1:9880".to_string() }
fn default_sovits_voice_map() -> String { "sovits_voices.json".to_string() }

impl Config {
    pub fn load() -> Result<Self> {
        let path = Path::new("config.yml");
        if !path.exists() {
            anyhow::bail!("config.yml not found. Please create one.");
        }
        
        let content = fs::read_to_string(path).context("Failed to read config.yml")?;
        let config: Config = serde_norway::from_str(&content).context("Failed to parse config.yml")?;
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
