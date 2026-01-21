use crate::config::Config;
use crate::tts::{
    TtsClient, Voice, 
    VOICE_ID_MOB_MALE, VOICE_ID_MOB_FEMALE, VOICE_ID_MOB_NEUTRAL,
    VOICE_ID_CHAPTER_MOB_MALE, VOICE_ID_CHAPTER_MOB_FEMALE
};
use crate::acgnai::{load_or_refresh_metadata, AcgnaiVoiceMap};
use crate::llm::LlmClient;
use crate::script::AudioSegment;
use crate::state::CharacterMap;
use anyhow::{anyhow, Result, Context};
use async_trait::async_trait;
use rand::seq::IndexedRandom;
use serde_json::json;
use tokio::sync::Semaphore;
use std::process::{Command, Stdio};
use std::io::Write;

pub async fn list_voices(config: &Config, llm: Option<&Box<dyn LlmClient>>) -> Result<Vec<Voice>> {
    let metadata = load_or_refresh_metadata(config, llm).await?;
    Ok(metadata_to_voices(&metadata))
}

fn metadata_to_voices(metadata: &AcgnaiVoiceMap) -> Vec<Voice> {
    metadata.iter().map(|(name, meta)| {
        Voice {
            name: name.clone(),
            short_name: name.clone(),
            gender: meta.gender.clone(),
            locale: "zh".to_string(),
            friendly_name: Some(format!("{} {:?}", name, meta.tags)),
        }
    }).collect()
}

pub struct AcgnaiClient {
    config: Config,
    metadata: AcgnaiVoiceMap,
    semaphore: Semaphore,
}

impl AcgnaiClient {
    pub async fn new(config: &Config, llm: Option<&Box<dyn LlmClient>>) -> Result<Self> {
        let metadata = load_or_refresh_metadata(config, llm).await?;
        let concurrency = config.audio.acgnai.as_ref().map(|c| c.concurrency).unwrap_or(5);
        
        Ok(Self {
            config: config.clone(),
            metadata,
            semaphore: Semaphore::new(concurrency),
        })
    }

    fn pick_random_voice(&self, gender: Option<&str>, excluded_voices: &[String]) -> Result<String> {
        let mut rng = rand::rng();
        let candidates: Vec<&String> = self.metadata.iter()
            .filter_map(|(name, meta)| {
                if excluded_voices.contains(name) { return None; }
                if let Some(g) = gender {
                    if !meta.gender.eq_ignore_ascii_case(g) { return None; }
                }
                Some(name)
            })
            .collect();
            
        if let Some(v) = candidates.choose(&mut rng) {
            Ok(v.to_string())
        } else {
            // Fallback to any voice not excluded?
            let fallback: Vec<&String> = self.metadata.keys()
                .filter(|k| !excluded_voices.contains(k))
                .collect();
             if let Some(v) = fallback.choose(&mut rng) {
                 Ok(v.to_string())
             } else {
                 // Absolute fallback
                 self.metadata.keys().next().cloned().ok_or_else(|| anyhow!("No Acgnai voices available"))
             }
        }
    }

    async fn resolve_voice(
        &self,
        speaker: &str,
        char_map: &CharacterMap,
        excluded_voices: &[String],
    ) -> Result<String> {
        let acgnai_config = self.config.audio.acgnai.as_ref().ok_or_else(|| anyhow!("Acgnai config missing"))?;

        // 1. Narrator
        if speaker == "旁白" || speaker.eq_ignore_ascii_case("Narrator") {
            if let Some(v) = &acgnai_config.narrator_voice {
                return Ok(v.clone());
            }
             // If no narrator set, use random female?
             return self.pick_random_voice(Some("Female"), excluded_voices);
        }

        // 2. Character Map
        if let Some(info) = char_map.characters.get(speaker) {
            if let Some(voice_id) = &info.voice_id {
                // Check placeholders
                match voice_id.as_str() {
                    VOICE_ID_MOB_MALE | VOICE_ID_CHAPTER_MOB_MALE => {
                        return self.pick_random_voice(Some("Male"), excluded_voices);
                    }
                    VOICE_ID_MOB_FEMALE | VOICE_ID_CHAPTER_MOB_FEMALE => {
                        return self.pick_random_voice(Some("Female"), excluded_voices);
                    }
                    VOICE_ID_MOB_NEUTRAL => {
                        return self.pick_random_voice(None, excluded_voices);
                    }
                    _ => return Ok(voice_id.clone()),
                }
            }
            
            // 3. Gender default
            match info.gender.to_lowercase().as_str() {
                "male" => {
                    if let Some(v) = &acgnai_config.default_male_voice {
                        return Ok(v.clone());
                    }
                }
                "female" => {
                    if let Some(v) = &acgnai_config.default_female_voice {
                        return Ok(v.clone());
                    }
                }
                _ => {}
            }
            
            // Random based on gender
            return self.pick_random_voice(Some(&info.gender), excluded_voices);
        }

        // 4. Fallback
        self.pick_random_voice(None, excluded_voices)
    }
}

#[async_trait]
impl TtsClient for AcgnaiClient {
    async fn list_voices(&self) -> Result<Vec<Voice>> {
        Ok(metadata_to_voices(&self.metadata))
    }

    async fn get_voice_styles(&self, voice_id: &str) -> Result<Vec<String>> {
         if let Some(meta) = self.metadata.get(voice_id) {
             let mut styles = Vec::new();
             for s_list in meta.supported_styles.values() {
                 styles.extend(s_list.clone());
             }
             styles.sort();
             styles.dedup();
             Ok(styles)
         } else {
             Ok(Vec::new())
         }
    }

    async fn synthesize(
        &self,
        segment: &AudioSegment,
        char_map: &CharacterMap,
        excluded_voices: &[String],
    ) -> Result<Vec<u8>> {
        let _permit = self.semaphore.acquire().await.context("Semaphore acquisition failed")?;
        
        let voice_id = self.resolve_voice(&segment.speaker, char_map, excluded_voices).await?;
        let acgnai_config = self.config.audio.acgnai.as_ref().ok_or_else(|| anyhow!("Acgnai config missing"))?;
        
        let payload = json!({
          "version": "v4",
          "model_name": voice_id,
          "prompt_text_lang": "",
          "emotion": segment.style.clone().unwrap_or_default(),
          "text": segment.text,
          "text_lang": "zh", 
          "top_k": acgnai_config.top_k,
          "top_p": acgnai_config.top_p,
          "temperature": acgnai_config.temperature,
          "text_split_method": "按标点符号切",
          "batch_size": 1,
          "batch_threshold": 0.75,
          "split_bucket": true,
          "speed_facter": acgnai_config.speed_factor,
          "fragment_interval": 0.3,
          "media_type": "wav",
          "parallel_infer": true,
          "repetition_penalty": acgnai_config.repetition_penalty,
          "seed": -1,
          "sample_steps": 16,
          "if_sr": false
        });

        let client = reqwest::Client::new();
        let mut req = client.post(&acgnai_config.infer_url).json(&payload);
        
        if !acgnai_config.token.is_empty() {
             req = req.header("Authorization", format!("Bearer {}", acgnai_config.token));
        }

        let resp = req.send().await?;
        if !resp.status().is_success() {
             let txt = resp.text().await?;
             return Err(anyhow!("Acgnai synthesis failed: {}", txt));
        }
        
        // Response is likely the URL string directly, or a JSON with "data" or "url".
        // User said: "成了以後會回傳一字串" (returns a string).
        let body_text = resp.text().await?;
        
        // Handle cases where it might be quoted
        let download_url = body_text.trim().trim_matches('"').to_string();
        
        println!("Acgnai Download URL: {}", download_url); 

        // Download WAV
        let wav_resp = client.get(&download_url).send().await?;
        let wav_bytes = wav_resp.bytes().await?;

        // Convert to MP3
        let mut child = Command::new("ffmpeg")
            .args(&["-f", "wav", "-i", "pipe:0", "-f", "mp3", "-b:a", "192k", "-y", "pipe:1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null()) 
            .spawn()
            .context("Failed to spawn ffmpeg")?;

        {
            let stdin = child.stdin.as_mut().context("Failed to open ffmpeg stdin")?;
            stdin.write_all(&wav_bytes).context("Failed to write to ffmpeg stdin")?;
        }

        let output = child.wait_with_output().context("Failed to wait for ffmpeg")?;
        
        if !output.status.success() {
            return Err(anyhow!("ffmpeg conversion failed"));
        }

        Ok(output.stdout)
    }

    async fn get_random_voice(
        &self,
        gender: Option<&str>,
        excluded_voices: &[String],
    ) -> Result<String> {
        self.pick_random_voice(gender, excluded_voices)
    }
}
