use futures_util::StreamExt;
use reqwest::{multipart, Client};
use serde_json::{json, Value};
use std::error::Error;
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
) -> Result<Vec<u8>, Box<dyn Error>> {
    let client = Client::new();

    // --- 第一步：上傳檔案 ---
    debug!("正在讀取並上傳檔案: {}", voice_file_path);
    let file_content = tokio::fs::read(voice_file_path).await?;
    let part = multipart::Part::bytes(file_content)
        .file_name("model.pt")
        .mime_str("application/octet-stream")?;

    let form = multipart::Form::new().part("files", part);

    let upload_resp = client
        .post(format!("{}/gradio_api/upload", base_url))
        .multipart(form)
        .send()
        .await?
        .json::<Vec<String>>()
        .await?;

    let uploaded_server_path = upload_resp.get(0).ok_or("Upload failed: empty response")?;
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
        .await?
        .json::<Value>()
        .await?;

    let event_id = gen_resp["event_id"].as_str().ok_or("No event_id found")?;
    debug!("任務 ID: {}", event_id);

    // --- 第三步：監聽 SSE Stream 直到完成 ---
    let mut stream = client
        .get(format!("{}/gradio_api/call/load_prompt_and_gen/{}", base_url, event_id))
        .send()
        .await?
        .bytes_stream();

    let mut download_path = String::new();

    while let Some(item) = stream.next().await {
        let chunk = item?;
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
        return Err("Failed to get output file path from stream".into());
    }

    // --- 第四步：下載最終檔案內容 ---
    debug!("正在下載檔案資料...");
    let download_url = format!("{}/gradio_api/file={}", base_url, download_path);
    let file_bytes = client.get(download_url).send().await?.bytes().await?;

    debug!("下載成功，大小: {} bytes", file_bytes.len());
    Ok(file_bytes.to_vec())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // 這裡可以初始化 logger 測試，不初始化則不會有輸出
    // env_logger::init(); 

    let audio_data = qwen3_tts_infer(
        "http://127.0.0.1:8000",
        "E:\\project\\novel2audiobook\\qwen3_tts_voices\\zh-星穹铁道_大毫-angry.pt",
        "你好，這是一段自動生成的語音測試。",
        "Chinese"
    ).await?;

    // 測試：將結果存檔
    tokio::fs::write("output.wav", &audio_data).await?;
    
    Ok(())
}