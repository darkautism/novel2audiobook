use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use tokio_tungstenite::{
    connect_async, tungstenite::client::IntoClientRequest, tungstenite::protocol::Message,
};
use uuid::Uuid;

const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
const LIST_VOICES_URL: &str =
    "https://speech.platform.bing.com/consumer/speech/synthesize/readaloud/voices/list";
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
        headers.insert(
            "Sec-CH-UA",
            HeaderValue::from_static(
                "\"Not_A Brand\";v=\"8\", \"Chromium\";v=\"120\", \"Microsoft Edge\";v=\"120\"",
            ),
        );
        headers.insert("Sec-CH-UA-Mobile", HeaderValue::from_static("?0"));
        headers.insert("User-Agent", HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0"));
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
            HeaderValue::from_static("gzip, deflate, br"),
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
    pub async fn synthesize(ssml: &str) -> Result<Vec<u8>> {
        let uuid = Uuid::new_v4().to_string().replace("-", "");
        let url = format!(
            "{}?TrustedClientToken={}&ConnectionId={}",
            WSS_URL, TRUSTED_CLIENT_TOKEN, uuid
        );

        let mut request = url.into_client_request()?;
        let headers = request.headers_mut();

        // 1. 模擬更真實的瀏覽器 User-Agent (使用 Edge 120+)
        headers.insert("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36 Edg/121.0.0.0".parse()?);

        // 2. Origin 必須嚴格匹配
        headers.insert(
            "Origin",
            "chrome-extension://jdiccldimpdaibmpdkjnbmckianbfold".parse()?,
        );

        // 3. 其他標準 Header
        headers.insert("Pragma", "no-cache".parse()?);
        headers.insert("Cache-Control", "no-cache".parse()?);
        headers.insert("Accept-Language", "en-US,en;q=0.9".parse()?);

        // 注意：如果是 tokio-tungstenite，建議這裡不要手動加 Accept-Encoding，
        // 除非您有配置 extension，否則讓底層自動處理比較安全。

        // 4. 建立連線 (確保 Cargo.toml 使用了 native-tls)
        let (ws_stream, _) = connect_async(request)
            .await
            .context("Failed to connect to Edge TTS WebSocket (Check TLS config in Cargo.toml)")?;

        let (mut write, mut read) = ws_stream.split();

        // 1. Send config
        let config_msg = format!(
            "X-Timestamp:{}\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n{{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":{{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"false\"}},\"outputFormat\":\"audio-24khz-48kbitrate-mono-mp3\"}}}}}}}}",
            current_time_str()
        );
        write.send(Message::Text(config_msg.into())).await?;

        // 2. Send SSML
        let request_id = Uuid::new_v4().to_string().replace("-", "");
        let ssml_msg = format!(
            "X-RequestId:{}\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:{}\r\nPath:ssml\r\n\r\n{}",
            request_id,
            current_time_str(),
            ssml
        );
        write.send(Message::Text(ssml_msg.into())).await?;

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
            // It's possible the connection closed cleanly but we got audio.
            // Or we might have errored.
        }

        Ok(audio_data)
    }
}

// 建議修改時間格式函數，使其更像 JavaScript 的 Date().toString()
// 雖然 RFC2822 通常也能過，但越像官方越安全
fn current_time_str() -> String {
    // 範例輸出: "Thu Jun 15 2023 12:00:00 GMT+0000 (Coordinated Universal Time)"
    // 使用 chrono 自定義格式
    let now = Utc::now();
    format!("{}", now.format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)"))
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
