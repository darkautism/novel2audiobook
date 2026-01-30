use anyhow::Result;
use novel2audiobook::core::config::Config;
use novel2audiobook::core::io::{NativeStorage, Storage};
use novel2audiobook::services::{llm, setup, tts, workflow::WorkflowManager};
use std::sync::Arc;

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

    // 3. Interactive Setup (Voice Selection)
    setup::run_setup(&mut config, Some(llm.as_ref())).await?;

    // 4. Initialize TTS
    let tts = tts::create_tts_client(&config, Some(llm.as_ref())).await?;

    // 5. Initialize Storage
    #[cfg(not(target_arch = "wasm32"))]
    let storage = Arc::new(NativeStorage::new());
    
    // For WASM, main is not used, but if it were, we'd need WebStorage.
    // Since main.rs is native binary entry point, NativeStorage is correct.

    // 6. Initialize and Run Workflow
    let mut manager = WorkflowManager::new(config.clone(), llm, tts, storage).await?;
    manager.run().await?;

    Ok(())
}
