use crate::core::state::CharacterMap;
use crate::services::script::{AudioSegment, ScriptGenerator};
use crate::services::tts::qwen3_api::client::qwen3_tts_infer;
use crate::services::tts::qwen3_api::server::Qwen3Server;
use crate::services::tts::{TtsClient, Voice};
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use hf_hub::api::tokio::Api;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use url::Url;
use zhconv::{zhconv, Variant};

// --- Config ---

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Qwen3TtsConfig {
    #[serde(default)]
    pub self_host: bool,
    #[serde(default = "default_qwen3_base_url")]
    pub base_url: String,
    pub narrator_voice: Option<String>,
}

fn default_qwen3_base_url() -> String {
    "http://127.0.0.1:8000".to_string()
}

// --- Metadata ---

#[derive(Debug, Deserialize, Clone)]
struct VoiceMetadata {
    gender: String,
    #[serde(default)]
    tags: Vec<String>,
    emotion: Vec<String>,
}

type Metadata = HashMap<String, HashMap<String, VoiceMetadata>>;

// --- Client ---

pub struct Qwen3TtsClient {
    config: Qwen3TtsConfig,
    language: String,
    #[allow(dead_code)]
    server: Option<Qwen3Server>,
    metadata: Metadata,
    voice_list: Vec<Voice>,
}

impl Qwen3TtsClient {
    pub async fn new(config: Qwen3TtsConfig, language: String) -> Result<Self> {
        info!("Initializing Qwen3 TTS Client...");
        
        // 1. Start Server if self_host
        let server = if config.self_host {
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
            if lang == &language {
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
            config,
            language,
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

            // Create parent directories if needed
            if let Some(parent) = target_path.parent() {
                if !parent.exists() {
                    fs::create_dir_all(parent).await?;
                }
            }

            // Manual download to avoid hf-hub panic with CJK filenames
            let mut url = Url::parse("https://huggingface.co/kautism/qwen3_tts_voices/resolve/main/")?;
            url.path_segments_mut()
                .map_err(|_| anyhow!("Invalid URL"))?
                .push(&filename);

            let response = reqwest::get(url).await?;
            if !response.status().is_success() {
                return Err(anyhow!("Failed to download {}: {}", filename, response.status()));
            }

            let content = response.bytes().await?;
            let mut file = fs::File::create(&target_path).await?;
            file.write_all(&content).await?;
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
        let base_url = &self.config.base_url;

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
        let lang = &self.language;

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

        let text = if infer_lang == "Chinese" {
            &zhconv(&segment.text, Variant::ZhCN)
        } else {
            &segment.text
        };

        qwen3_tts_infer(
            base_url,
            file_path.to_str().unwrap(),
            text,
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
        let lang = &self.language;
        if let Some(lang_map) = self.metadata.get(lang) {
            if let Some(v_data) = lang_map.get(voice_id) {
                return Ok(v_data.emotion.clone());
            }
        }
        Ok(vec![])
    }

    fn get_narrator_voice_id(&self) -> String {
        self.config
            .narrator_voice
            .clone()
            .unwrap_or_else(|| "default".to_string())
    }

    fn is_mob_enabled(&self) -> bool {
        false
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
        Box::new(crate::services::script::Qwen3ScriptGenerator::new(
            self.get_narrator_voice_id(),
        ))
    }
}
