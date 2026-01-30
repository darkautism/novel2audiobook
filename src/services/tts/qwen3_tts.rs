use crate::core::state::CharacterMap;
use crate::services::script::{AudioSegment, ScriptGenerator};
use crate::services::tts::qwen3_api::client::qwen3_tts_infer;
#[cfg(not(target_arch = "wasm32"))]
use crate::services::tts::qwen3_api::server::Qwen3Server;
use crate::services::tts::{TtsClient, Voice};
use crate::core::io::Storage;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
#[cfg(not(target_arch = "wasm32"))]
use hf_hub::api::tokio::Api;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use zhconv::{zhconv, Variant};

// --- Config ---

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Qwen3TtsConfig {
    #[serde(default)]
    pub self_host: bool,
    #[serde(default = "default_qwen3_base_url")]
    pub base_url: String,
    pub narrator_voice: Option<String>,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    pub device: Option<String>,
}

impl Default for Qwen3TtsConfig {
    fn default() -> Self {
        Self {
            self_host: false,
            base_url: default_qwen3_base_url(),
            narrator_voice: None,
            concurrency: default_concurrency(),
            device: None,
        }
    }
}

fn default_qwen3_base_url() -> String {
    "http://127.0.0.1:8000".to_string()
}

fn default_concurrency() -> usize {
    1
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
    #[cfg(not(target_arch = "wasm32"))]
    server: Option<Qwen3Server>,
    metadata: Metadata,
    voice_list: Vec<Voice>,
    storage: Arc<dyn Storage>,
}

impl Qwen3TtsClient {
    pub async fn new(config: Qwen3TtsConfig, language: String, storage: Arc<dyn Storage>) -> Result<Self> {
        info!("Initializing Qwen3 TTS Client...");
        
        // 1. Start Server if self_host (Native only)
        #[cfg(not(target_arch = "wasm32"))]
        let server = if config.self_host {
            let s = Qwen3Server::new(config.clone());
            s.start().await?;
            Some(s)
        } else {
            None
        };

        // 2. Check/Download voices
        // We use "qwen3_tts_voices" as a virtual folder in storage
        download_voices_if_needed(storage.as_ref()).await?;

        // 3. Load metadata
        let metadata_path = "qwen3_tts_voices/metadata.json";
        let metadata_bytes = storage.read(metadata_path)
            .await
            .context("Failed to read metadata.json from storage")?;
        let metadata_content = String::from_utf8(metadata_bytes).context("Invalid UTF-8 in metadata.json")?;
        let metadata: Metadata =
            serde_json::from_str(&metadata_content).context("Failed to parse metadata.json")?;

        // 4. Build Voice List
        let mut voice_list = Vec::new();
        for (lang, voices) in &metadata {
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
            #[cfg(not(target_arch = "wasm32"))]
            server,
            metadata,
            voice_list,
            storage,
        })
    }
}

async fn download_voices_if_needed(storage: &dyn Storage) -> Result<()> {
    info!("Checking voice files...");
    let target_dir = "qwen3_tts_voices";

    // Ensure "directory" exists (Storage assumes paths, but mostly for native)
    // WebStorage doesn't care.

    #[cfg(not(target_arch = "wasm32"))]
    {
        info!("Downloading voices from HuggingFace via hf-hub...");
        let api = Api::new()?;
        let repo = api.model("kautism/qwen3_tts_voices".to_string());
        let info = repo.info().await?;

        for file in info.siblings {
            let filename = file.rfilename;
            let target_path = format!("{}/{}", target_dir, filename);

            if !storage.exists(&target_path).await? {
                info!("Downloading {}...", filename);
                let path = repo.get(&filename).await?;
                let content = tokio::fs::read(path).await?;
                storage.write(&target_path, &content).await?;
            }
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        // On WASM, we only fetch metadata.json initially to save bandwidth.
        // Other files are fetched on demand in synthesize.
        let filename = "metadata.json";
        let target_path = format!("{}/{}", target_dir, filename);

        if !storage.exists(&target_path).await? {
            info!("Downloading metadata.json...");
            let url = format!("https://huggingface.co/kautism/qwen3_tts_voices/resolve/main/{}", filename);
            let resp = reqwest::get(&url).await?;
            if !resp.status().is_success() {
                return Err(anyhow!("Failed to download metadata.json: {}", resp.status()));
            }
            let data = resp.bytes().await?.to_vec();
            storage.write(&target_path, &data).await?;
        }
    }

    Ok(())
}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
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
            self.get_narrator_voice_id()
        };

        let style = segment.style.as_deref().unwrap_or("中立");
        let lang = &self.language;

        let final_style = if let Some(lang_map) = self.metadata.get(lang) {
            if let Some(v_data) = lang_map.get(&voice_id) {
                if v_data.emotion.iter().any(|e| e == style) {
                    style.to_string()
                } else {
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

        let filename = format!("{}-{}-{}.pt", lang, voice_id, final_style);
        let file_path = format!("qwen3_tts_voices/{}", filename);

        let voice_data = if self.storage.exists(&file_path).await? {
             self.storage.read(&file_path).await?
        } else {
             #[cfg(target_arch = "wasm32")]
             {
                 info!("Downloading voice file: {}...", filename);
                 let url = format!("https://huggingface.co/kautism/qwen3_tts_voices/resolve/main/{}", filename);
                 let resp = reqwest::get(&url).await?;
                 if !resp.status().is_success() {
                     return Err(anyhow!("Failed to download voice file {}: {}", filename, resp.status()));
                 }
                 let data = resp.bytes().await?.to_vec();
                 self.storage.write(&file_path, &data).await?;
                 data
             }
             #[cfg(not(target_arch = "wasm32"))]
             {
                 return Err(anyhow!("Voice file not found: {}. Please ensure all voices are downloaded.", file_path));
             }
        };

        let infer_lang = match lang.as_str() {
            "zh" => "Chinese",
            "en" => "English",
            _ => "Chinese", 
        };

        let text = if infer_lang == "Chinese" {
            &zhconv(&segment.text, Variant::ZhCN)
        } else {
            &segment.text
        };

        qwen3_tts_infer(
            base_url,
            &voice_data,
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

    async fn merge_audio_files(
        &self,
        inputs: &[String],
        output: &str,
        storage: &dyn Storage,
    ) -> Result<()> {
        let mut buffers = Vec::new();
        for input in inputs {
            let data = storage.read(input).await?;
            buffers.push(std::io::Cursor::new(data));
        }

        let mut output_buffer = Vec::new();
        {
            let mut cursor_output = std::io::Cursor::new(&mut output_buffer);
            let mut readers: Vec<&mut dyn crate::utils::audio::ReadSeek> = buffers.iter_mut()
                .map(|c| c as &mut dyn crate::utils::audio::ReadSeek)
                .collect();
            
            crate::utils::audio::merge_wav_files(&mut readers, &mut cursor_output)?;
        }
        
        storage.write(output, &output_buffer).await?;
        Ok(())
    }

    fn max_concurrency(&self) -> usize {
        self.config.concurrency
    }
}
