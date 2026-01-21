use crate::config::Config;
use crate::state::CharacterMap;
use anyhow::{Result, Context};
use serde_json;

pub trait ScriptGenerator: Send + Sync {
    fn get_system_prompt(&self) -> String;
    fn generate_prompt(&self, text: &str, char_map: &CharacterMap) -> Result<String>;
    fn parse_response(&self, response: &str) -> Result<Vec<String>>;
}

pub struct SsmlScriptGenerator {
    config: Config,
}

impl SsmlScriptGenerator {
    pub fn new(config: &Config) -> Self {
        Self { config: config.clone() }
    }
}

impl ScriptGenerator for SsmlScriptGenerator {
    fn get_system_prompt(&self) -> String {
        "你是一個 SSML 生成器。請僅返回有效的 JSON。".to_string()
    }

    fn generate_prompt(&self, text: &str, char_map: &CharacterMap) -> Result<String> {
        let characters_json = serde_json::to_string(&char_map.characters)?;
        let ssml_prompt = format!(
            "請將以下小說文本轉換為 Edge TTS 的 SSML。\
            使用提供的角色映射進行語音分配。\
            對於有 'voice_id' 的角色，請使用該語音。\
            對於其他人，請使用性別來選擇通用語調（但在此處不選擇語音名稱，僅在需要時標記角色/性別，或者僅輸出文本片段）。\
            \n\n\
            實際上，為了簡單起見：\
            輸出一個字符串的 JSON 列表。每個字符串都是一個有效的 SSML <speak> 塊。\
            將文本分解為邏輯段落（段落或對話）。\
            使用 <voice name='...'> 標籤。\
            \n\
            配置：\
            默認男性語音：'{}'\n\
            默認女性語音：'{}'\n\
            旁白語音：'{}'\n\
            \n\
            角色映射：{}\n\
            \n\
            規則：\
            1. 每個段落都使用 <voice name='...'>。\n\
            2. 對於旁白，使用旁白語音。\n\
            3. 對於對話，檢查說話者。如果在角色映射中且有 voice_id，則使用它。\n\
               如果沒有 voice_id，根據性別使用默認男性/女性語音。\n\
            4. 如果上下文建議，調整 <prosody> 以表達情感。\n\
            5. 僅返回 JSON：[ \"<speak>...</speak>\", ... ] \
            \n\n文本：\n{}",
            self.config.audio.default_male_voice.as_deref().unwrap_or(""),
            self.config.audio.default_female_voice.as_deref().unwrap_or(""),
            self.config.audio.narrator_voice.as_deref().unwrap_or(""),
            characters_json,
            text
        );
        Ok(ssml_prompt)
    }

    fn parse_response(&self, response: &str) -> Result<Vec<String>> {
        let clean_json = strip_code_blocks(response);
        let ssml_segments: Vec<String> = serde_json::from_str(&clean_json)
            .context(format!("Failed to parse SSML JSON: {}", clean_json))?;
        Ok(ssml_segments)
    }
}

pub struct PlainScriptGenerator;

impl PlainScriptGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl ScriptGenerator for PlainScriptGenerator {
    fn get_system_prompt(&self) -> String {
        "You are a story narrator.".to_string()
    }

    fn generate_prompt(&self, _text: &str, _char_map: &CharacterMap) -> Result<String> {
        // TODO: Implement prompt generation for plain text/independent channel
        // Placeholder for future implementation
        Ok("TODO: Implement prompt".to_string())
    }

    fn parse_response(&self, _response: &str) -> Result<Vec<String>> {
        // TODO: Implement parsing for plain text/independent channel
        // Placeholder for future implementation
        Ok(vec![])
    }
}

pub fn strip_code_blocks(s: &str) -> String {
    let s = s.trim();
    if s.starts_with("```json") {
        s.trim_start_matches("```json").trim_end_matches("```").trim().to_string()
    } else if s.starts_with("```") {
        s.trim_start_matches("```").trim_end_matches("```").trim().to_string()
    } else {
        s.to_string()
    }
}
