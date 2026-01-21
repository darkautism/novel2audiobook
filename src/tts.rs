use crate::config::Config;
use crate::script::AudioSegment;
use crate::state::CharacterMap;
use crate::sovits::SovitsVoiceLibrary;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use rand::seq::IndexedRandom; // For random selection

// --- Constants ---

pub const VOICE_ID_MOB_MALE: &str = "placeholder_mob_male";
pub const VOICE_ID_MOB_FEMALE: &str = "placeholder_mob_female";
pub const VOICE_ID_MOB_NEUTRAL: &str = "placeholder_mob_neutral";

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const CHROMIUM_MAJOR_VERSION: &str = "143";
const LIST_VOICES_URL: &str = "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list";

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Voice {
    pub name: String,
    pub short_name: String,
    pub gender: String,
    pub locale: String,
    pub friendly_name: Option<String>,
}

fn get_user_agent() -> String {
    format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.0.0 Safari/537.36 Edg/{}.0.0.0",
        CHROMIUM_MAJOR_VERSION, CHROMIUM_MAJOR_VERSION
    )
}

fn get_sec_ch_ua() -> String {
    format!(
        "\" Not;A Brand\";v=\"99\", \"Microsoft Edge\";v=\"{}\", \"Chromium\";v=\"{}\"",
        CHROMIUM_MAJOR_VERSION, CHROMIUM_MAJOR_VERSION
    )
}

#[async_trait]
pub trait TtsClient: Send + Sync {
    async fn list_voices(&self) -> Result<Vec<Voice>>;
    async fn synthesize(&self, segment: &AudioSegment, char_map: &CharacterMap) -> Result<Vec<u8>>;
}

pub async fn create_tts_client(config: &Config) -> Result<Box<dyn TtsClient>> {
    match config.audio.provider.as_str() {
        "edge-tts" => Ok(Box::new(EdgeTtsClient::new(config).await?)),
        "sovits-offline" => Ok(Box::new(SovitsTtsClient::new(config).await?)),
        _ => Err(anyhow!("Unknown TTS provider: {}", config.audio.provider)),
    }
}

// --- Shared Helper for EdgeTTS ---

async fn list_voices_http() -> Result<Vec<Voice>> {
    let url = format!(
        "{}?trustedclienttoken={}",
        LIST_VOICES_URL, TRUSTED_CLIENT_TOKEN
    );
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    
    headers.insert("Authority", HeaderValue::from_static("speech.platform.bing.com"));
    headers.insert("Sec-CH-UA", HeaderValue::from_str(&get_sec_ch_ua())?);
    headers.insert("Sec-CH-UA-Mobile", HeaderValue::from_static("?0"));
    headers.insert("User-Agent", HeaderValue::from_str(&get_user_agent())?);
    headers.insert("Sec-CH-UA-Platform", HeaderValue::from_static("\"Windows\""));
    headers.insert("Accept", HeaderValue::from_static("*/*"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("cors"));
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("empty"));
    headers.insert("Accept-Encoding", HeaderValue::from_static("gzip, deflate, br, zstd"));
    headers.insert("Accept-Language", HeaderValue::from_static("en-US,en;q=0.9"));

    let resp = client.get(&url).headers(headers).send().await?;
    if !resp.status().is_success() {
        return Err(anyhow!("Failed to list voices: {}", resp.status()));
    }
    let voices: Vec<Voice> = resp.json().await?;
    Ok(voices)
}

// --- Edge TTS Client ---

pub struct EdgeTtsClient {
    config: Config,
    voices_cache: Vec<Voice>,
}

impl EdgeTtsClient {
    pub async fn new(config: &Config) -> Result<Self> {
        // Pre-fetch voices for caching
        let voices_cache = list_voices_http().await.unwrap_or_else(|e| {
            eprintln!("Warning: Failed to fetch EdgeTTS voices for random selection: {}", e);
            Vec::new()
        });
        Ok(Self { config: config.clone(), voices_cache })
    }

    #[cfg(test)]
    pub fn new_with_voices(config: &Config, voices: Vec<Voice>) -> Self {
        Self { config: config.clone(), voices_cache: voices }
    }

    fn pick_random_voice(&self, gender: Option<&str>) -> String {
        let lang_prefix = &self.config.audio.language;
        let mut rng = rand::rng(); 

        let candidates: Vec<&Voice> = self.voices_cache.iter().filter(|v| {
            if !v.locale.starts_with(lang_prefix) {
                return false;
            }
            if let Some(g) = gender {
                if !v.gender.eq_ignore_ascii_case(g) {
                    return false;
                }
            }
            true
        }).collect();

        if let Some(v) = candidates.choose(&mut rng) {
            v.short_name.clone()
        } else {
             // Fallback
             self.config.audio.edge_tts.as_ref()
                .and_then(|c| c.narrator_voice.clone())
                .unwrap_or_else(|| "zh-TW-HsiaoChenNeural".to_string())
        }
    }

    fn resolve_voice(&self, speaker: &str, char_map: &CharacterMap) -> String {
        let edge_config = self.config.audio.edge_tts.as_ref();
        
        // 1. Check if Narrator
        if speaker == "旁白" || speaker.eq_ignore_ascii_case("Narrator") {
            if let Some(cfg) = edge_config {
                if let Some(v) = &cfg.narrator_voice {
                    return v.clone();
                }
            }
        }

        // 2. Check Character Map
        if let Some(info) = char_map.characters.get(speaker) {
            if let Some(voice_id) = &info.voice_id {
                // Check for Special Mob IDs
                match voice_id.as_str() {
                    VOICE_ID_MOB_MALE => return self.pick_random_voice(Some("Male")),
                    VOICE_ID_MOB_FEMALE => return self.pick_random_voice(Some("Female")),
                    VOICE_ID_MOB_NEUTRAL => return self.pick_random_voice(None),
                    _ => return voice_id.clone(),
                }
            }
            
            // 3. Fallback to Gender Default
            if let Some(cfg) = edge_config {
                match info.gender.to_lowercase().as_str() {
                    "male" => {
                        if let Some(v) = &cfg.default_male_voice { return v.clone(); }
                    },
                    "female" => {
                        if let Some(v) = &cfg.default_female_voice { return v.clone(); }
                    },
                    _ => {}
                }
            }
        }

        // 4. Ultimate Fallback (Narrator or first available)
        if let Some(cfg) = edge_config {
            if let Some(v) = &cfg.narrator_voice { return v.clone(); }
        }
        
        "zh-TW-HsiaoChenNeural".to_string() // Hard fallback
    }
}

#[async_trait]
impl TtsClient for EdgeTtsClient {
    async fn list_voices(&self) -> Result<Vec<Voice>> {
        // Return cached voices if available, or fetch
        if !self.voices_cache.is_empty() {
            Ok(self.voices_cache.clone())
        } else {
            list_voices_http().await
        }
    }

    async fn synthesize(&self, segment: &AudioSegment, char_map: &CharacterMap) -> Result<Vec<u8>> {
        let voice = self.resolve_voice(&segment.speaker, char_map);
        let ssml = format!(
            "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'><voice name='{}'>{}</voice></speak>",
            voice, segment.text
        );

        tokio::task::spawn_blocking(move || {
            edge_tts::request_audio(&ssml, "audio-24khz-48kbitrate-mono-mp3")
                .map_err(|e| anyhow!("Edge TTS crate error: {:?}", e))
        })
        .await?
    }
}

// --- SoVITS Client ---

pub struct SovitsTtsClient {
    config: Config,
    voice_library: SovitsVoiceLibrary,
}

impl SovitsTtsClient {
    pub async fn new(config: &Config) -> Result<Self> {
        let path = config.audio.sovits.as_ref()
            .map(|c| c.voice_map_path.clone())
            .unwrap_or_else(|| "sovits_voices.json".to_string());
        
        let library = crate::sovits::load_sovits_voices(&path)?;
        Ok(Self { config: config.clone(), voice_library: library })
    }

    fn pick_random_voice(&self, gender: Option<&str>) -> Option<String> {
        let mut rng = rand::rng();
        // Sovits voices usually don't have Locale metadata in the struct provided in context 
        // (impl just shows gender/prompt_lang). 
        // We will assume all loaded Sovits voices are valid for the configured language or just ignore locale for Sovits 
        // as Sovits is usually specific.
        
        let candidates: Vec<&String> = self.voice_library.iter().filter_map(|(id, def)| {
             if let Some(g) = gender {
                 if !def.gender.eq_ignore_ascii_case(g) {
                     return None;
                 }
             }
             Some(id)
        }).collect();

        candidates.choose(&mut rng).map(|s| s.to_string())
    }

    fn resolve_voice(&self, speaker: &str, char_map: &CharacterMap) -> Option<String> {
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
                         if let Some(v) = self.pick_random_voice(Some("Male")) { return Some(v); }
                     },
                     VOICE_ID_MOB_FEMALE => {
                         if let Some(v) = self.pick_random_voice(Some("Female")) { return Some(v); }
                     },
                     VOICE_ID_MOB_NEUTRAL => {
                         if let Some(v) = self.pick_random_voice(None) { return Some(v); }
                     },
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
        // Return voices from the library converted to Voice struct
        let voices = self.voice_library.iter().map(|(id, def)| {
            Voice {
                name: format!("{} ({:?})", id, def.tags),
                short_name: id.clone(),
                gender: def.gender.clone(),
                locale: def.prompt_lang.clone(),
                friendly_name: Some(format!("{} - {:?}", id, def.tags)),
            }
        }).collect();
        Ok(voices)
    }

    async fn synthesize(&self, segment: &AudioSegment, char_map: &CharacterMap) -> Result<Vec<u8>> {
        let voice_id = self.resolve_voice(&segment.speaker, char_map)
            .ok_or_else(|| anyhow!("No voice resolved for speaker: {}", segment.speaker))?;

        let voice_def = self.voice_library.get(&voice_id)
            .ok_or_else(|| anyhow!("Voice ID not found in library: {}", voice_id))?;

        let base_url = self.config.audio.sovits.as_ref()
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
        let resp = client.post(&url)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err_text = resp.text().await?;
            return Err(anyhow!("SoVITS API Error: {}", err_text));
        }

        let audio_data = resp.bytes().await?.to_vec();
        Ok(audio_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AudioConfig, EdgeTtsConfig};

    #[test]
    fn test_pick_random_voice() {
        let config = Config {
            input_folder: "".to_string(),
            output_folder: "".to_string(),
            build_folder: "".to_string(),
            unattended: false,
            llm: crate::config::LlmConfig {
                provider: "mock".to_string(),
                gemini: None,
                ollama: None,
                openai: None,
            },
            audio: AudioConfig {
                provider: "edge-tts".to_string(),
                language: "zh".to_string(),
                edge_tts: Some(EdgeTtsConfig {
                   narrator_voice: Some("Narrator".to_string()),
                   ..Default::default()
                }),
                ..Default::default()
            },
        };
        
        let voices = vec![
            Voice { short_name: "zh-CN-Male".to_string(), gender: "Male".to_string(), locale: "zh-CN".to_string(), name: "".to_string(), friendly_name: None },
            Voice { short_name: "zh-TW-Female".to_string(), gender: "Female".to_string(), locale: "zh-TW".to_string(), name: "".to_string(), friendly_name: None },
            Voice { short_name: "en-US-Male".to_string(), gender: "Male".to_string(), locale: "en-US".to_string(), name: "".to_string(), friendly_name: None },
        ];
        
        let client = EdgeTtsClient::new_with_voices(&config, voices);
        
        // Test filtering
        let v = client.pick_random_voice(Some("Male"));
        assert_eq!(v, "zh-CN-Male"); // Only one zh Male
        
        let v = client.pick_random_voice(Some("Female"));
        assert_eq!(v, "zh-TW-Female");
        
        // Test Neutral (should pick either zh-CN-Male or zh-TW-Female)
        let v = client.pick_random_voice(None);
        assert!(v == "zh-CN-Male" || v == "zh-TW-Female");
        
        // Test Language mismatch
        // If I change config language to "en"
        let mut config_en = config.clone();
        config_en.audio.language = "en".to_string();
        let client_en = EdgeTtsClient::new_with_voices(&config_en, client.voices_cache.clone());
        let v = client_en.pick_random_voice(Some("Male"));
        assert_eq!(v, "en-US-Male");
    }
}
