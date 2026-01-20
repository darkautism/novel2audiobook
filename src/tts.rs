use crate::config::Config;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;

// --- Constants based on Python Version ---

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";

// Chromium Versions
const CHROMIUM_MAJOR_VERSION: &str = "143"; // Split from full version

// URLs
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

/// Helper to generate the User-Agent string dynamically
fn get_user_agent() -> String {
    format!(
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{}.0.0.0 Safari/537.36 Edg/{}.0.0.0",
        CHROMIUM_MAJOR_VERSION, CHROMIUM_MAJOR_VERSION
    )
}

/// Helper to generate the Sec-CH-UA string dynamically
fn get_sec_ch_ua() -> String {
    format!(
        "\" Not;A Brand\";v=\"99\", \"Microsoft Edge\";v=\"{}\", \"Chromium\";v=\"{}\"",
        CHROMIUM_MAJOR_VERSION, CHROMIUM_MAJOR_VERSION
    )
}

#[async_trait]
pub trait TtsClient: Send + Sync {
    async fn list_voices(&self) -> Result<Vec<Voice>>;
    async fn synthesize(&self, ssml: &str) -> Result<Vec<u8>>;
}

pub fn create_tts_client(config: &Config) -> Result<Box<dyn TtsClient>> {
    match config.audio.provider.as_str() {
        "edge-tts" | "edge-tts-online" => Ok(Box::new(EdgeTtsNativeClient)),
        _ => Err(anyhow!("Unknown TTS provider: {}", config.audio.provider)),
    }
}

// --- Shared Helper ---

async fn list_voices_http() -> Result<Vec<Voice>> {
    let url = format!(
        "{}?trustedclienttoken={}",
        LIST_VOICES_URL, TRUSTED_CLIENT_TOKEN
    );
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    
    // Construct headers to match Python's BASE_HEADERS + VOICE_HEADERS
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

// --- Edge TTS Native Client (Using Crate) ---

pub struct EdgeTtsNativeClient;

#[async_trait]
impl TtsClient for EdgeTtsNativeClient {
    async fn list_voices(&self) -> Result<Vec<Voice>> {
        list_voices_http().await
    }

    async fn synthesize(&self, ssml: &str) -> Result<Vec<u8>> {
        let ssml = ssml.to_string();
        tokio::task::spawn_blocking(move || {
            edge_tts::request_audio(&ssml, "audio-24khz-48kbitrate-mono-mp3")
                .map_err(|e| anyhow!("Edge TTS crate error: {:?}", e))
        })
        .await?
    }
}
