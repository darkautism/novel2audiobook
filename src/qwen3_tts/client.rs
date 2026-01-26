use futures_util::StreamExt;
use reqwest::{multipart, Client};
use serde_json::{json, Value};
use anyhow::{Result, Context, anyhow};
use log::debug;

/// Qwen3 TTS 推論函式
/// 
/// * `base_url`: Gradio 伺服器位址 (例如 "http://127.0.0.1:8000")
/// * `voice_file_path`: 本地 .pt 權重檔案路徑
/// * `text`: 想要合成的文字
/// * `language`: 語言設定 (例如 "Chinese")
pub async fn qwen3_tts_infer(
    base_url: &str,
    voice_file_path: &str,
    text: &str,
    language: &str,
) -> Result<Vec<u8>> {
    let client = Client::new();

    // --- 第一步：上傳檔案 ---
    debug!("正在讀取並上傳檔案: {}", voice_file_path);
    let file_content = tokio::fs::read(voice_file_path).await.context("Failed to read voice file")?;
    let part = multipart::Part::bytes(file_content)
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

    let uploaded_server_path = upload_resp.get(0).ok_or(anyhow!("Upload failed: empty response"))?;
    debug!("檔案已上傳至伺服器: {}", uploaded_server_path);

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
            if line.starts_with("data: ") {
                let json_str = &line[6..];
                if json_str.is_empty() || json_str == "null" { continue; }

                if let Ok(data) = serde_json::from_str::<Value>(json_str) {
                    // 解析 Gradio 的回傳結構
                    if let Some(output_array) = data.as_array() {
                        if let Some(file_info) = output_array.get(0) {
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
