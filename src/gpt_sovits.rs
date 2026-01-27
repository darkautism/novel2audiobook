use crate::llm::LlmClient;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

// --- Config ---

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GptSovitsConfig {
    pub token: String,

    #[serde(default)]
    pub retry: i32,

    #[serde(default = "default_gpt_sovits_base_url")]
    pub base_url: String,

    #[serde(default = "default_gpt_sovits_top_k")]
    pub top_k: i32,
    #[serde(default = "default_gpt_sovits_top_p")]
    pub top_p: u8,
    #[serde(default = "default_gpt_sovits_temperature")]
    pub temperature: u8,
    #[serde(default = "default_gpt_sovits_speed_factor")]
    pub speed_factor: u8,
    #[serde(default = "default_gpt_sovits_repetition_penalty")]
    pub repetition_penalty: f64,

    pub narrator_voice: Option<String>,

    #[serde(default)]
    pub autofix: bool,
}

fn default_gpt_sovits_base_url() -> String {
    "https://gsv2p.acgnai.top/".to_string()
}

fn default_gpt_sovits_top_k() -> i32 {
    10
}
fn default_gpt_sovits_top_p() -> u8 {
    1
}
fn default_gpt_sovits_temperature() -> u8 {
    1
}
fn default_gpt_sovits_speed_factor() -> u8 {
    1
}
fn default_gpt_sovits_repetition_penalty() -> f64 {
    1.35
}

// --- Metadata ---

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GptSovitsVoiceMetadata {
    pub gender: String, // "Male", "Female", "Neutral"
    pub tags: Vec<String>,
    pub emotion: Vec<String>,
}

// Map from Model Name -> Metadata
pub type GptSovitsVoiceMap = HashMap<String, GptSovitsVoiceMetadata>;

// API Response Structs
#[derive(Debug, Deserialize)]
struct GptSovitsModelResponse {
    #[allow(dead_code)]
    msg: String,
    models: HashMap<String, HashMap<String, Vec<String>>>,
}

#[derive(Deserialize)]
struct LlmVoiceInfo {
    gender: String,
    tags: Vec<String>,
}

pub async fn load_or_refresh_metadata(
    config: &GptSovitsConfig,
    language: &str,
    llm: Option<&dyn LlmClient>,
) -> Result<GptSovitsVoiceMap> {
    let url = url::Url::parse(&config.base_url).unwrap();
    let filename = url.host_str().unwrap_or("").to_string() + ".json";
    let file_path = Path::new(&filename);
    let local_map: GptSovitsVoiceMap = if file_path.exists() {
        let content = fs::read_to_string(&file_path).await?;
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        // 沒有舊檔，製作新檔
        // 1. Fetch API
        let mut local_map: GptSovitsVoiceMap = HashMap::new();
        let client = reqwest::Client::new();
        let url = config.base_url.clone() + "models/v4";

        // If GPT-SoVITS is not configured or URL is empty, return empty map
        if url.is_empty() {
            return Ok(HashMap::new());
        }

        let token = config.token.clone();

        let mut req = client.get(&url);
        if !token.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", token));
        }

        let resp = req
            .send()
            .await
            .context("Failed to fetch GPT-SoVITS models")?;
        let api_data: GptSovitsModelResponse = resp
            .json()
            .await
            .context("Failed to parse GPT-SoVITS models JSON")?;

        let new_models = api_data.models.keys().cloned().collect::<Vec<String>>();
        if let Some(llm_client) = llm {
            println!("Classifying GPT-SoVITS voices via LLM...");
            // Process in chunks
            for chunk in new_models.chunks(500) {
                let prompt = format!(
                     "請分析以下角色名稱，並猜測他們的性別。並根據該作品分析角色的標籤，如年長、年幼、開朗、深沉之類的\n\
                     Names: {:?}\n\
                     所有的姓名將被作為key，所以請勿修改.\n\
                     我們服務的目標語言是 {}，故可以略過完全無關的語言(Key內通常帶有語言資訊)\n\
                     你應該回傳如下的JSON陣列 {{ \"gender\": \"Male\"/\"Female\", \"tags\": [\"Tag1\", \"Tag2\"] }}.\n\
                     For gender, use 'Male' or 'Female'. Defaults to 'Female' if unsure.\n\
                     Use Traditional Chinese for tags.\n\
                     Ensure the JSON is valid.",
                     chunk,
                     language,
                 );

                match llm_client
                    .chat("You are an ACG expert. Return valid JSON only.", &prompt)
                    .await
                {
                    Ok(response) => {
                        let clean_json = crate::script::strip_code_blocks(&response);
                        match serde_json::from_str::<HashMap<String, LlmVoiceInfo>>(&clean_json) {
                            Ok(parsed) => {
                                for (name, info) in parsed {
                                    // Verify name is in our list (LLM might hallucinate keys)
                                    if let Some(langs) = api_data.models.get(&name) {
                                        local_map.insert(
                                            name.clone(),
                                            GptSovitsVoiceMetadata {
                                                gender: info.gender,
                                                tags: info.tags,
                                                emotion: langs
                                                    .values()
                                                    .flatten()
                                                    .cloned()
                                                    .collect(),
                                            },
                                        );
                                    }
                                }
                            }
                            Err(e) => eprintln!("Failed to parse LLM JSON for voices: {}", e),
                        }
                    }
                    Err(e) => eprintln!("Failed to classify voices via LLM: {}", e),
                }
            }
        }

        // Fill remaining with defaults
        for name in new_models {
            if !local_map.contains_key(&name) {
                if let Some(langs) = api_data.models.get(&name) {
                    local_map.insert(
                        name.clone(),
                        GptSovitsVoiceMetadata {
                            gender: "Female".to_string(),
                            tags: vec![],
                            emotion: langs.values().flatten().cloned().collect(),
                        },
                    );
                }
            }
        }

        // Save
        let content = serde_json::to_string_pretty(&local_map)?;
        fs::write(&file_path, content).await?;
        println!("Updated {}", file_path.display());
        local_map
    };

    Ok(local_map)
}
