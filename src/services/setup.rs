use crate::core::config::Config;
use crate::services::llm::LlmClient;
use crate::services::tts::{fetch_voice_list, Voice};
use crate::core::io::Storage;
use anyhow::{anyhow, Result};
#[cfg(not(target_arch = "wasm32"))]
use inquire::Select;
use std::sync::Arc;

#[cfg(target_arch = "wasm32")]
pub async fn run_setup(_config: &mut Config, _llm: Option<&dyn LlmClient>, _storage: Arc<dyn Storage>) -> Result<()> {
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn run_setup(config: &mut Config, llm: Option<&dyn LlmClient>, storage: Arc<dyn Storage>) -> Result<()> {
    {
        let mut needs_save = false;
        let provider = config.audio.provider.clone();

        match provider.as_str() {
            "edge-tts" => {
                if config.audio.edge_tts.is_none() {
                    config.audio.edge_tts = Some(Default::default());
                }

                // Check if setup needed
                let setup_needed = {
                    let cfg = config.audio.edge_tts.as_ref().unwrap();
                    cfg.narrator_voice.is_none()
                        || cfg.default_male_voice.is_none()
                        || cfg.default_female_voice.is_none()
                };

                if setup_needed {
                    println!("Fetching Edge-TTS voices...");
                    let voices = fetch_voice_list(config, llm, storage.clone()).await?;
                    let lang = &config.audio.language;
                    let filtered_voices: Vec<Voice> = voices
                        .into_iter()
                        .filter(|v| v.locale.starts_with(lang))
                        .collect();

                    if filtered_voices.is_empty() {
                        return Err(anyhow!("No voices found for language: {}", lang));
                    }

                    let cfg = config.audio.edge_tts.as_mut().unwrap();

                    if cfg.narrator_voice.is_none() {
                        cfg.narrator_voice = Some(select_voice(
                            "Select Narrator Voice:",
                            &filtered_voices,
                            |_| true,
                        )?);
                        needs_save = true;
                    }
                    if cfg.default_male_voice.is_none() {
                        cfg.default_male_voice = Some(select_voice(
                            "Select Default Male Voice:",
                            &filtered_voices,
                            |v| v.gender == "Male",
                        )?);
                        needs_save = true;
                    }
                    if cfg.default_female_voice.is_none() {
                        cfg.default_female_voice = Some(select_voice(
                            "Select Default Female Voice:",
                            &filtered_voices,
                            |v| v.gender == "Female",
                        )?);
                        needs_save = true;
                    }
                }
            }
            "gpt_sovits" => {
                if config.audio.gpt_sovits.is_none() {
                    config.audio.gpt_sovits = Some(crate::services::tts::gpt_sovits_config::GptSovitsConfig {
                        token: "".to_string(),
                        base_url: "https://gsv2p.acgnai.top/".to_string(),
                        top_k: 10,
                        top_p: 1,
                        temperature: 1,
                        speed_factor: 1,
                        repetition_penalty: 1.35,
                        narrator_voice: None,
                        ..Default::default()
                    });
                }

                let setup_needed = {
                    let cfg = config.audio.gpt_sovits.as_ref().unwrap();
                    cfg.narrator_voice.is_none()
                };

                if setup_needed {
                    println!("Fetching GPT-SoVITS models...");
                    let voices = fetch_voice_list(config, llm, storage.clone()).await?;

                    if voices.is_empty() {
                        return Err(anyhow!(
                            "No GPT-SoVITS models found. Please check internet connection or config."
                        ));
                    }

                    let cfg = config.audio.gpt_sovits.as_mut().unwrap();

                    if cfg.narrator_voice.is_none() {
                        cfg.narrator_voice =
                            Some(select_voice("Select Narrator Voice:", &voices, |_| true)?);
                        needs_save = true;
                    }
                }
            }
            "qwen3_tts" => {
                if config.audio.qwen3_tts.is_none() {
                    config.audio.qwen3_tts = Some(crate::services::tts::qwen3_tts::Qwen3TtsConfig {
                        self_host: true,
                        base_url: "http://127.0.0.1:8000".to_string(),
                        narrator_voice: None,
                        concurrency: 1,
                        device: None,
                    });
                }

                let setup_needed = {
                    let cfg = config.audio.qwen3_tts.as_ref().unwrap();
                    cfg.narrator_voice.is_none()
                };

                if setup_needed {
                    println!("Fetching Qwen3-TTS voices...");
                    let voices = fetch_voice_list(config, llm, storage.clone()).await?;
                    let lang = &config.audio.language;
                    let filtered_voices: Vec<Voice> = voices
                        .into_iter()
                        .filter(|v| v.locale.starts_with(lang))
                        .collect();

                    if filtered_voices.is_empty() {
                        return Err(anyhow!("No voices found for language: {}", lang));
                    }

                    let cfg = config.audio.qwen3_tts.as_mut().unwrap();

                    if cfg.narrator_voice.is_none() {
                        cfg.narrator_voice = Some(select_voice(
                            "Select Narrator Voice:",
                            &filtered_voices,
                            |_| true,
                        )?);
                        needs_save = true;
                    }
                }
            }
            _ => {
                println!("Setup not implemented for provider: {}", provider);
            }
        }

        if needs_save {
            config.save()?;
            println!("Configuration saved.");
        }

        Ok(())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn select_voice<F>(prompt: &str, voices: &[Voice], filter: F) -> Result<String>
where
    F: Fn(&Voice) -> bool,
{
    let filtered: Vec<&Voice> = voices.iter().filter(|v| filter(v)).collect();

    // Fallback if filter leaves nothing (e.g. no Male voices found), show all
    let options_source = if filtered.is_empty() {
        voices.iter().collect()
    } else {
        filtered
    };

    let options: Vec<String> = options_source
        .iter()
        .map(|v| {
            format!(
                "{} ({}) - {}",
                v.short_name,
                v.gender,
                v.friendly_name.as_deref().unwrap_or(&v.name)
            )
        })
        .collect();

    let selection = Select::new(prompt, options).prompt()?;

    let short_name = selection.split_whitespace().next().unwrap().to_string();
    Ok(short_name)
}
