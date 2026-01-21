use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use anyhow::{Result, Context};

#[derive(Debug, Deserialize, Clone)]
pub struct SovitsVoiceDefinition {
    pub ref_audio_path: String,
    pub prompt_text: String,
    pub prompt_lang: String,
    pub gender: String,
    pub tags: Vec<String>,
}

pub type SovitsVoiceLibrary = HashMap<String, SovitsVoiceDefinition>;

pub fn load_sovits_voices(path: &str) -> Result<SovitsVoiceLibrary> {
    let path = Path::new(path);
    if !path.exists() {
        anyhow::bail!("SoVITS voice definition file not found: {:?}", path);
    }
    let content = fs::read_to_string(path).context("Failed to read SoVITS voice file")?;
    let library: SovitsVoiceLibrary = serde_json::from_str(&content).context("Failed to parse SoVITS voice JSON")?;
    Ok(library)
}
