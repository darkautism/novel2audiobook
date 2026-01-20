mod config;
mod llm;
mod tts;
mod setup;
mod workflow;

use anyhow::Result;
use config::Config;
use workflow::WorkflowManager;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    
    // 1. Load or Create Config
    // If load fails, we should check if it's because it doesn't exist.
    // If it doesn't exist, we can't do much without API keys.
    // But we can verify directories.
    let mut config = match Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Error loading config: {}", e);
            eprintln!("Please ensure 'config.yml' exists with valid LLM settings.");
            return Err(e);
        }
    };

    config.ensure_directories()?;

    // 2. Interactive Setup (Voice Selection)
    setup::run_setup(&mut config).await?;

    // 3. Initialize LLM
    let llm = llm::create_llm(&config)?;

    // 4. Initialize TTS
    let tts = tts::create_tts_client(&config)?;

    // 5. Initialize and Run Workflow
    let mut manager = WorkflowManager::new(config.clone(), llm, tts)?;
    manager.run().await?;

    Ok(())
}
