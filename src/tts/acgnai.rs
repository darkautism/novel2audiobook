use crate::acgnai::{load_or_refresh_metadata, AcgnaiVoiceMap};
use crate::config::Config;
use crate::llm::LlmClient;
use crate::script::AudioSegment;
use crate::state::CharacterMap;
use crate::tts::{
    TtsClient, Voice, VOICE_ID_CHAPTER_MOB_FEMALE, VOICE_ID_CHAPTER_MOB_MALE, VOICE_ID_MOB_FEMALE,
    VOICE_ID_MOB_MALE, VOICE_ID_MOB_NEUTRAL,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rand::seq::IndexedRandom;
use serde_json::json;

#[derive(serde::Deserialize)]
struct AcgnaiDownloadResponse {
    msg: String,
    audio_url: String,
}

pub async fn list_voices(config: &Config, llm: Option<&Box<dyn LlmClient>>) -> Result<Vec<Voice>> {
    let metadata = load_or_refresh_metadata(config, llm).await?;
    Ok(metadata_to_voices(&metadata))
}

fn metadata_to_voices(metadata: &AcgnaiVoiceMap) -> Vec<Voice> {
    metadata
        .iter()
        .map(|(name, meta)| Voice {
            name: name.clone(),
            short_name: name.clone(),
            gender: meta.gender.clone(),
            locale: "zh".to_string(),
            friendly_name: Some(format!("{} {:?}", name, meta.tags)),
        })
        .collect()
}

pub struct AcgnaiClient {
    config: Config,
    metadata: AcgnaiVoiceMap,
}

impl AcgnaiClient {
    pub async fn new(config: &Config, llm: Option<&Box<dyn LlmClient>>) -> Result<Self> {
        let metadata = load_or_refresh_metadata(config, llm).await?;

        Ok(Self {
            config: config.clone(),
            metadata,
        })
    }

    fn pick_random_voice(
        &self,
        gender: Option<&str>,
        excluded_voices: &[String],
    ) -> Result<String> {
        let mut rng = rand::rng();
        let candidates: Vec<&String> = self
            .metadata
            .iter()
            .filter_map(|(name, meta)| {
                if excluded_voices.contains(name) {
                    return None;
                }
                if let Some(g) = gender {
                    if !meta.gender.eq_ignore_ascii_case(g) {
                        return None;
                    }
                }
                Some(name)
            })
            .collect();

        if let Some(v) = candidates.choose(&mut rng) {
            Ok(v.to_string())
        } else {
            // Fallback to any voice not excluded?
            let fallback: Vec<&String> = self
                .metadata
                .keys()
                .filter(|k| !excluded_voices.contains(k))
                .collect();
            if let Some(v) = fallback.choose(&mut rng) {
                Ok(v.to_string())
            } else {
                // Absolute fallback
                self.metadata
                    .keys()
                    .next()
                    .cloned()
                    .ok_or_else(|| anyhow!("No Acgnai voices available"))
            }
        }
    }

    async fn resolve_voice(
        &self,
        speaker: &str,
        char_map: &CharacterMap,
        excluded_voices: &[String],
    ) -> Result<String> {
        let acgnai_config = self
            .config
            .audio
            .acgnai
            .as_ref()
            .ok_or_else(|| anyhow!("Acgnai config missing"))?;

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
            Ok(meta.emotion.clone())
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

        let voice_id = if let Some(vid) = &segment.voice_id {
            vid.clone()
        } else if let Some(speaker) = &segment.speaker {
            self.resolve_voice(speaker, char_map, excluded_voices).await?
        } else {
            panic!("No speaker or voice_id specified for segment");
        };
        let acgnai_config = self
            .config
            .audio
            .acgnai
            .as_ref()
            .ok_or_else(|| anyhow!("Acgnai config missing"))?;

        let payload = json!({
          "batch_size": 10,
          "batch_threshold": 0.75,
          "emotion": segment.style.clone().unwrap_or_default(),
          "fragment_interval": 0.3,
          "if_sr": false,
          "media_type": "wav",
          "model_name": voice_id,
          "parallel_infer": true,
          "prompt_text_lang": "中文",
          "repetition_penalty": acgnai_config.repetition_penalty,
          "sample_steps": 16,
          "seed": format!("{}", rand::random::<u32>()),
          "speed_facter": acgnai_config.speed_factor,
          "split_bucket": true,
          "version": "v4",
          "text": segment.text,
          "text_lang": "中文",
          "top_k": acgnai_config.top_k,
          "top_p": acgnai_config.top_p,
          "temperature": acgnai_config.temperature,
          "text_split_method": "按标点符号切",
          //"text_split_method": "凑四句一切",
        });

        let client = reqwest::Client::new();

        let mut retry = acgnai_config.retry;
        let mut download_url = String::new();
        while retry > 0 {
            let mut req = client.post(&format!("{}infer_single", acgnai_config.base_url)).json(&payload);

            if !acgnai_config.token.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", acgnai_config.token));
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                let txt = resp.text().await?;
                return Err(anyhow!("Acgnai synthesis failed: {}", txt));
            }

            let body_text = resp.text().await?;

            // Handle cases where it might be quoted
            let response = body_text.trim().trim_matches('"').to_string();
            let download_response: AcgnaiDownloadResponse =
                serde_json::from_str(&response).unwrap();
            if download_response.msg != "合成成功" {
                if retry == 1 {
                    return Err(anyhow!(
                        "Acgnai synthesis failed: {}",
                        download_response.msg
                    ));
                } else {
                    println!(
                        "Acgnai synthesis failed: {}, retrying...\nPayload: {:?}",
                        payload,
                        download_response.msg
                    );
                    retry -= 1;
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                    continue;
                }
            }
            download_url = download_response.audio_url;
        }

        println!("Downloading from URL: {}", download_url);
        // Download WAV
        let wav_resp = client.get(&download_url).send().await?;
        let wav_bytes = wav_resp.bytes().await?;

        println!(
            "Acgnai synthesis completed: {} bytes",
            wav_bytes.len()
        );

        Ok(wav_bytes.into())
    }

    async fn get_random_voice(
        &self,
        gender: Option<&str>,
        excluded_voices: &[String],
    ) -> Result<String> {
        self.pick_random_voice(gender, excluded_voices)
    }
}
