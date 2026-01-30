#[cfg(not(target_arch = "wasm32"))]
use anyhow::Result;
#[cfg(not(target_arch = "wasm32"))]
use novel2audiobook::core::config::Config;
#[cfg(not(target_arch = "wasm32"))]
use novel2audiobook::core::io::{NativeStorage, Storage};
#[cfg(not(target_arch = "wasm32"))]
use novel2audiobook::services::{llm, setup, tts, workflow::WorkflowManager};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    // 1. Load or Create Config
    let mut config = match Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Error loading config: {}", e);
            eprintln!("Please ensure 'config.yml' exists with valid LLM settings.");
            return Err(e);
        }
    };

    config.ensure_directories()?;

    // 2. Initialize LLM
    let llm = llm::create_llm(&config.llm)?;

    // 3. Initialize Storage
    let storage = Arc::new(NativeStorage::new());

    // 4. Interactive Setup (Voice Selection)
    setup::run_setup(&mut config, Some(llm.as_ref()), storage.clone()).await?;

    // 5. Initialize TTS
    let tts = tts::create_tts_client(&config, Some(llm.as_ref()), storage.clone()).await?;

    // 6. Initialize and Run Workflow
    let mut manager = WorkflowManager::new(config.clone(), llm, tts, storage).await?;
    manager.run().await?;

    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}
