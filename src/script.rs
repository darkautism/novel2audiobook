use crate::config::Config;
use crate::state::CharacterMap;
use crate::tts::Voice;
use anyhow::{Context, Ok, Result};
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AudioSegment {
    pub text: String,
    pub speaker: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub voice_id: Option<String>,
}

pub trait ScriptGenerator: Send + Sync {
    fn get_system_prompt(&self) -> String;
    fn generate_prompt(
        &self,
        text: &str,
        char_map: &CharacterMap,
        voice_styles: &HashMap<String, Vec<String>>,
        available_voices: &[Voice],
    ) -> Result<String>;
    fn parse_response(&self, response: &str) -> Result<Vec<AudioSegment>>;
    fn support_style(&self) -> Vec<String>;
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

    fn generate_prompt(
        &self,
        text: &str,
        char_map: &CharacterMap,
        voice_styles: &HashMap<String, Vec<String>>,
        _available_voices: &[Voice],
    ) -> Result<String> {
        let characters_json = serde_json::to_string(&char_map.characters)?;
        let global_styles = self.support_style();

        // Build style info string
        let mut specific_styles_str = String::new();
        for (name, info) in &char_map.characters {
            if let Some(vid) = &info.voice_id {
                if let Some(styles) = voice_styles.get(vid) {
                    if !styles.is_empty() {
                        use std::fmt::Write;
                        let _ = write!(
                            specific_styles_str,
                            "- {} ({}): [{}]\n",
                            name,
                            vid,
                            styles.join(", ")
                        );
                    }
                }
            }
        }

        let style_instruction = if !specific_styles_str.is_empty() {
            format!(
                "通用情緒：[{}]\n特別指定角色情緒（請優先使用）：\n{}",
                global_styles.join(", "),
                specific_styles_str
            )
        } else {
            format!("支援的情緒：{}", global_styles.join(", "))
        };

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
            {}\n\
            規則：\n\
            1. 每個段落都必須是一個對象。\n\
            2. 如果是旁白，speaker 填寫 '旁白'。旁白應要根據前後文有語氣抑揚頓挫，避免死念書。\n\
            3. 如果是對話，speaker 填寫角色名稱。\n\
            4. 保持文本完整，不要遺漏。\n\
            5. 對於不重要的路人角色，請根據性別使用 '路人(男)', '路人(女)' 或 '路人' 作為 speaker。\n\
            6. 若角色有特別指定情緒，請從該列表中選擇最合適的情緒。\n\
            \n\n文本：\n{}",
            characters_json,
            style_instruction,
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

    fn support_style(&self) -> Vec<String> {
        [
            "cheerful",
            "sad",
            "angry",
            "affectionate",
            "newscast",
            "assistant",
            "lyrical",
            "calm",
            "fearful",
            "whispering",
        ]
        .map(String::from)
        .to_vec()
    }
}

pub struct GptSovitsScriptGenerator {
    narrator_voice_id: String,
}

impl GptSovitsScriptGenerator {
    pub fn new(narrator_voice_id: String) -> Self {
        Self { narrator_voice_id }
    }
}

impl ScriptGenerator for GptSovitsScriptGenerator {
    fn get_system_prompt(&self) -> String {
        "你是一個有聲書腳本生成器。請將小說文本轉換為結構化的腳本 JSON。".to_string()
    }

    fn generate_prompt(
        &self,
        text: &str,
        char_map: &CharacterMap,
        voice_styles: &HashMap<String, Vec<String>>,
        available_voices: &[Voice],
    ) -> Result<String> {
        let characters_json = serde_json::to_string(&char_map.characters)?;

        let mut voices_str = String::new();
        // Limit available voices if too many? For now list all provided (caller should filter)
        for voice in available_voices {
            let styles = voice_styles
                .get(&voice.short_name)
                .map(|v| v.join(", "))
                .unwrap_or_default();
            let tags = voice.friendly_name.as_deref().unwrap_or("");
            use std::fmt::Write;
            let _ = write!(
                voices_str,
                "- ID: {}, Name: {}, Gender: {}, Styles: [{}], Info: {}\n",
                voice.short_name, voice.name, voice.gender, styles, tags
            );
        }

        let prompt = format!(
            "請將以下小說文本分解為對話和旁白段落。\
            根據提供的角色映射識別說話者。\
            \n\
            旁白ID：{}\n\
            角色映射：{}\n\
            \n\
            可用聲音列表（供未分配聲音的角色選用）：\n\
            {}\n\
            \n\
            輸出格式（JSON 列表）：\n\
            [\n\
              {{ \"speaker\": \"角色名\", \"text\": \"文本內容\", \"style\": \"情緒(可選)\", \"voice_id\": \"聲音ID(可選)\" }},\n\
              ...\n\
            ]\n\
            \n\
            規則：\n\
            1. 每個段落都必須是一個對象。\n\
            2. Speaker 和 voice_id 擇一填寫，未填者填入null。\n\
            3. 旁白使用上方提供的voice_id，其情緒可從可用聲音列表找到，可以從文本中提取出適合的情緒避免念稿。\n\
            4. 若角色無 voice_id（如路人），請從「可用聲音列表」中選擇合適的 voice_id 填入。\n\
            5. 指定 style，必須是該 voice_id 支援的情緒 (emotion)。\n\
            6. 重要：voice_id 和 style 的值必須嚴格對應列表中的 Key，**絕對禁止翻譯或修改**（例如 'happy' 不能寫成 '開心'）。\n\
            7. 保持文本完整，不要遺漏。\n\
            \n\n文本：\n{}",
            self.narrator_voice_id,
            characters_json,
            voices_str,
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

    fn support_style(&self) -> Vec<String> {
        vec![]
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

    fn generate_prompt(
        &self,
        _text: &str,
        _char_map: &CharacterMap,
        _styles: &HashMap<String, Vec<String>>,
        _available_voices: &[Voice],
    ) -> Result<String> {
        Ok("TODO: Implement prompt".to_string())
    }

    fn parse_response(&self, _response: &str) -> Result<Vec<AudioSegment>> {
        Ok(vec![])
    }

    fn support_style(&self) -> Vec<String> {
        todo!()
    }
}

pub fn strip_code_blocks(s: &str) -> String {
    let s = s.trim();
    if s.starts_with("```json") {
        s.trim_start_matches("```json")
            .trim_end_matches("```")
            .trim()
            .to_string()
    } else if s.starts_with("```") {
        s.trim_start_matches("```")
            .trim_end_matches("```")
            .trim()
            .to_string()
    } else {
        s.to_string()
    }
}
