use crate::config::Config;
use crate::script::{AudioSegment, ScriptGenerator};
use crate::state::CharacterMap;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use log::info;
use serde::Deserialize;

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

#[async_trait]
pub trait TtsClient: Send + Sync {
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
        _llm: &dyn crate::llm::LlmClient,
    ) -> Result<()> {
        Ok(())
    }

    fn get_narrator_voice_id(&self) -> String;
    fn is_mob_enabled(&self) -> bool;
    fn format_voice_list_for_analysis(&self, voices: &[Voice]) -> String;
    fn get_script_generator(&self) -> Box<dyn ScriptGenerator>;
}

pub async fn fetch_voice_list(
    config: &Config,
    llm: Option<&dyn crate::llm::LlmClient>,
) -> Result<Vec<Voice>> {
    match config.audio.provider.as_str() {
        "edge-tts" => edge::list_voices().await,
        "gpt_sovits" => gpt_sovits::list_voices(config, llm).await,
        "qwen3_tts" => {
            let client = qwen3_tts::Qwen3TtsClient::new(config).await?;
            client.list_voices().await
        }
        _ => Err(anyhow::anyhow!(
            "Unknown TTS provider: {}",
            config.audio.provider
        )),
    }
}

pub async fn create_tts_client(
    config: &Config,
    llm: Option<&dyn crate::llm::LlmClient>,
) -> Result<Box<dyn TtsClient>> {
    info!("GG");
    match config.audio.provider.as_str() {
        "edge-tts" => Ok(Box::new(edge::EdgeTtsClient::new(config).await?)),
        "gpt_sovits" => Ok(Box::new(
            gpt_sovits::GptSovitsClient::new(config, llm).await?,
        )),
        "qwen3_tts" => Ok(Box::new(qwen3_tts::Qwen3TtsClient::new(config).await?)),
        _ => Err(anyhow!("Unknown TTS provider: {}", config.audio.provider)),
    }
}

pub mod edge;
pub mod gpt_sovits;
pub mod qwen3_tts;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AudioConfig;

    #[test]
    fn test_pick_random_voice() {
        // This test logic is now also covered in edge::tests, but we can keep a high level check here if needed.
        // However, we cannot access EdgeTtsClient::new_with_voices because EdgeTtsClient is now in edge module and new_with_voices might need to be pub.
        // It is `pub fn new_with_voices` in `src/tts/edge.rs`.
        // So we can access it via `edge::EdgeTtsClient`.

        let config = Config {
            input_folder: "".to_string(),
            output_folder: "".to_string(),
            build_folder: "".to_string(),
            unattended: false,
            llm: crate::config::LlmConfig {
                provider: "mock".to_string(),
                retry_count: 0,
                retry_delay_seconds: 0,
                gemini: None,
                ollama: None,
                openai: None,
            },
            audio: AudioConfig {
                provider: "edge-tts".to_string(),
                language: "zh".to_string(),
                exclude_locales: vec!["zh-HK".to_string()],
                ..Default::default()
            },
        };

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

        let client = edge::EdgeTtsClient::new_with_voices(&config, voices);

        // Test filtering
        let v = client.pick_random_voice(Some("Male"), &[]);
        assert_eq!(v, "zh-CN-Male"); // Only one zh Male
    }
}
