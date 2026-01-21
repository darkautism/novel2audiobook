use crate::config::Config;
use crate::state::CharacterMap;
use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use serde_json;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AudioSegment {
    pub text: String,
    pub speaker: String,
    #[serde(default)]
    pub style: Option<String>,
}

pub trait ScriptGenerator: Send + Sync {
    fn get_system_prompt(&self) -> String;
    fn generate_prompt(&self, text: &str, char_map: &CharacterMap) -> Result<String>;
    fn parse_response(&self, response: &str) -> Result<Vec<AudioSegment>>;
}

pub struct JsonScriptGenerator;

impl JsonScriptGenerator {
    pub fn new(_config: &Config) -> Self {
        Self
    }
}

impl ScriptGenerator for JsonScriptGenerator {
    fn get_system_prompt(&self) -> String {
        "你是一個有聲書腳本生成器。請將小說文本轉換為結構化的音頻腳本 JSON。".to_string()
    }

    fn generate_prompt(&self, text: &str, char_map: &CharacterMap) -> Result<String> {
        let characters_json = serde_json::to_string(&char_map.characters)?;
        let prompt = format!(
            "請將以下小說文本分解為對話和旁白段落。\
            根據提供的角色映射識別說話者。\
            \n\
            角色映射：{}\n\
            \n\
            輸出格式（JSON 列表）：\n\
            [\n\
              {{ \"speaker\": \"角色名或'旁白'\", \"text\": \"文本內容\", \"style\": \"情感/語氣(可選)\" }},\n\
              ...\n\
            ]\n\
            \n\
            規則：\n\
            1. 每個段落都必須是一個對象。\n\
            2. 如果是旁白，speaker 填寫 '旁白'。旁白應要根據前後文有語氣抑揚頓挫，避免死念書。\n\
            3. 如果是對話，speaker 填寫角色名稱。\n\
            4. 保持文本完整，不要遺漏。\n\
            5. 對於不重要的路人角色，請根據性別使用 '路人(男)', '路人(女)' 或 '路人' 作為 speaker。\n\
            \n\n文本：\n{}",
            characters_json,
            text
        );
        Ok(prompt)
    }

    fn parse_response(&self, response: &str) -> Result<Vec<AudioSegment>> {
        let clean_json = strip_code_blocks(response);
        let segments: Vec<AudioSegment> = serde_json::from_str(&clean_json)
            .context(format!("Failed to parse Script JSON: {}", clean_json))?;
        Ok(segments)
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
        Ok("TODO: Implement prompt".to_string())
    }

    fn parse_response(&self, _response: &str) -> Result<Vec<AudioSegment>> {
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
