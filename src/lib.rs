pub mod core;
pub mod services;
pub mod utils;
#[cfg(target_arch = "wasm32")]
pub mod ui;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Debug).unwrap_or(());
    
    leptos::mount_to_body(|| {
        use crate::ui::App;
        view! { <App/> }
    });
}

// --- Integration Test ---
#[cfg(target_arch = "wasm32")]
const TEST_CHAPTER_TEXT: &str = include_str!("../testassets/input_chapters/00001.txt");
#[cfg(target_arch = "wasm32")]
const TEST_SEGMENTS_JSON: &str = include_str!("../testassets/build/00001_txt/segments.json");
#[cfg(target_arch = "wasm32")]
const TEST_CHAR_MAP_JSON: &str = include_str!("../testassets/build/character_map.json");

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub async fn run_wasm_integration_test(base_url: String) -> Result<String, JsValue> {
    use std::sync::Arc;
    use crate::core::io::Storage;
    use crate::core::io::WebStorage;
    use crate::core::config::{Config, AudioConfig};
    use crate::services::workflow::WorkflowManager;
    use crate::services::tts::create_tts_client;

    // 1. Init Storage
    let storage = Arc::new(WebStorage::new().await.map_err(|e| JsValue::from_str(&e.to_string()))?);

    // 2. Setup Assets
    let input_folder = "input";
    let build_folder = "build";
    let output_folder = "output";

    // Write input chapter
    storage.write(&format!("{}/00001.txt", input_folder), TEST_CHAPTER_TEXT.as_bytes())
        .await.map_err(|e| JsValue::from_str(&e.to_string()))?;

    // Write cached segments (to bypass LLM)
    // Structure: build/00001_txt/segments.json
    storage.write(&format!("{}/00001_txt/segments.json", build_folder), TEST_SEGMENTS_JSON.as_bytes())
        .await.map_err(|e| JsValue::from_str(&e.to_string()))?;

    // Write character map
    storage.write(&format!("{}/character_map.json", build_folder), TEST_CHAR_MAP_JSON.as_bytes())
        .await.map_err(|e| JsValue::from_str(&e.to_string()))?;

    // 3. Config
    let config = Config {
        input_folder: input_folder.to_string(),
        output_folder: output_folder.to_string(),
        build_folder: build_folder.to_string(),
        unattended: true,
        llm: crate::services::llm::LlmConfig {
             provider: "gemini".to_string(),
             retry_count: 0,
             retry_delay_seconds: 0,
             gemini: Some(crate::services::llm::GeminiConfig { api_key: "dummy".to_string(), model: "dummy".to_string() }),
             ollama: None,
             openai: None,
        },
        audio: AudioConfig {
            provider: "qwen3_tts".to_string(),
            language: "zh".to_string(),
            qwen3_tts: Some(crate::services::tts::qwen3_tts::Qwen3TtsConfig {
                self_host: false,
                base_url: base_url,
                narrator_voice: None,
                concurrency: 1,
                device: None,
            }),
            exclude_locales: vec![],
        },
    };

    // 4. Create TTS Client
    let tts = create_tts_client(&config, None, storage.clone())
        .await.map_err(|e| JsValue::from_str(&e.to_string()))?;

    // 5. Run Workflow
    #[derive(Debug)]
    struct DummyLlm;
    #[async_trait::async_trait(?Send)]
    impl crate::services::llm::LlmClient for DummyLlm {
        async fn chat(&self, _: &str, _: &str) -> anyhow::Result<String> {
             Err(anyhow::anyhow!("LLM should not be called in this test"))
        }
    }

    let llm = Box::new(DummyLlm);

    let mut manager = WorkflowManager::new(config, llm, tts, storage.clone())
        .await.map_err(|e| JsValue::from_str(&e.to_string()))?;

    manager.run().await.map_err(|e| JsValue::from_str(&e.to_string()))?;

    // 6. Verify Output
    let output_file = format!("{}/00001.mp3", output_folder);
    if storage.exists(&output_file).await.map_err(|e| JsValue::from_str(&e.to_string()))? {
        Ok(format!("Success: Audio generated at {}", output_file))
    } else {
        Err(JsValue::from_str("Output audio file not found"))
    }
}
