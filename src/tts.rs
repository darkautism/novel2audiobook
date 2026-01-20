use anyhow::{Result, anyhow, Context};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use futures::{StreamExt, SinkExt};
use uuid::Uuid;
use chrono::Utc;

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const LIST_VOICES_URL: &str = "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list";
const WSS_URL: &str = "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1";

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct Voice {
    pub name: String,
    pub short_name: String,
    pub gender: String,
    pub locale: String,
    pub friendly_name: Option<String>,
}

pub struct EdgeTtsClient;

impl EdgeTtsClient {
    pub async fn list_voices() -> Result<Vec<Voice>> {
        let url = format!("{}?trustedclienttoken={}", LIST_VOICES_URL, TRUSTED_CLIENT_TOKEN);
        let client = reqwest::Client::new();
        let mut headers = HeaderMap::new();
        headers.insert("Authority", HeaderValue::from_static("speech.platform.bing.com"));
        headers.insert("Sec-CH-UA", HeaderValue::from_static("\"Not_A Brand\";v=\"8\", \"Chromium\";v=\"120\", \"Microsoft Edge\";v=\"120\""));
        headers.insert("Sec-CH-UA-Mobile", HeaderValue::from_static("?0"));
        headers.insert("User-Agent", HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0"));
        headers.insert("Sec-CH-UA-Platform", HeaderValue::from_static("\"Windows\""));
        headers.insert("Accept", HeaderValue::from_static("*/*"));
        headers.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
        headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("cors"));
        headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("empty"));
        headers.insert("Accept-Encoding", HeaderValue::from_static("gzip, deflate, br"));
        headers.insert("Accept-Language", HeaderValue::from_static("en-US,en;q=0.9"));

        let resp = client.get(&url).headers(headers).send().await?;
        if !resp.status().is_success() {
             return Err(anyhow!("Failed to list voices: {}", resp.status()));
        }
        let voices: Vec<Voice> = resp.json().await?;
        Ok(voices)
    }

    pub async fn synthesize(ssml: &str) -> Result<Vec<u8>> {
        let uuid = Uuid::new_v4().to_string().replace("-", "");
        let url = format!("{}?TrustedClientToken={}&ConnectionId={}", WSS_URL, TRUSTED_CLIENT_TOKEN, uuid);
        let (ws_stream, _) = connect_async(&url).await.context("Failed to connect to Edge TTS WebSocket")?;
        let (mut write, mut read) = ws_stream.split();

        // 1. Send config
        let config_msg = format!(
            "X-Timestamp:{}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}",
            current_time_str()
        );
        write.send(Message::Text(config_msg)).await?;

        // 2. Send SSML
        let request_id = Uuid::new_v4().to_string().replace("-", "");
        let ssml_msg = format!(
            "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}\r\nPath:ssml\r\n\r\n{}",
            request_id,
            current_time_str(),
            ssml
        );
        write.send(Message::Text(ssml_msg)).await?;

        // 3. Collect audio
        let mut audio_data = Vec::new();
        let mut turn_end = false;

        while let Some(msg) = read.next().await {
            let msg = msg?;
            match msg {
                Message::Binary(data) => {
                    if let Some(pos) = find_subsequence(&data, b"Path:audio\r\n") {
                        // Find the start of the payload (after headers)
                        if let Some(header_end) = find_subsequence(&data[pos..], b"\r\n\r\n") {
                             let payload_start = pos + header_end + 4;
                             if payload_start < data.len() {
                                 audio_data.extend_from_slice(&data[payload_start..]);
                             }
                        }
                    } else if let Some(_) = find_subsequence(&data, b"Path:turn.end") {
                        turn_end = true;
                        break;
                    }
                },
                Message::Text(text) => {
                    if text.contains("Path:turn.end") {
                        turn_end = true;
                        break;
                    }
                },
                _ => {}
            }
        }
        
        if !turn_end {
             // It's possible the connection closed cleanly but we got audio.
             // Or we might have errored.
        }

        Ok(audio_data)
    }
}

fn current_time_str() -> String {
    // Format: "Thu Jun 15 2023 12:00:00 GMT+0000 (Coordinated Universal Time)"
    // Or just ISO. The python library uses a simple date string.
    Utc::now().to_rfc2822()
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|window| window == needle)
}
