use crate::config::Config;
use crate::tts::{EdgeTtsClient, Voice};
use anyhow::{Result, anyhow};
use inquire::Select;

pub async fn run_setup(config: &mut Config) -> Result<()> {
    let mut needs_save = false;

    // 1. Check if we need to select voices
    if config.audio.narrator_voice.is_none() 
        || config.audio.default_male_voice.is_none() 
        || config.audio.default_female_voice.is_none() 
    {
        println!("Voice settings missing. Fetching available voices for language: {}...", config.audio.language);
        
        let voices = EdgeTtsClient::list_voices().await?;
        let filtered_voices: Vec<Voice> = voices.into_iter()
            .filter(|v| v.locale.starts_with(&config.audio.language))
            .collect();

        if filtered_voices.is_empty() {
            return Err(anyhow!("No voices found for language: {}", config.audio.language));
        }

        let voice_options: Vec<String> = filtered_voices.iter()
            .map(|v| format!("{} ({}) - {}", v.short_name, v.gender, v.friendly_name.as_deref().unwrap_or(&v.name)))
            .collect();

        // Helper to find short_name from selection
        let find_short_name = |selection: &str| -> String {
             let short_name = selection.split_whitespace().next().unwrap();
             short_name.to_string()
        };

        if config.audio.narrator_voice.is_none() {
             let selection = Select::new("Select Narrator Voice:", voice_options.clone())
                .prompt()?;
             config.audio.narrator_voice = Some(find_short_name(&selection));
             needs_save = true;
        }

        if config.audio.default_male_voice.is_none() {
             let male_options: Vec<String> = filtered_voices.iter()
                .filter(|v| v.gender == "Male")
                .map(|v| format!("{} ({}) - {}", v.short_name, v.gender, v.friendly_name.as_deref().unwrap_or(&v.name)))
                .collect();
            
             // Fallback to all if no male voices found (rare)
             let options = if male_options.is_empty() { &voice_options } else { &male_options };

             let selection = Select::new("Select Default Male Voice:", options.clone())
                .prompt()?;
             config.audio.default_male_voice = Some(find_short_name(&selection));
             needs_save = true;
        }

        if config.audio.default_female_voice.is_none() {
             let female_options: Vec<String> = filtered_voices.iter()
                .filter(|v| v.gender == "Female")
                .map(|v| format!("{} ({}) - {}", v.short_name, v.gender, v.friendly_name.as_deref().unwrap_or(&v.name)))
                .collect();
            
             let options = if female_options.is_empty() { &voice_options } else { &female_options };

             let selection = Select::new("Select Default Female Voice:", options.clone())
                .prompt()?;
             config.audio.default_female_voice = Some(find_short_name(&selection));
             needs_save = true;
        }
    }

    if needs_save {
        config.save()?;
        println!("Configuration saved.");
    }

    Ok(())
}
