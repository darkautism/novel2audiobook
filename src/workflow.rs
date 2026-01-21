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
        let total_chapters = entries.len();

        for (i, path) in entries.iter().enumerate() {
            let filename = path.file_name().unwrap().to_string_lossy().to_string();
            
            if self.state.completed_chapters.contains(&filename) {
                println!("Skipping completed chapter: {}", filename);
                continue;
            }

            println!("Processing chapter: {}", filename);
            self.process_chapter(path, &filename).await?;
            
            self.state.completed_chapters.push(filename);
            self.save_state()?;

            if !self.config.unattended && i < total_chapters - 1 {
                let ans = inquire::Confirm::new("Continue to next chapter?")
                    .with_default(true)
                    .prompt();
                
                match ans {
                    Ok(true) => {},
                    Ok(false) => {
                        println!("Stopping as requested.");
                        break;
                    },
                    Err(_) => {
                        println!("Error reading input, stopping.");
                        break;
                    }
                }
            }
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

            let existing_chars_str = self.character_map.characters.keys()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ");
                
            let mut voices = self.tts.list_voices().await.unwrap_or_default();
            // Filter voices by language
            voices.retain(|v| {
                v.locale.starts_with(&self.config.audio.language) && 
                !self.config.audio.exclude_locales.contains(&v.locale)
            });
            
            let voice_list_str = voices.iter()
                .map(|v| format!("{{ \"id\": \"{}\", \"gender\": \"{}\", \"locale\": \"{}\" }}", v.short_name, v.gender, v.locale))
                .collect::<Vec<_>>()
                .join("\n");

            let narrator_voice_id = match self.config.audio.provider.as_str() {
                "edge-tts" => self.config.audio.edge_tts.as_ref().and_then(|c| c.narrator_voice.clone()),
                "sovits-offline" => self.config.audio.sovits.as_ref().and_then(|c| c.narrator_voice.clone()),
                _ => None,
            }.unwrap_or_else(|| "zh-TW-HsiaoChenNeural".to_string());

            let analysis_prompt = format!(
                "請分析以下文本。識別所有說話的角色。\
                \n\n上下文資訊 (Context):\
                \n1. 目前已存在的角色 (Existing Characters): [{}]\
                \n2. 旁白聲音 ID (Narrator Voice ID): \"{}\"\
                \n3. 可用聲音列表 (Available Voices):\n[{}]\
                \n\n指令 (Instructions):\
                \n- 識別文本中的說話角色，確定性別（Male/Female）及是否為主要角色。\
                \n- 若角色已存在於「目前已存在的角色」中，請使用相同的名稱。\
                \n- 若文本為第一人稱（如「我」），請識別主角，並將其 voice_id 設定為旁白聲音 ID。\
                \n- 主要角色，尤其主角，請避免重複使用該聲音。旁白亦同。\
                \n- 對於新角色，你可以從「可用聲音列表」中選擇合適的 voice_id (選填)，否則留空。\
                \n- 系統已內建路人、路人(男)、路人(女)三個角色，請勿重複創建。\
                \n- 不重要的丟棄式角色請直接使用路人。\
                \n- 創建的JSON對象由於是key必須使用繁體中文。使用簡體將導致程式出錯。\
                \n\n請僅返回一個 JSON 對象：\
                {{ \"characters\": [ {{ \"name\": \"...\", \"gender\": \"Male/Female\", \"important\": true/false, \"description\": \"...\", \"voice_id\": \"...\" }} ] }} \
                \n\n文本：\n{}", 
                existing_chars_str,
                narrator_voice_id,
                voice_list_str,
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
                #[serde(default)]
                voice_id: Option<String>,
            }
            
            // Clean markdown code blocks if present
            let clean_json = strip_code_blocks(&analysis_json);
            let analysis: AnalysisResult = serde_json::from_str(&clean_json)
                .context(format!("Failed to parse analysis JSON: {}", clean_json))?;

            // Update Character Map
            let mut updated_map = false;
            for char in analysis.characters {
                let entry = self.character_map.characters.entry(char.name.clone());
                match entry {
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(CharacterInfo {
                            gender: char.gender,
                            voice_id: char.voice_id, 
                            description: char.description,
                        });
                        updated_map = true;
                    },
                    std::collections::hash_map::Entry::Occupied(mut e) => {
                        if e.get().voice_id.is_none() && char.voice_id.is_some() {
                             e.get_mut().voice_id = char.voice_id;
                             updated_map = true;
                        }
                    }
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
        let output_filename = Path::new(filename).with_extension("mp3").file_name().unwrap().to_string_lossy().to_string();
        let final_audio_path = Path::new(&self.config.output_folder).join(output_filename);

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
            unattended: false,
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
    async fn test_flattened_output_structure() -> Result<()> {
        let test_root = Path::new("test_output_flattened");
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
            unattended: false,
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

        let filename = "chapter_flat.txt";
        let chapter_path = input_dir.join(filename);
        fs::write(&chapter_path, "Text")?;

        // Pre-populate segments to skip LLM
        let chapter_build_dir = build_dir.join("chapter_flat_txt");
        fs::create_dir_all(&chapter_build_dir)?;
        let segments_path = chapter_build_dir.join("segments.json");
        let cached_segments = vec![AudioSegment {
            speaker: "Narrator".to_string(),
            text: "Audio".to_string(),
            style: None,
        }];
        fs::write(&segments_path, serde_json::to_string(&cached_segments)?)?;
        
        let mock_llm = Box::new(MockLlmClient::new());
        let mock_tts = Box::new(MockTtsClient { should_fail: false });

        let mut workflow = WorkflowManager::new(config, mock_llm, mock_tts)?;
        workflow.process_chapter(&chapter_path, filename).await?;

        // Check output
        let output_file = output_dir.join("chapter_flat.mp3");
        assert!(output_file.exists(), "Output file should exist at root of output folder");
        
        let sub_dir = output_dir.join("chapter_flat_txt");
        assert!(!sub_dir.exists(), "Subdirectory should NOT exist in output folder");

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
            unattended: false,
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

    #[tokio::test]
    async fn test_voice_filtering_in_analysis_prompt() -> Result<()> {
        let test_root = Path::new("test_output_filter");
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
            unattended: false,
            llm: crate::config::LlmConfig {
                provider: "mock".to_string(),
                gemini: None,
                ollama: None,
                openai: None,
            },
            audio: crate::config::AudioConfig {
                provider: "edge-tts".to_string(),
                language: "zh".to_string(),
                exclude_locales: vec!["zh-HK".to_string()],
                ..crate::config::AudioConfig::default()
            },
        };

        let filename = "chapter_filter.txt";
        let chapter_path = input_dir.join(filename);
        fs::write(&chapter_path, "Text")?;

        // Setup Mock LLM to capture prompt
        struct CapturingLlmClient {
            prompts: Arc<Mutex<Vec<String>>>,
        }
        #[async_trait]
        impl LlmClient for CapturingLlmClient {
            async fn chat(&self, _system: &str, user: &str) -> Result<String> {
                self.prompts.lock().unwrap().push(user.to_string());
                // Return valid JSON to proceed
                Ok(r#"{"characters": []}"#.to_string())
            }
        }
        let prompts_store = Arc::new(Mutex::new(Vec::new()));
        let mock_llm = Box::new(CapturingLlmClient { prompts: prompts_store.clone() });

        // Setup Mock TTS with voices
        struct MockTts { voices: Vec<crate::tts::Voice> }
        #[async_trait]
        impl TtsClient for MockTts {
            async fn list_voices(&self) -> Result<Vec<crate::tts::Voice>> { Ok(self.voices.clone()) }
            async fn synthesize(&self, _: &AudioSegment, _: &CharacterMap) -> Result<Vec<u8>> { Ok(vec![]) }
        }
        
        let voices = vec![
            crate::tts::Voice { short_name: "zh-TW-A".to_string(), gender: "Male".to_string(), locale: "zh-TW".to_string(), name: "A".to_string(), friendly_name: None },
            crate::tts::Voice { short_name: "zh-HK-B".to_string(), gender: "Female".to_string(), locale: "zh-HK".to_string(), name: "B".to_string(), friendly_name: None },
            crate::tts::Voice { short_name: "zh-CN-C".to_string(), gender: "Male".to_string(), locale: "zh-CN".to_string(), name: "C".to_string(), friendly_name: None },
        ];
        let mock_tts = Box::new(MockTts { voices });

        let mut workflow = WorkflowManager::new(config, mock_llm, mock_tts)?;
        let _ = workflow.process_chapter(&chapter_path, filename).await;

        let prompts = prompts_store.lock().unwrap();
        let analysis_prompt = &prompts[0];
        
        // Assertions
        assert!(analysis_prompt.contains("zh-TW-A"));
        assert!(analysis_prompt.contains("zh-CN-C"));
        assert!(!analysis_prompt.contains("zh-HK-B"), "Excluded locale voice should not be in prompt");

        let _ = fs::remove_dir_all(test_root);
        Ok(())
    }
}
