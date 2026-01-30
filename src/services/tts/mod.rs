use crate::core::config::Config;
use crate::services::script::{AudioSegment, ScriptGenerator};
use crate::core::state::CharacterMap;
use crate::services::llm::LlmClient;
use crate::core::io::Storage;
use crate::utils::audio::ReadSeek;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::info;
use serde::Deserialize;
use std::io::Cursor;
use std::sync::Arc;

// --- Constants ---

pub const VOICE_ID_MOB_MALE: &str = "placeholder_mob_male";
pub const VOICE_ID_MOB_FEMALE: &str = "placeholder_mob_female";
pub const VOICE_ID_MOB_NEUTRAL: &str = "placeholder_mob_neutral";
pub const VOICE_ID_CHAPTER_MOB_MALE: &str = "placeholder_chapter_mob_male";
pub const VOICE_ID_CHAPTER_MOB_FEMALE: &str = "placeholder_chapter_mob_female";

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Voice {
    pub name: String,
    pub short_name: String,
    pub gender: String,
    pub locale: String,
    pub friendly_name: Option<String>,
}

#[cfg(target_arch = "wasm32")]
pub trait TtsBounds {}
#[cfg(target_arch = "wasm32")]
impl<T> TtsBounds for T {}

#[cfg(not(target_arch = "wasm32"))]
pub trait TtsBounds: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync> TtsBounds for T {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait TtsClient: TtsBounds {
    async fn list_voices(&self) -> Result<Vec<Voice>>;
    async fn synthesize(
        &self,
        segment: &AudioSegment,
        char_map: &CharacterMap,
        excluded_voices: &[String],
    ) -> Result<Vec<u8>>;
    async fn get_random_voice(
        &self,
        gender: Option<&str>,
        excluded_voices: &[String],
    ) -> Result<String>;

    async fn get_voice_styles(&self, _voice_id: &str) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    async fn check_and_fix_segments(
        &self,
        _segments: &mut Vec<AudioSegment>,
        _char_map: &CharacterMap,
        _excluded_voices: &[String],
        _llm: &dyn LlmClient,
    ) -> Result<()> {
        Ok(())
    }

    fn get_narrator_voice_id(&self) -> String;
    fn is_mob_enabled(&self) -> bool;
    fn format_voice_list_for_analysis(&self, voices: &[Voice]) -> String;
    fn get_script_generator(&self) -> Box<dyn ScriptGenerator>;

    async fn merge_audio_files(
        &self,
        inputs: &[String],
        output: &str,
        storage: &dyn Storage,
    ) -> Result<()> {
        // Default implementation using Storage (Binary merge for MP3)
        let mut buffers = Vec::new();
        for input in inputs {
            let data = storage.read(input).await?;
            buffers.push(Cursor::new(data));
        }

        let mut output_buffer = Vec::new();
        // Scope for mutable borrow of output_buffer
        {
            let mut cursor_output = Cursor::new(&mut output_buffer);
            let mut readers: Vec<&mut dyn ReadSeek> = buffers.iter_mut()
                .map(|c| c as &mut dyn ReadSeek)
                .collect();
            
            crate::utils::audio::merge_binary_files(&mut readers, &mut cursor_output)?;
        }
        
        storage.write(output, &output_buffer).await?;
        Ok(())
    }

    fn max_concurrency(&self) -> usize {
        5
    }
}

pub async fn fetch_voice_list(
    config: &Config,
    llm: Option<&dyn LlmClient>,
    storage: Arc<dyn Storage>,
) -> Result<Vec<Voice>> {
    match config.audio.provider.as_str() {
        #[cfg(not(target_arch = "wasm32"))]
        "edge-tts" => edge::list_voices().await,
        #[cfg(not(target_arch = "wasm32"))]
        "gpt_sovits" => {
            let gpt_config = config
                .audio
                .gpt_sovits
                .as_ref()
                .ok_or_else(|| anyhow!("GPT-Sovits config missing"))?;
            let language = &config.audio.language;
            gpt_sovits::list_voices(gpt_config, language, llm).await
        }
        "qwen3_tts" => {
            let qwen_config = config
                .audio
                .qwen3_tts
                .clone()
                .ok_or_else(|| anyhow!("Qwen3 TTS config missing"))?;
            let language = config.audio.language.clone();
            let client = qwen3_tts::Qwen3TtsClient::new(qwen_config, language, storage).await?;
            client.list_voices().await
        }
        _ => Err(anyhow::anyhow!(
            "Unknown TTS provider (or not supported on this platform): {}",
            config.audio.provider
        )),
    }
}

pub async fn create_tts_client(
    config: &Config,
    llm: Option<&dyn LlmClient>,
    storage: Arc<dyn Storage>,
) -> Result<Box<dyn TtsClient>> {
    info!("Initializing TTS Client for provider: {}", config.audio.provider);
    match config.audio.provider.as_str() {
        #[cfg(not(target_arch = "wasm32"))]
        "edge-tts" => {
            let edge_config = config.audio.edge_tts.clone().unwrap_or_default();
            let exclude_locales = config.audio.exclude_locales.clone();
            let language = config.audio.language.clone();
            Ok(Box::new(
                edge::EdgeTtsClient::new(edge_config, exclude_locales, language).await?,
            ))
        }
        #[cfg(not(target_arch = "wasm32"))]
        "gpt_sovits" => {
            let gpt_config = config
                .audio
                .gpt_sovits
                .clone()
                .ok_or_else(|| anyhow!("GPT-Sovits config missing"))?;
            let language = config.audio.language.clone();
            Ok(Box::new(
                gpt_sovits::GptSovitsClient::new(gpt_config, &language, llm).await?,
            ))
        }
        "qwen3_tts" => {
            let qwen_config = config
                .audio
                .qwen3_tts
                .clone()
                .ok_or_else(|| anyhow!("Qwen3 TTS config missing"))?;
            let language = config.audio.language.clone();
            Ok(Box::new(
                qwen3_tts::Qwen3TtsClient::new(qwen_config, language, storage).await?,
            ))
        }
        _ => Err(anyhow!("Unknown TTS provider: {}", config.audio.provider)),
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod edge;
#[cfg(not(target_arch = "wasm32"))]
pub mod gpt_sovits;
pub mod qwen3_tts;
#[cfg(not(target_arch = "wasm32"))]
pub mod gpt_sovits_config;
pub mod qwen3_api;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::tts::edge::EdgeTtsConfig;

    #[test]
    fn test_pick_random_voice() {
        let edge_config = EdgeTtsConfig {
            narrator_voice: Some("Narrator".to_string()),
            ..Default::default()
        };
        let exclude_locales = vec!["zh-HK".to_string()];
        let language = "zh".to_string();

        let voices = vec![
            Voice {
                short_name: "zh-CN-Male".to_string(),
                gender: "Male".to_string(),
                locale: "zh-CN".to_string(),
                name: "".to_string(),
                friendly_name: None,
            },
            Voice {
                short_name: "zh-TW-Female".to_string(),
                gender: "Female".to_string(),
                locale: "zh-TW".to_string(),
                name: "".to_string(),
                friendly_name: None,
            },
            Voice {
                short_name: "en-US-Male".to_string(),
                gender: "Male".to_string(),
                locale: "en-US".to_string(),
                name: "".to_string(),
                friendly_name: None,
            },
        ];

        let client = edge::EdgeTtsClient::new_with_voices(
            edge_config,
            exclude_locales,
            language,
            voices,
        );

        // Test filtering
        let v = client.pick_random_voice(Some("Male"), &[]);
        assert_eq!(v, "zh-CN-Male"); // Only one zh Male
    }
}
