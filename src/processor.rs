use crate::config::Config;
use crate::llm::LlmClient;
use crate::script::{strip_code_blocks, AudioSegment, ScriptGenerator};
use crate::state::{CharacterInfo, CharacterMap};
use crate::tts::{TtsClient, Voice};
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;

pub struct Processor<'a> {
    config: &'a Config,
    llm: &'a dyn LlmClient,
    tts: &'a dyn TtsClient,
}

impl<'a> Processor<'a> {
    pub fn new(config: &'a Config, llm: &'a dyn LlmClient, tts: &'a dyn TtsClient) -> Self {
        Self { config, llm, tts }
    }

    pub async fn analyze_characters(
        &self,
        text: &str,
        character_map: &CharacterMap,
        voices: &[Voice],
    ) -> Result<Vec<AnalysisChar>> {
        let existing_chars_str = character_map
            .characters
            .keys()
            .map(|k| k.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        let voice_list_str = self.tts.format_voice_list_for_analysis(voices);
        let narrator_voice_id = self.tts.get_narrator_voice_id();
        let enable_mobs = self.tts.is_mob_enabled();

        let mob_instruction = if enable_mobs {
            "- 系統已內建路人、路人(男)、路人(女)、章節路人(男)、章節路人(女)等角色，請勿重複創建。\n\
             - 章節內話多但後續不出現的角色，請使用「章節路人(男)」或「章節路人(女)」。\n\
             - 不重要的丟棄式角色請直接使用路人、路人(男)或路人(女)。"
        } else {
            "- 對於不重要的路人或龍套角色，無須分配，直接略過即可。"
        };

        let analysis_prompt = format!(
            "請分析以下文本。識別所有說話的角色。\
            \n\n上下文資訊 (Context):\
            \n1. 目前已存在的角色 (Existing Characters): [{}]\
            \n2. 旁白聲音 ID (Narrator Voice ID): \"{}\"\
            \n3. 可用聲音列表 (Available Voices):\n[{}]\
            \n\n指令 (Instructions):\
            \n- 識別文本中的說話角色，確定性別（Male/Female）及是否為主要角色。\
            \n- 若角色為「主角」(Protagonist)，請將 \"is_protagonist\" 欄位設為 true。\
            \n- 若角色已存在於「目前已存在的角色」中，請使用相同的名稱。\
            \n- 若文本為第一人稱（如「我」），請識別主角，將其 voice_id 設定為旁白聲音 ID，並設定 \"is_protagonist\": true。\
            \n- 主要角色，尤其主角，請避免重複使用該聲音。旁白亦同。\
            \n- 對於新角色，你可以從「可用聲音列表」中選擇合適的 voice_id (選填)，否則留空。\
            \n{}\n\
            \n- 創建的JSON對象由於是key必須使用繁體中文。使用簡體將導致程式出錯。\
            \n\n請僅返回一個 JSON 對象(不可翻譯json key)：\
            {{ \"characters\": [ {{ \"name\": \"...\", \"gender\": \"Male/Female\", \"is_protagonist\": true/false, \"important\": true/false, \"description\": \"...\", \"voice_id\": \"...\" }} ] }} \
            \n\n文本：\n{}", 
            existing_chars_str,
            narrator_voice_id,
            voice_list_str,
            mob_instruction,
            text.chars().take(10000).collect::<String>(),
        );

        let mut analysis_json = self
            .llm
            .chat("你是一位文學助手。請僅返回有效的 JSON。", &analysis_prompt)
            .await?;

        analysis_json = analysis_json.replace("\n", "");

        let clean_json = strip_code_blocks(&analysis_json);
        let result: AnalysisResult = serde_json::from_str(&clean_json)
            .context(format!("Failed to parse analysis JSON: {}", clean_json))?;

        Ok(result.characters)
    }

    pub async fn generate_script(
        &self,
        text: &str,
        character_map: &CharacterMap,
        voices: &[Voice],
    ) -> Result<Vec<AudioSegment>> {
        let script_generator = self.tts.get_script_generator();

        // Gather voice styles
        let mut voice_styles = HashMap::new();
        for info in character_map.characters.values() {
            if let Some(vid) = &info.voice_id {
                if let Ok(styles) = self.tts.get_voice_styles(vid).await {
                    voice_styles.insert(vid.clone(), styles);
                }
            }
        }

        // For GPT-SoVITS, populate styles for ALL available voices (candidates)
        if self.config.audio.provider == "gpt_sovits" {
            for v in voices {
                if !voice_styles.contains_key(&v.short_name) {
                    if let Ok(styles) = self.tts.get_voice_styles(&v.short_name).await {
                        voice_styles.insert(v.short_name.clone(), styles);
                    }
                }
            }
        }

        let prompt = script_generator.generate_prompt(
            text,
            character_map,
            &voice_styles,
            voices,
        )?;
        let system_instruction = script_generator.get_system_prompt();

        let script_json = self.llm.chat(&system_instruction, &prompt).await?;
        script_generator.parse_response(&script_json)
    }
}

#[derive(Deserialize)]
struct AnalysisResult {
    characters: Vec<AnalysisChar>,
}

#[derive(Deserialize)]
pub struct AnalysisChar {
    pub name: String,
    pub gender: String,
    #[serde(default)]
    pub important: bool,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub voice_id: Option<String>,
    #[serde(default)]
    pub is_protagonist: bool,
}
