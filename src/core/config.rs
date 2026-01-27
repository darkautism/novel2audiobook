use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

use crate::services::llm::LlmConfig;
use crate::services::tts::edge::EdgeTtsConfig;
use crate::services::tts::gpt_sovits_config::GptSovitsConfig;
use crate::services::tts::qwen3_tts::Qwen3TtsConfig;

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
fn default_tts_provider() -> String {
    "edge-tts".to_string()
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Path::new("config.yml");
        if !path.exists() {
            anyhow::bail!("config.yml not found. Please create one.");
        }

        let content = fs::read_to_string(path).context("Failed to read config.yml")?;
        let config: Config =
            serde_yaml::from_str(&content).context("Failed to parse config.yml")?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let content = serde_yaml::to_string(self)?;
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
