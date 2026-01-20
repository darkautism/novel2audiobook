use crate::config::Config;
use crate::llm::LlmClient;
use crate::tts::TtsClient;
use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use tokio::fs as tokio_fs;
use std::path::Path;
use std::io::Write;

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct WorkflowState {
    pub completed_chapters: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CharacterMap {
    pub characters: HashMap<String, CharacterInfo>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CharacterInfo {
    pub gender: String, // "Male", "Female"
    pub voice_id: Option<String>,
    pub description: Option<String>, // Context for LLM
}

pub struct WorkflowManager {
    config: Config,
    llm: Box<dyn LlmClient>,
    state: WorkflowState,
    character_map: CharacterMap,
    tts: Box<dyn TtsClient>,
}

impl WorkflowManager {
    pub fn new(config: Config, llm: Box<dyn LlmClient>, tts: Box<dyn TtsClient>) -> Result<Self> {
        let state = Self::load_state(&config.build_folder)?;
        let character_map = Self::load_character_map(&config.build_folder)?;
        
        Ok(Self {
            config,
            llm,
            state,
            character_map,
            tts,
        })
    }

    fn load_state(build_dir: &str) -> Result<WorkflowState> {
        let path = Path::new(build_dir).join("state.json");
        if path.exists() {
            let content = fs::read_to_string(path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(WorkflowState::default())
        }
    }

    fn save_state(&self) -> Result<()> {
        let path = Path::new(&self.config.build_folder).join("state.json");
        let content = serde_json::to_string_pretty(&self.state)?;
        fs::write(path, content)?;
        Ok(())
    }

    fn load_character_map(build_dir: &str) -> Result<CharacterMap> {
        let path = Path::new(build_dir).join("character_map.json");
        if path.exists() {
            let content = fs::read_to_string(path)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(CharacterMap { characters: HashMap::new() })
        }
    }

    fn save_character_map(&self) -> Result<()> {
        let path = Path::new(&self.config.build_folder).join("character_map.json");
        let content = serde_json::to_string_pretty(&self.character_map)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub async fn run(&mut self) -> Result<()> {
        // List input files
        let input_path = Path::new(&self.config.input_folder);
        let mut entries = Vec::new();
        let mut dir = tokio_fs::read_dir(input_path).await?;
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "txt") {
                entries.push(path);
            }
        }
        
        entries.sort();

        for path in entries {
            let filename = path.file_name().unwrap().to_string_lossy().to_string();
            
            if self.state.completed_chapters.contains(&filename) {
                println!("Skipping completed chapter: {}", filename);
                continue;
            }

            println!("Processing chapter: {}", filename);
            self.process_chapter(&path, &filename).await?;
            
            self.state.completed_chapters.push(filename);
            self.save_state()?;
        }

        println!("All chapters processed!");
        Ok(())
    }

    async fn process_chapter(&mut self, path: &Path, filename: &str) -> Result<()> {
        let text = fs::read_to_string(path)?;
        
        let chapter_build_dir = Path::new(&self.config.build_folder).join(filename.replace(".", "_"));
        fs::create_dir_all(&chapter_build_dir)?;
        let ssml_path = chapter_build_dir.join("SSML.json");

        let ssml_segments: Vec<String> = if ssml_path.exists() {
            println!("Loading cached SSML from {:?}", ssml_path);
            let content = fs::read_to_string(&ssml_path)?;
            serde_json::from_str(&content)?
        } else {
            // 1. Analyze Characters
            println!("Analyzing characters...");
            let analysis_prompt = format!(
                "請分析以下文本。識別所有說話的角色。\
                確定他們的性別（Male/Female）以及是否為主要角色（important）。\
                僅返回一個 JSON 對象：\
                {{ \"characters\": [ {{ \"name\": \"...\", \"gender\": \"Male/Female\", \"important\": true/false, \"description\": \"...\" }} ] }} \
                \n\n文本：\n{}", 
                text.chars().take(10000).collect::<String>() // Limit context if needed, but ideally full chapter.
            );

            let mut analysis_json = self.llm.chat("你是一位文學助手。請僅返回有效的 JSON。", &analysis_prompt).await?;

            analysis_json = analysis_json.replace("\n", ""); // Clean newlines
            
            // Parse JSON
            #[derive(Deserialize)]
            struct AnalysisResult {
                characters: Vec<AnalysisChar>,
            }
            #[derive(Deserialize)]
            struct AnalysisChar {
                name: String,
                gender: String,
                #[serde(default)]
                important: bool,
                #[serde(default)]
                description: Option<String>,
            }
            
            // Clean markdown code blocks if present
            let clean_json = strip_code_blocks(&analysis_json);
            let analysis: AnalysisResult = serde_json::from_str(&clean_json)
                .context(format!("Failed to parse analysis JSON: {}", clean_json))?;

            // Update Character Map
            let mut updated_map = false;
            for char in analysis.characters {
                if !self.character_map.characters.contains_key(&char.name) {
                    self.character_map.characters.insert(char.name.clone(), CharacterInfo {
                        gender: char.gender,
                        voice_id: None, 
                        description: char.description,
                    });
                    updated_map = true;
                }
            }
            if updated_map {
                self.save_character_map()?;
            }

            // 2. SSML Generation
            println!("Generating SSML...");
            let characters_json = serde_json::to_string(&self.character_map.characters)?;
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

            let ssml_json = self.llm.chat("你是一個 SSML 生成器。請僅返回有效的 JSON。", &ssml_prompt).await?;
            let clean_ssml_json = strip_code_blocks(&ssml_json);
            
            let ssml_segments: Vec<String> = serde_json::from_str(&clean_ssml_json)
                .context(format!("Failed to parse SSML JSON: {}", clean_ssml_json))?;

            // Save SSML to cache
            fs::write(&ssml_path, serde_json::to_string_pretty(&ssml_segments)?)?;
            
            ssml_segments
        };

        // 3. Synthesize
        println!("Synthesizing audio ({} segments)...", ssml_segments.len());
        
        let mut audio_files = Vec::new();

        for (i, ssml) in ssml_segments.iter().enumerate() {
            let chunk_path = chapter_build_dir.join(format!("chunk_{:04}.mp3", i));
            if chunk_path.exists() {
                // simple resume within chapter
                audio_files.push(chunk_path);
                continue; 
            }

            println!("Synthesizing chunk {}/{}", i + 1, ssml_segments.len());
            // Retry logic?
            let audio_data = self.tts.synthesize(ssml).await?;
            fs::write(&chunk_path, audio_data)?;
            audio_files.push(chunk_path);
        }

        // 4. Merge
        println!("Merging audio...");
        // Requirement 3: "output folder(cfg), contents is chapter_*** of the folder"
        let output_chapter_dir = Path::new(&self.config.output_folder).join(filename.replace(".", "_"));
        fs::create_dir_all(&output_chapter_dir)?;
        let final_audio_path = output_chapter_dir.join("audio.mp3");

        let mut final_file = fs::File::create(&final_audio_path)?;
        for path in audio_files {
            let data = fs::read(path)?;
            final_file.write_all(&data)?;
        }

        println!("Chapter complete: {:?}", final_audio_path);
        Ok(())
    }
}

fn strip_code_blocks(s: &str) -> String {
    let s = s.trim();
    if s.starts_with("```json") {
        s.trim_start_matches("```json").trim_end_matches("```").trim().to_string()
    } else if s.starts_with("```") {
        s.trim_start_matches("```").trim_end_matches("```").trim().to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use async_trait::async_trait;
    use std::fs;
    use std::path::Path;

    #[test]
    fn test_strip_code_blocks() {
        assert_eq!(strip_code_blocks("json"), "json");
        assert_eq!(strip_code_blocks("```json\n{}\n```"), "{}");
        assert_eq!(strip_code_blocks("```\n{}\n```"), "{}");
        assert_eq!(strip_code_blocks("  ```json  \n  {}  \n  ```  "), "{}");
    }

    // Mock LLM Client
    struct MockLlmClient {
        call_count: Arc<Mutex<usize>>,
    }

    impl MockLlmClient {
        fn new() -> Self {
            Self {
                call_count: Arc::new(Mutex::new(0)),
            }
        }
    }

    #[async_trait]
    impl LlmClient for MockLlmClient {
        async fn chat(&self, _system: &str, user: &str) -> Result<String> {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
            
            if user.contains("請分析以下文本") {
                return Ok(r#"{"characters": [{"name": "Hero", "gender": "Male"}]}"#.to_string());
            } else if user.contains("請將以下小說文本轉換為 Edge TTS 的 SSML") {
                return Ok(r#"["<speak>Test audio</speak>"]"#.to_string());
            }
            
            Ok("{}".to_string())
        }
    }

    struct MockTtsClient {
        should_fail: bool,
    }

    #[async_trait]
    impl TtsClient for MockTtsClient {
        async fn list_voices(&self) -> Result<Vec<crate::tts::Voice>> {
            Ok(vec![])
        }
        async fn synthesize(&self, _ssml: &str) -> Result<Vec<u8>> {
            if self.should_fail {
                Err(anyhow::anyhow!("Mock TTS error"))
            } else {
                Ok(vec![0u8; 10])
            }
        }
    }

    #[tokio::test]
    async fn test_cache_miss_generates_ssml_file() -> Result<()> {
        let test_root = Path::new("test_output_miss");
        if test_root.exists() { fs::remove_dir_all(test_root)?; }
        
        let build_dir = test_root.join("build");
        let input_dir = test_root.join("input");
        let output_dir = test_root.join("output");
        
        fs::create_dir_all(&build_dir)?;
        fs::create_dir_all(&input_dir)?;
        fs::create_dir_all(&output_dir)?;

        let config = Config {
            input_folder: input_dir.to_string_lossy().to_string(),
            output_folder: output_dir.to_string_lossy().to_string(),
            build_folder: build_dir.to_string_lossy().to_string(),
            llm: crate::config::LlmConfig {
                provider: "mock".to_string(),
                gemini: None,
                ollama: None,
                openai: None,
            },
            audio: crate::config::AudioConfig::default(),
        };

        let filename = "chapter_1.txt";
        let chapter_path = input_dir.join(filename);
        fs::write(&chapter_path, "Some story text.")?;

        let mock_llm = Box::new(MockLlmClient::new());
        let call_count = mock_llm.call_count.clone();
        
        let mock_tts = Box::new(MockTtsClient { should_fail: true });

        let mut workflow = WorkflowManager::new(config.clone(), mock_llm, mock_tts)?;

        // Run process_chapter
        // We expect it to fail at synthesis step due to network, but generate SSML before that.
        let result = workflow.process_chapter(&chapter_path, filename).await;
        
        // Assertions
        // Expect error due to synthesis network fail (mock configured to fail)
        assert!(result.is_err(), "Expected synthesis failure due to mock error");
        
        // Check LLM calls
        assert_eq!(*call_count.lock().unwrap(), 2, "Should call LLM twice (Analysis + SSML)");

        // Check SSML file existence
        let ssml_path = build_dir.join("chapter_1_txt").join("SSML.json");
        assert!(ssml_path.exists(), "SSML.json should be created");
        
        let content = fs::read_to_string(ssml_path)?;
        assert!(content.contains("<speak>Test audio</speak>"));
        
        // Cleanup
        let _ = fs::remove_dir_all(test_root);
        Ok(())
    }

    #[tokio::test]
    async fn test_cache_hit_skips_llm() -> Result<()> {
        let test_root = Path::new("test_output_hit");
        if test_root.exists() { fs::remove_dir_all(test_root)?; }
        
        let build_dir = test_root.join("build");
        let input_dir = test_root.join("input");
        let output_dir = test_root.join("output");
        
        fs::create_dir_all(&build_dir)?;
        fs::create_dir_all(&input_dir)?;
        fs::create_dir_all(&output_dir)?;

        let config = Config {
            input_folder: input_dir.to_string_lossy().to_string(),
            output_folder: output_dir.to_string_lossy().to_string(),
            build_folder: build_dir.to_string_lossy().to_string(),
            llm: crate::config::LlmConfig {
                provider: "mock".to_string(),
                gemini: None,
                ollama: None,
                openai: None,
            },
            audio: crate::config::AudioConfig::default(),
        };

        let filename = "chapter_2.txt";
        let chapter_path = input_dir.join(filename);
        fs::write(&chapter_path, "Some story text.")?;

        // Pre-populate SSML cache
        let chapter_build_dir = build_dir.join("chapter_2_txt");
        fs::create_dir_all(&chapter_build_dir)?;
        let ssml_path = chapter_build_dir.join("SSML.json");
        let cached_ssml = vec!["<speak>Cached audio</speak>".to_string()];
        fs::write(&ssml_path, serde_json::to_string(&cached_ssml)?)?;

        // Pre-populate audio chunk to skip synthesis
        let chunk_path = chapter_build_dir.join("chunk_0000.mp3");
        fs::write(&chunk_path, b"fake mp3 data")?;

        let mock_llm = Box::new(MockLlmClient::new());
        let call_count = mock_llm.call_count.clone();
        
        let mock_tts = Box::new(MockTtsClient { should_fail: false });

        let mut workflow = WorkflowManager::new(config.clone(), mock_llm, mock_tts)?;

        // Run process_chapter
        let result = workflow.process_chapter(&chapter_path, filename).await;
        
        assert!(result.is_ok(), "Should complete successfully (skipping synthesis via cache)");
        
        // Check LLM calls - Should be 0
        assert_eq!(*call_count.lock().unwrap(), 0, "Should use cache and NOT call LLM");

        // Cleanup
        let _ = fs::remove_dir_all(test_root);
        Ok(())
    }
}
