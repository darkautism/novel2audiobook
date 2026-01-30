use futures_util::StreamExt;
use reqwest::{multipart, Client};
use serde_json::{json, Value};
use anyhow::{Result, Context, anyhow};
use log::{debug, warn};
#[cfg(not(target_arch = "wasm32"))]
use tokio::time::{sleep, Duration};
#[cfg(target_arch = "wasm32")]
use std::time::Duration;

#[cfg(target_arch = "wasm32")]
async fn sleep(duration: Duration) {
    use wasm_bindgen::prelude::*;
    let millis = duration.as_millis() as i32;
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        web_sys::window().unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis)
            .unwrap();
    });
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

/// Qwen3 TTS 推論函式
/// 
/// * `base_url`: Gradio 伺服器位址 (例如 "http://127.0.0.1:8000")
/// * `voice_data`: 本地 .pt 權重檔案內容
/// * `text`: 想要合成的文字
/// * `language`: 語言設定 (例如 "Chinese")
pub async fn qwen3_tts_infer(
    base_url: &str,
    voice_data: &[u8],
    text: &str,
    language: &str,
) -> Result<Vec<u8>> {
    let client = Client::new();

    // --- 第一步：上傳檔案 ---
    debug!("正在上傳語音檔案 ({} bytes)", voice_data.len());
    let part = multipart::Part::bytes(voice_data.to_vec())
        .file_name("model.pt")
        .mime_str("application/octet-stream")
        .context("Invalid mime type")?;

    let form = multipart::Form::new().part("files", part);

    let upload_resp = client
        .post(format!("{}/gradio_api/upload", base_url))
        .multipart(form)
        .send()
        .await
        .context("Failed to send upload request")?
        .json::<Vec<String>>()
        .await
        .context("Failed to parse upload response")?;

    let uploaded_server_path = upload_resp.first().ok_or(anyhow!("Upload failed: empty response"))?;
    debug!("檔案已上傳至伺服器: {}", uploaded_server_path);

    // 重試機制
    let max_retries = 3;
    let mut last_error = anyhow!("Unknown error");

    for attempt in 0..max_retries {
        if attempt > 0 {
            warn!("Qwen3 TTS 生成失敗 (嘗試 {}/{})，正在重試...", attempt + 1, max_retries);
            sleep(Duration::from_secs(2)).await;
        }

        match try_generate_and_download(&client, base_url, uploaded_server_path, text, language).await {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
                warn!("Qwen3 TTS 生成錯誤: {:#}", e);
                last_error = e;
            }
        }
    }

    Err(last_error.context("Qwen3 TTS 生成在重試後仍然失敗"))
}

async fn try_generate_and_download(
    client: &Client,
    base_url: &str,
    uploaded_server_path: &str,
    text: &str,
    language: &str,
) -> Result<Vec<u8>> {
    // --- 第二步：提交生成任務 ---
    debug!("正在提交生成請求...");
    let gen_url = format!("{}/gradio_api/call/load_prompt_and_gen", base_url);
    let payload = json!({
        "data": [
            {
                "path": uploaded_server_path,
                "meta": {"_type": "gradio.FileData"}
            },
            text,
            language
        ]
    });

    let gen_resp = client
        .post(gen_url)
        .json(&payload)
        .send()
        .await
        .context("Failed to send gen request")?
        .json::<Value>()
        .await
        .context("Failed to parse gen response")?;

    let event_id = gen_resp["event_id"].as_str().ok_or(anyhow!("No event_id found"))?;
    debug!("任務 ID: {}", event_id);

    // --- 第三步：監聽 SSE Stream 直到完成 ---
    let mut stream = client
        .get(format!("{}/gradio_api/call/load_prompt_and_gen/{}", base_url, event_id))
        .send()
        .await
        .context("Failed to connect to event stream")?
        .bytes_stream();

    let mut download_path = String::new();

    while let Some(item) = stream.next().await {
        let chunk = item.context("Stream error")?;
        let chunk_text = String::from_utf8_lossy(&chunk);

        for line in chunk_text.lines() {
            if let Some(json_str) = line.strip_prefix("data: ") {
                if json_str.is_empty() || json_str == "null" { continue; }

                if let Ok(data) = serde_json::from_str::<Value>(json_str) {
                    // 解析 Gradio 的回傳結構
                    if let Some(output_array) = data.as_array() {
                        if let Some(file_info) = output_array.first() {
                            if let Some(final_path) = file_info["path"].as_str() {
                                debug!("生成完成，取得路徑: {}", final_path);
                                download_path = final_path.to_string();
                                break;
                            }
                        }
                    }
                }
            }
        }
        if !download_path.is_empty() { break; }
    }

    if download_path.is_empty() {
        return Err(anyhow!("Failed to get output file path from stream"));
    }

    // --- 第四步：下載最終檔案內容 ---
    debug!("正在下載檔案資料...");
    let download_url = format!("{}/gradio_api/file={}", base_url, download_path);
    let file_bytes = client.get(download_url).send().await.context("Failed to download result")?.bytes().await.context("Failed to get bytes")?;

    debug!("下載成功，大小: {} bytes", file_bytes.len());
    Ok(file_bytes.to_vec())
}
