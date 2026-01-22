use crate::config::Config;
use crate::script::AudioSegment;
use crate::sovits::SovitsVoiceLibrary;
use crate::state::CharacterMap;
use crate::tts::{TtsClient, Voice, VOICE_ID_MOB_FEMALE, VOICE_ID_MOB_MALE, VOICE_ID_MOB_NEUTRAL};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rand::seq::IndexedRandom;

pub fn list_voices(config: &Config) -> Result<Vec<Voice>> {
    let path = config
        .audio
        .sovits
        .as_ref()
        .map(|c| c.voice_map_path.clone())
        .unwrap_or_else(|| "sovits_voices.json".to_string());

    let library = crate::sovits::load_sovits_voices(&path)?;
    Ok(library_to_voices(&library))
}

fn library_to_voices(library: &SovitsVoiceLibrary) -> Vec<Voice> {
    library
        .iter()
        .map(|(id, def)| Voice {
            name: format!("{} ({:?})", id, def.tags),
            short_name: id.clone(),
            gender: def.gender.clone(),
            locale: def.prompt_lang.clone(),
            friendly_name: Some(format!("{} - {:?}", id, def.tags)),
        })
        .collect()
}

// --- SoVITS Client ---

pub struct SovitsTtsClient {
    config: Config,
    voice_library: SovitsVoiceLibrary,
}

impl SovitsTtsClient {
    pub async fn new(config: &Config) -> Result<Self> {
        let path = config
            .audio
            .sovits
            .as_ref()
            .map(|c| c.voice_map_path.clone())
            .unwrap_or_else(|| "sovits_voices.json".to_string());

        let library = crate::sovits::load_sovits_voices(&path)?;
        Ok(Self {
            config: config.clone(),
            voice_library: library,
        })
    }

    fn pick_random_voice(
        &self,
        gender: Option<&str>,
        excluded_voices: &[String],
    ) -> Option<String> {
        let mut rng = rand::rng();
        // Sovits voices usually don't have Locale metadata in the struct provided in context
        // (impl just shows gender/prompt_lang).
        // We will assume all loaded Sovits voices are valid for the configured language or just ignore locale for Sovits
        // as Sovits is usually specific.

        let candidates: Vec<&String> = self
            .voice_library
            .iter()
            .filter_map(|(id, def)| {
                if excluded_voices.contains(id) {
                    return None;
                }
                if let Some(g) = gender {
                    if !def.gender.eq_ignore_ascii_case(g) {
                        return None;
                    }
                }
                Some(id)
            })
            .collect();

        candidates.choose(&mut rng).map(|s| s.to_string())
    }

    fn resolve_voice(
        &self,
        speaker: &str,
        char_map: &CharacterMap,
        excluded_voices: &[String],
    ) -> Option<String> {
        let sovits_config = self.config.audio.sovits.as_ref()?;

        // 1. Narrator
        if speaker == "旁白" || speaker.eq_ignore_ascii_case("Narrator") {
            return sovits_config.narrator_voice.clone();
        }

        // 2. Character Map
        if let Some(info) = char_map.characters.get(speaker) {
            if let Some(voice_id) = &info.voice_id {
                // Check for Special Mob IDs
                match voice_id.as_str() {
                    VOICE_ID_MOB_MALE => {
                        if let Some(v) = self.pick_random_voice(Some("Male"), excluded_voices) {
                            return Some(v);
                        }
                    }
                    VOICE_ID_MOB_FEMALE => {
                        if let Some(v) = self.pick_random_voice(Some("Female"), excluded_voices) {
                            return Some(v);
                        }
                    }
                    VOICE_ID_MOB_NEUTRAL => {
                        if let Some(v) = self.pick_random_voice(None, excluded_voices) {
                            return Some(v);
                        }
                    }
                    _ => return Some(voice_id.clone()),
                }
                // If random failed (no voices), fall through to default
            }

            // 3. Fallback to Gender Default
            match info.gender.to_lowercase().as_str() {
                "male" => return sovits_config.default_male_voice.clone(),
                "female" => return sovits_config.default_female_voice.clone(),
                _ => {}
            }
        }

        // 4. Default to Narrator
        sovits_config.narrator_voice.clone()
    }
}

#[async_trait]
impl TtsClient for SovitsTtsClient {
    async fn list_voices(&self) -> Result<Vec<Voice>> {
        Ok(library_to_voices(&self.voice_library))
    }

    async fn synthesize(
        &self,
        segment: &AudioSegment,
        char_map: &CharacterMap,
        excluded_voices: &[String],
    ) -> Result<Vec<u8>> {
        let voice_id = if let Some(vid) = &segment.voice_id {
            vid.clone()
        } else {
            self.resolve_voice(&segment.speaker, char_map, excluded_voices)
                .ok_or_else(|| anyhow!("No voice resolved for speaker: {}", segment.speaker))?
        };

        let voice_def = self
            .voice_library
            .get(&voice_id)
            .ok_or_else(|| anyhow!("Voice ID not found in library: {}", voice_id))?;

        let base_url = self
            .config
            .audio
            .sovits
            .as_ref()
            .map(|c| c.base_url.clone())
            .unwrap_or_else(|| "http://127.0.0.1:9880".to_string());

        let url = format!("{}/tts", base_url.trim_end_matches('/'));

        // Construct Body
        let body = serde_json::json!({
            "text": segment.text,
            "text_lang": "zh",
            "ref_audio_path": voice_def.ref_audio_path,
            "prompt_text": voice_def.prompt_text,
            "prompt_lang": voice_def.prompt_lang,
            "text_split_method": "cut5",
            "batch_size": 1,
            "media_type": "wav",
            "streaming_mode": false,
            "parallel_infer": true,
            "repetition_penalty": 1.35
        });

        let client = reqwest::Client::new();
        let resp = client.post(&url).json(&body).send().await?;

        if !resp.status().is_success() {
            let err_text = resp.text().await?;
            return Err(anyhow!("SoVITS API Error: {}", err_text));
        }

        let audio_data = resp.bytes().await?.to_vec();
        Ok(audio_data)
    }

    async fn get_random_voice(
        &self,
        gender: Option<&str>,
        excluded_voices: &[String],
    ) -> Result<String> {
        self.pick_random_voice(gender, excluded_voices)
            .ok_or_else(|| anyhow!("No random voice available"))
    }
}
