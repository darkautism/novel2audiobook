use crate::config::Config;
use crate::llm::LlmClient;
use crate::tts::{TtsClient, VOICE_ID_MOB_MALE, VOICE_ID_MOB_FEMALE, VOICE_ID_MOB_NEUTRAL};
use crate::state::{WorkflowState, CharacterMap, CharacterInfo};
use crate::script::{ScriptGenerator, JsonScriptGenerator, PlainScriptGenerator, strip_code_blocks, AudioSegment};
use anyhow::{Result, Context};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use tokio::fs as tokio_fs;
use std::path::Path;
use std::io::Write;

pub struct WorkflowManager {
    config: Config,
    llm: Box<dyn LlmClient>,
    state: WorkflowState,
    character_map: CharacterMap,
    tts: Box<dyn TtsClient>,
    script_generator: Box<dyn ScriptGenerator>,
}

impl WorkflowManager {
    pub fn new(config: Config, llm: Box<dyn LlmClient>, tts: Box<dyn TtsClient>) -> Result<Self> {
        let state = Self::load_state(&config.build_folder)?;
        let mut character_map = Self::load_character_map(&config.build_folder)?;

        // Ensure mob characters exist
        let mut map_updated = false;
        
        if !character_map.characters.contains_key("路人(男)") {
            character_map.characters.insert("路人(男)".to_string(), CharacterInfo {
                gender: "Male".to_string(),
                voice_id: Some(VOICE_ID_MOB_MALE.to_string()),
                description: Some("一般男性路人".to_string()),
            });
            map_updated = true;
        }
        if !character_map.characters.contains_key("路人(女)") {
            character_map.characters.insert("路人(女)".to_string(), CharacterInfo {
                gender: "Female".to_string(),
                voice_id: Some(VOICE_ID_MOB_FEMALE.to_string()),
                description: Some("一般女性路人".to_string()),
            });
            map_updated = true;
        }
        if !character_map.characters.contains_key("路人") {
            character_map.characters.insert("路人".to_string(), CharacterInfo {
                gender: "Neutral".to_string(),
                voice_id: Some(VOICE_ID_MOB_NEUTRAL.to_string()),
                description: Some("一般路人".to_string()),
            });
            map_updated = true;
        }

        if map_updated {
            let path = Path::new(&config.build_folder).join("character_map.json");
            // Ensure build dir exists (it might not if it's the first run)
            fs::create_dir_all(&config.build_folder)?;
            let content = serde_json::to_string_pretty(&character_map)?;
            fs::write(path, content)?;
        }
        
        let script_generator: Box<dyn ScriptGenerator> = match config.audio.provider.as_str() {
            "edge-tts" | "sovits-offline" => Box::new(JsonScriptGenerator::new(&config)),
            _ => Box::new(PlainScriptGenerator::new()),
        };

        Ok(Self {
            config,
            llm,
            state,
            character_map,
            tts,
            script_generator,
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
        let segments_path = chapter_build_dir.join("segments.json");

        let segments: Vec<AudioSegment> = if segments_path.exists() {
            println!("Loading cached segments from {:?}", segments_path);
            let content = fs::read_to_string(&segments_path)?;
            serde_json::from_str(&content)?
        } else {
            // 1. Analyze Characters
            println!("Analyzing characters...");
            let analysis_prompt = format!(
                "請分析以下文本。識別所有說話的角色。\
                確定他們的性別（Male/Female）以及是否為主要角色（important）。\
                系統已內建路人、路人(男)、路人(女)三個角色，請勿重複創建。\
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
                _important: bool,
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

            // 2. Script Generation
            println!("Generating Script...");
            
            let prompt = self.script_generator.generate_prompt(&text, &self.character_map)?;
            let system_instruction = self.script_generator.get_system_prompt();
            
            let script_json = self.llm.chat(&system_instruction, &prompt).await?;
            let segments = self.script_generator.parse_response(&script_json)?;

            // Save Script to cache
            fs::write(&segments_path, serde_json::to_string_pretty(&segments)?)?;
            
            segments
        };

        // 3. Synthesize
        println!("Synthesizing audio ({} segments)...", segments.len());
        
        let mut audio_files = Vec::new();

        for (i, segment) in segments.iter().enumerate() {
            let chunk_path = chapter_build_dir.join(format!("chunk_{:04}.mp3", i));
            if chunk_path.exists() {
                // simple resume within chapter
                audio_files.push(chunk_path);
                continue; 
            }

            println!("Synthesizing chunk {}/{}", i + 1, segments.len());
            // Retry logic?
            let audio_data = self.tts.synthesize(segment, &self.character_map).await?;
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
            } else if user.contains("請將以下小說文本分解為對話和旁白段落") {
                return Ok(r#"[{"speaker": "旁白", "text": "Test audio"}]"#.to_string());
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
        async fn synthesize(&self, _segment: &AudioSegment, _map: &CharacterMap) -> Result<Vec<u8>> {
            if self.should_fail {
                Err(anyhow::anyhow!("Mock TTS error"))
            } else {
                Ok(vec![0u8; 10])
            }
        }
    }

    #[tokio::test]
    async fn test_cache_miss_generates_segments_file() -> Result<()> {
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
            audio: crate::config::AudioConfig {
                provider: "edge-tts".to_string(),
                ..crate::config::AudioConfig::default()
            },
        };

        let filename = "chapter_1.txt";
        let chapter_path = input_dir.join(filename);
        fs::write(&chapter_path, "Some story text.")?;

        let mock_llm = Box::new(MockLlmClient::new());
        let call_count = mock_llm.call_count.clone();
        
        let mock_tts = Box::new(MockTtsClient { should_fail: true });

        let mut workflow = WorkflowManager::new(config.clone(), mock_llm, mock_tts)?;

        let result = workflow.process_chapter(&chapter_path, filename).await;
        
        assert!(result.is_err(), "Expected synthesis failure due to mock error");
        
        assert_eq!(*call_count.lock().unwrap(), 2, "Should call LLM twice (Analysis + Script)");

        let segments_path = build_dir.join("chapter_1_txt").join("segments.json");
        assert!(segments_path.exists(), "segments.json should be created");
        
        let content = fs::read_to_string(segments_path)?;
        assert!(content.contains("Test audio"));
        
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
            audio: crate::config::AudioConfig {
                provider: "edge-tts".to_string(),
                ..crate::config::AudioConfig::default()
            },
        };

        let filename = "chapter_2.txt";
        let chapter_path = input_dir.join(filename);
        fs::write(&chapter_path, "Some story text.")?;

        let chapter_build_dir = build_dir.join("chapter_2_txt");
        fs::create_dir_all(&chapter_build_dir)?;
        let segments_path = chapter_build_dir.join("segments.json");
        
        let cached_segments = vec![AudioSegment {
            speaker: "Narrator".to_string(),
            text: "Cached audio".to_string(),
            style: None,
        }];
        fs::write(&segments_path, serde_json::to_string(&cached_segments)?)?;

        let chunk_path = chapter_build_dir.join("chunk_0000.mp3");
        fs::write(&chunk_path, b"fake mp3 data")?;

        let mock_llm = Box::new(MockLlmClient::new());
        let call_count = mock_llm.call_count.clone();
        
        let mock_tts = Box::new(MockTtsClient { should_fail: false });

        let mut workflow = WorkflowManager::new(config.clone(), mock_llm, mock_tts)?;

        let result = workflow.process_chapter(&chapter_path, filename).await;
        
        assert!(result.is_ok(), "Should complete successfully");
        
        assert_eq!(*call_count.lock().unwrap(), 0, "Should use cache and NOT call LLM");

        let _ = fs::remove_dir_all(test_root);
        Ok(())
    }
}
