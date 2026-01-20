use crate::config::Config;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use http::Request;
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{self, protocol::Message},
};
use url::Url;
use uuid::Uuid;

// --- Constants based on Python Version ---

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const BASE_URL: &str = "speech.platform.bing.com/consumer/speech/synthesize/readaloud";

// Chromium Versions
const CHROMIUM_FULL_VERSION: &str = "143.0.3650.75";
const CHROMIUM_MAJOR_VERSION: &str = "143"; // Split from full version

// URLs
const LIST_VOICES_URL: &str = "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list";
const WSS_BASE_URL: &str = "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";

// Constant: The difference between Windows Epoch (1601) and Unix Epoch (1970) in seconds.
const WIN_EPOCH_DIFF: i64 = 11_644_473_600;

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Voice {
    pub name: String,
    pub short_name: String,
    pub gender: String,
    pub locale: String,
    pub friendly_name: Option<String>,
}

/// Generates the Sec-MS-GEC authentication token.
pub fn generate_sec_ms_gec() -> String {
    let now = Utc::now();
    let unix_seconds = now.timestamp();
    let windows_seconds = unix_seconds + WIN_EPOCH_DIFF;
    // Round down to the nearest 5-minute (300 seconds) window
    let rounded_seconds = windows_seconds - (windows_seconds % 300);
    
    // Format as "ticks" (append 7 zeros)
    let ticks_str = format!("{}0000000", rounded_seconds);
    let input_string = format!("{}{}", ticks_str, TRUSTED_CLIENT_TOKEN);

    let mut hasher = Sha256::new();
    hasher.update(input_string);
    let hash_result = hasher.finalize();

    hex::encode(hash_result).to_uppercase()
}

/// Returns the GEC-Version header matching the Python logic: f"1-{CHROMIUM_FULL_VERSION}"
pub fn get_sec_ms_gec_version() -> String {
    format!("1-{}", CHROMIUM_FULL_VERSION)
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
        "edge-tts-online" => Ok(Box::new(EdgeTtsOnlineClient)),
        "edge-tts" => Ok(Box::new(EdgeTtsNativeClient)),
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

fn current_time_str() -> String {
    let now = Utc::now();
    format!(
        "{}",
        now.format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)")
    )
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// --- Edge TTS Online Client (WebSocket Implementation) ---

pub struct EdgeTtsOnlineClient;

#[async_trait]
impl TtsClient for EdgeTtsOnlineClient {
    async fn list_voices(&self) -> Result<Vec<Voice>> {
        list_voices_http().await
    }

    async fn synthesize(&self, ssml: &str) -> Result<Vec<u8>> {
        let connection_id = Uuid::new_v4().simple().to_string();
        let url_string = format!(
            "{}?TrustedClientToken={}&ConnectionId={}",
            WSS_BASE_URL, TRUSTED_CLIENT_TOKEN, connection_id
        );
        let url = Url::parse(&url_string)?;

        // 3. Generate Auth Tokens
        let gec_token = generate_sec_ms_gec();
        let gec_version = get_sec_ms_gec_version();

        // 4. Build the HTTP Request for the Handshake
        let request = Request::builder()
            .method("GET")
            .uri(url.as_str())
            .header("Host", "speech.platform.bing.com")
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Pragma", "no-cache")
            .header("Cache-Control", "no-cache")
            .header("User-Agent", get_user_agent())
            .header("Origin", "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", tungstenite::handshake::client::generate_key())
            .header("Accept-Encoding", "gzip, deflate, br, zstd")
            .header("Accept-Language", "en-US,en;q=0.9")
            // Auth Headers
            .header("Sec-MS-GEC", gec_token)
            .header("Sec-MS-GEC-Version", gec_version)
            .body(())?;

        // 5. Execute the Handshake
        let (mut ws_stream, response) = connect_async(request).await
            .map_err(|e| anyhow!("WebSocket connection failed: {}", e))?;

        if !response.status().is_informational() {
             println!("Handshake response status: {}", response.status());
        }

        let (mut write, mut read) = ws_stream.split();

        let config_msg = format!(
            "X-Timestamp:{}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}",
            current_time_str()
        );
        write.send(Message::Text(config_msg.into())).await?;

        let request_id = Uuid::new_v4().to_string().replace("-", "");
        let ssml_msg = format!(
            "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}\r\nPath:ssml\r\n\r\n{}",
            request_id,
            current_time_str(),
            ssml
        );
        write.send(Message::Text(ssml_msg.into())).await?;

        let mut audio_data = Vec::new();
        let mut turn_end = false;

        while let Some(msg) = read.next().await {
            let msg = msg?;
            match msg {
                Message::Binary(data) => {
                    if let Some(pos) = find_subsequence(&data, b"Path:audio\r\n") {
                        if let Some(header_end) = find_subsequence(&data[pos..], b"\r\n\r\n") {
                            let payload_start = pos + header_end + 4;
                            if payload_start < data.len() {
                                audio_data.extend_from_slice(&data[payload_start..]);
                            }
                        }
                    } else if find_subsequence(&data, b"Path:turn.end").is_some() {
                        turn_end = true;
                        break;
                    }
                }
                Message::Text(text) => {
                    if text.contains("Path:turn.end") {
                        turn_end = true;
                        break;
                    }
                }
                _ => {}
            }
        }

        if !turn_end {
             // Handle incomplete stream if necessary
        }

        Ok(audio_data)
    }
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