use crate::config::Config;
use crate::script::{AudioSegment, ScriptGenerator};
use crate::state::CharacterMap;
use crate::tts::{TtsClient, Voice};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use hf_hub::api::tokio::Api;
use log::{info, warn};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

use crate::qwen3_tts::client::qwen3_tts_infer;
use crate::qwen3_tts::server::Qwen3Server;

#[derive(Debug, Deserialize, Clone)]
struct VoiceMetadata {
    gender: String,
    #[serde(default)]
    tags: Vec<String>,
    emotion: Vec<String>,
}

type Metadata = HashMap<String, HashMap<String, VoiceMetadata>>;

pub struct Qwen3TtsClient {
    config: Config,
    #[allow(dead_code)]
    server: Option<Qwen3Server>,
    metadata: Metadata,
    voice_list: Vec<Voice>,
}

impl Qwen3TtsClient {
    pub async fn new(config: &Config) -> Result<Self> {
        info!("Initializing Qwen3 TTS Client...");
        let qwen_config = config
            .audio
            .qwen3_tts
            .as_ref()
            .context("Qwen3 TTS config missing")?;

        // 1. Start Server if self_host
        let server = if qwen_config.self_host {
            let s = Qwen3Server::new();
            s.start().await?;
            Some(s)
        } else {
            None
        };

        // 2. Prepare voices directory
        let voices_dir = Path::new("qwen3_tts_voices");
        if !voices_dir.exists() {
            fs::create_dir_all(voices_dir).await?;
        }

        // 3. Check/Download voices
        download_voices_if_needed(voices_dir).await?;

        // 4. Load metadata
        let metadata_path = voices_dir.join("metadata.json");
        let metadata_content = fs::read_to_string(&metadata_path)
            .await
            .context("Failed to read metadata.json")?;
        let metadata: Metadata =
            serde_json::from_str(&metadata_content).context("Failed to parse metadata.json")?;

        // 5. Build Voice List
        let mut voice_list = Vec::new();
        // Assume structure {"zh": {"Name": ...}}
        // We flatten this to Voice structs
        for (lang, voices) in &metadata {
            // Only include voices for configured language?
            // config.audio.language is "zh", "en", etc.
            // If they match, great.
            if lang == &config.audio.language {
                for (name, data) in voices {
                    voice_list.push(Voice {
                        name: name.clone(),
                        short_name: name.clone(), // Use name as ID
                        gender: data.gender.clone(),
                        locale: lang.clone(),
                        friendly_name: Some(format!("{} - {}", name, data.tags.join(","))),
                    });
                }
            }
        }

        Ok(Self {
            config: config.clone(),
            server,
            metadata,
            voice_list,
        })
    }
}

async fn download_voices_if_needed(target_dir: &Path) -> Result<()> {
    let api = Api::new()?;
    let repo = api.model("kautism/qwen3_tts_voices".to_string());

    // Get list of files
    // Note: info() retrieves repo info including siblings
    let info = repo.info().await?;

    for file in info.siblings {
        let filename = file.rfilename;
        let target_path = target_dir.join(&filename);

        if !target_path.exists() {
            info!("Downloading {}...", filename);
            let path = repo.get(&filename).await?;
            // hf-hub stores in cache. We need to copy it to target_dir.
            fs::copy(path, target_path).await?;
        }
    }

    Ok(())
}

#[async_trait]
impl TtsClient for Qwen3TtsClient {
    async fn list_voices(&self) -> Result<Vec<Voice>> {
        Ok(self.voice_list.clone())
    }

    async fn synthesize(
        &self,
        segment: &AudioSegment,
        char_map: &CharacterMap,
        _excluded_voices: &[String],
    ) -> Result<Vec<u8>> {
        let qwen_config = self.config.audio.qwen3_tts.as_ref().unwrap();
        let base_url = &qwen_config.base_url;

        // Determine Voice ID
        let voice_id = if let Some(vid) = &segment.voice_id {
            vid.clone()
        } else if let Some(speaker) = &segment.speaker {
            if let Some(char_info) = char_map.characters.get(speaker) {
                char_info
                    .voice_id
                    .clone()
                    .unwrap_or_else(|| self.get_narrator_voice_id())
            } else {
                warn!("Speaker {} not found in map", speaker);
                self.get_narrator_voice_id()
            }
        } else {
            // Narrator
            self.get_narrator_voice_id()
        };

        // Determine Style
        let style = segment.style.as_deref().unwrap_or("中立");

        // Check if style exists for this voice
        let lang = &self.config.audio.language; // "zh"

        // Validate style in metadata
        let final_style = if let Some(lang_map) = self.metadata.get(lang) {
            if let Some(v_data) = lang_map.get(&voice_id) {
                if v_data.emotion.iter().any(|e| e == style) {
                    style.to_string()
                } else {
                    // Fallback to first emotion or "中立"
                    v_data
                        .emotion
                        .first()
                        .cloned()
                        .unwrap_or("中立".to_string())
                }
            } else {
                style.to_string()
            }
        } else {
            style.to_string()
        };

        // Construct .pt filename
        // Format: {lang}-{voice_id}-{style}.pt
        let filename = format!("{}-{}-{}.pt", lang, voice_id, final_style);
        let file_path = Path::new("qwen3_tts_voices").join(&filename);

        if !file_path.exists() {
            return Err(anyhow!("Voice file not found: {:?}", file_path));
        }

        // Call infer
        let infer_lang = match lang.as_str() {
            "zh" => "Chinese",
            "en" => "English",
            _ => "Chinese", // Default
        };

        qwen3_tts_infer(
            base_url,
            file_path.to_str().unwrap(),
            &segment.text,
            infer_lang,
        )
        .await
    }

    async fn get_random_voice(
        &self,
        gender: Option<&str>,
        excluded_voices: &[String],
    ) -> Result<String> {
        use rand::prelude::IndexedRandom;
        let candidates: Vec<&Voice> = self
            .voice_list
            .iter()
            .filter(|v| {
                if let Some(g) = gender {
                    if v.gender != g {
                        return false;
                    }
                }
                if excluded_voices.contains(&v.short_name) {
                    return false;
                }
                true
            })
            .collect();

        if let Some(v) = candidates.choose(&mut rand::rng()) {
            Ok(v.short_name.clone())
        } else {
            Err(anyhow!("No available voices for random selection"))
        }
    }

    async fn get_voice_styles(&self, voice_id: &str) -> Result<Vec<String>> {
        let lang = &self.config.audio.language;
        if let Some(lang_map) = self.metadata.get(lang) {
            if let Some(v_data) = lang_map.get(voice_id) {
                return Ok(v_data.emotion.clone());
            }
        }
        Ok(vec![])
    }

    fn get_narrator_voice_id(&self) -> String {
        self.config
            .audio
            .qwen3_tts
            .as_ref()
            .and_then(|c| c.narrator_voice.clone())
            .unwrap_or_else(|| "default".to_string())
    }

    fn is_mob_enabled(&self) -> bool {
        true
    }

    fn format_voice_list_for_analysis(&self, voices: &[Voice]) -> String {
        let mut s = String::new();
        for v in voices {
            s.push_str(&format!(
                "- ID: {}, Gender: {}, Info: {}\n",
                v.short_name,
                v.gender,
                v.friendly_name.as_deref().unwrap_or("")
            ));
        }
        s
    }

    fn get_script_generator(&self) -> Box<dyn ScriptGenerator> {
        Box::new(crate::script::Qwen3ScriptGenerator::new(
            self.get_narrator_voice_id(),
        ))
    }
}
