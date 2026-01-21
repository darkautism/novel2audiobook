use crate::config::Config;
use crate::script::AudioSegment;
use crate::state::CharacterMap;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rand::seq::IndexedRandom;
use reqwest::header::{HeaderMap, HeaderValue};

use crate::tts::{Voice, TtsClient, VOICE_ID_MOB_MALE, VOICE_ID_MOB_FEMALE, VOICE_ID_MOB_NEUTRAL};

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const CHROMIUM_MAJOR_VERSION: &str = "143";
const LIST_VOICES_URL: &str =
    "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list";

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

// --- Shared Helper for EdgeTTS ---

pub async fn list_voices() -> Result<Vec<Voice>> {
    let url = format!(
        "{}?trustedclienttoken={}",
        LIST_VOICES_URL, TRUSTED_CLIENT_TOKEN
    );
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();

    headers.insert(
        "Authority",
        HeaderValue::from_static("speech.platform.bing.com"),
    );
    headers.insert("Sec-CH-UA", HeaderValue::from_str(&get_sec_ch_ua())?);
    headers.insert("Sec-CH-UA-Mobile", HeaderValue::from_static("?0"));
    headers.insert("User-Agent", HeaderValue::from_str(&get_user_agent())?);
    headers.insert(
        "Sec-CH-UA-Platform",
        HeaderValue::from_static("\"Windows\""),
    );
    headers.insert("Accept", HeaderValue::from_static("*/*"));
    headers.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
    headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("cors"));
    headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("empty"));
    headers.insert(
        "Accept-Encoding",
        HeaderValue::from_static("gzip, deflate, br, zstd"),
    );
    headers.insert(
        "Accept-Language",
        HeaderValue::from_static("en-US,en;q=0.9"),
    );

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
        let voices_cache = list_voices().await.unwrap_or_else(|e| {
            eprintln!(
                "Warning: Failed to fetch EdgeTTS voices for random selection: {}",
                e
            );
            Vec::new()
        });
        Ok(Self {
            config: config.clone(),
            voices_cache,
        })
    }

    #[cfg(test)]
    pub fn new_with_voices(config: &Config, voices: Vec<Voice>) -> Self {
        Self {
            config: config.clone(),
            voices_cache: voices,
        }
    }

    pub fn pick_random_voice(&self, gender: Option<&str>, excluded_voices: &[String]) -> String {
        let lang_prefix = &self.config.audio.language;
        let mut rng = rand::rng();

        let candidates: Vec<&Voice> = self
            .voices_cache
            .iter()
            .filter(|v| {
                if !v.locale.starts_with(lang_prefix) {
                    return false;
                }
                if self.config.audio.exclude_locales.contains(&v.locale) {
                    return false;
                }
                if excluded_voices.contains(&v.short_name) {
                    return false;
                }
                if let Some(g) = gender {
                    if !v.gender.eq_ignore_ascii_case(g) {
                        return false;
                    }
                }
                true
            })
            .collect();

        if let Some(v) = candidates.choose(&mut rng) {
            v.short_name.clone()
        } else {
            // Fallback
            self.config
                .audio
                .edge_tts
                .as_ref()
                .and_then(|c| c.narrator_voice.clone())
                .unwrap_or_else(|| "zh-TW-HsiaoChenNeural".to_string())
        }
    }

    fn resolve_voice(
        &self,
        speaker: &str,
        char_map: &CharacterMap,
        excluded_voices: &[String],
    ) -> String {
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
                    VOICE_ID_MOB_MALE => {
                        return self.pick_random_voice(Some("Male"), excluded_voices)
                    }
                    VOICE_ID_MOB_FEMALE => {
                        return self.pick_random_voice(Some("Female"), excluded_voices)
                    }
                    VOICE_ID_MOB_NEUTRAL => return self.pick_random_voice(None, excluded_voices),
                    _ => return voice_id.clone(),
                }
            }

            // 3. Fallback to Gender Default
            if let Some(cfg) = edge_config {
                match info.gender.to_lowercase().as_str() {
                    "male" => {
                        if let Some(v) = &cfg.default_male_voice {
                            return v.clone();
                        }
                    }
                    "female" => {
                        if let Some(v) = &cfg.default_female_voice {
                            return v.clone();
                        }
                    }
                    _ => {}
                }
            }
        }

        // 4. Ultimate Fallback (Narrator or first available)
        if let Some(cfg) = edge_config {
            if let Some(v) = &cfg.narrator_voice {
                return v.clone();
            }
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
            list_voices().await
        }
    }

    async fn synthesize(
        &self,
        segment: &AudioSegment,
        char_map: &CharacterMap,
        excluded_voices: &[String],
    ) -> Result<Vec<u8>> {
        let voice = self.resolve_voice(&segment.speaker, char_map, excluded_voices);
        let using_style = self.config.audio.edge_tts.clone().unwrap_or_default().style;
        let ssml = match (using_style, &segment.style) {
            (true, Some(style)) =>format!(
                "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'><voice name='{}'><mstts:express-as style='{}'>{}</mstts:express-as></voice></speak>",
                voice, style, segment.text
            ),
            _ =>
            format!(
                "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'><voice name='{}'>{}</voice></speak>",
                voice, segment.text
            )
        };

        tokio::task::spawn_blocking(move || {
            edge_tts::request_audio(&ssml, "audio-24khz-48kbitrate-mono-mp3")
                .map_err(|e| anyhow!("Edge TTS crate error: {:?}", e))
        })
        .await?
    }

    async fn get_random_voice(
        &self,
        gender: Option<&str>,
        excluded_voices: &[String],
    ) -> Result<String> {
        Ok(self.pick_random_voice(gender, excluded_voices))
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
                retry_count: 0,
                retry_delay_seconds: 0,
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

        let client = EdgeTtsClient::new_with_voices(&config, voices);

        // Test filtering
        let v = client.pick_random_voice(Some("Male"), &[]);
        assert_eq!(v, "zh-CN-Male"); // Only one zh Male

        let v = client.pick_random_voice(Some("Female"), &[]);
        assert_eq!(v, "zh-TW-Female");

        // Test Neutral (should pick either zh-CN-Male or zh-TW-Female)
        let v = client.pick_random_voice(None, &[]);
        assert!(v == "zh-CN-Male" || v == "zh-TW-Female");

        // Test Language mismatch
        // If I change config language to "en"
        let mut config_en = config.clone();
        config_en.audio.language = "en".to_string();
        let client_en = EdgeTtsClient::new_with_voices(&config_en, client.voices_cache.clone());
        let v = client_en.pick_random_voice(Some("Male"), &[]);
        assert_eq!(v, "en-US-Male");

        // Test Exclude Locales
        let mut config_ex = config.clone();
        config_ex.audio.exclude_locales = vec!["zh-TW".to_string()];
        let client_ex = EdgeTtsClient::new_with_voices(&config_ex, client.voices_cache.clone());

        // zh-TW-Female should be excluded
        // so if we ask for Female, and only zh-TW-Female is available (which matches lang zh), it should fallback
        let v_female = client_ex.pick_random_voice(Some("Female"), &[]);
        assert_eq!(v_female, "Narrator");

        // If we ask for Neutral/None, it should pick zh-CN-Male because zh-TW-Female is excluded
        let v_neutral = client_ex.pick_random_voice(None, &[]);
        assert_eq!(v_neutral, "zh-CN-Male");

        // Test Excluded Voices
        let v_excluded = client.pick_random_voice(Some("Male"), &["zh-CN-Male".to_string()]);
        // Should fallback to narrator because the only male voice is excluded
        assert_eq!(v_excluded, "Narrator");
    }
}
