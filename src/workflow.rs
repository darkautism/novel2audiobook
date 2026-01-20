use crate::config::Config;
use crate::llm::LlmClient;
use crate::tts::EdgeTtsClient;
use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
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
}

impl WorkflowManager {
    pub fn new(config: Config, llm: Box<dyn LlmClient>) -> Result<Self> {
        let state = Self::load_state(&config.build_folder)?;
        let character_map = Self::load_character_map(&config.build_folder)?;
        
        Ok(Self {
            config,
            llm,
            state,
            character_map,
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
        let mut entries: Vec<PathBuf> = fs::read_dir(input_path)?
            .filter_map(|res| res.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |ext| ext == "txt"))
            .collect();
        
        entries.sort();

        for path in entries {
            let filename = path.file_name().unwrap().to_string_lossy().to_string();
            
            if self.state.completed_chapters.contains(&filename) {
                println!("Skipping completed chapter: {}", filename);
                continue;
            }

            println!("Processing chapter: {}", filename);
            self.process_chapter(&path, &filename).await?;
            
            self.state.completed_chapters.push(filename.clone());
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
                "Analyze the following text. Identify all speaking characters. \
                Determine their gender (Male/Female) and if they are a main character (important). \
                Return ONLY a JSON object: \
                {{ \"characters\": [ {{ \"name\": \"...\", \"gender\": \"Male/Female\", \"important\": true/false, \"description\": \"...\" }} ] }} \
                \n\nText:\n{}", 
                text.chars().take(10000).collect::<String>() // Limit context if needed, but ideally full chapter.
            );
            // Truncate text for analysis if too long? 
            // With Gemini 1.5 Flash we have 1M context, so sending full text is fine usually.
            // Assuming reasonably sized chapters.

            let mut analysis_json = self.llm.chat("You are a literary assistant. Return valid JSON only.", &analysis_prompt).await?;

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
                    // If important, maybe we should assign a specific voice ID?
                    // For now, the prompt says "Unimportant roles can be unbound, just gender".
                    // "Important characters bind audio ID".
                    // But where do we get the ID? We only have a list of *available* voices.
                    // Should we auto-assign from the list?
                    // Or does the user *manually* bind?
                    // "2. Record character list... merge with previous... 9. Return to narrator section... extract list and let user choose?"
                    // Wait, item 9: "Back to narrator section... let user choose... write to config" -> that was initialization.
                    // Item 2: "Map names to gender/audio ID. Unimportant -> just gender".
                    // This implies *auto-assignment* or just persistent tracking.
                    // I'll leave `voice_id` None for now unless we implement an auto-assigner.
                    // Actually, if `voice_id` is None, the generation step will pick a default Male/Female voice.
                    // If we want consistent voices for main characters, we should probably pick one from the available list and save it.
                    // For simplicity/MVP: I will just store gender. The `voice_id` will be None.
                    // During generation, if `voice_id` is None, use `default_male` or `default_female`.
                    
                    // OPTIONAL: Auto-assign random consistent voice for "Important" characters from unused voices?
                    // Let's stick to defaults for now to be safe, unless user requirement "3. Audio dictionary... merged" implies complex management.
                    
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
                "Convert the following novel text into SSML for Edge TTS. \
                Use the provided Character Map for voice assignment. \
                For characters with 'voice_id', use that voice. \
                For others, use Gender to pick a generic tone (but you don't pick the voice name here, just mark the role/gender if needed, OR just output text segments). \
                \n\n\
                Actually, to make it easier: \
                Output a JSON LIST of strings. Each string is a valid SSML <speak> block. \
                Break the text into logical segments (paragraphs or dialogues). \
                Use the <voice name='...'> tag. \
                \n\
                Configuration: \
                Default Male Voice: '{}' \
                Default Female Voice: '{}' \
                Narrator Voice: '{}' \
                \n\
                Character Map: {} \
                \n\
                Rules: \
                1. Use <voice name='...'> for every segment. \
                2. For narration, use Narrator Voice. \
                3. For dialogue, check the speaker. If in Character Map and has voice_id, use it. \
                   If no voice_id, use Default Male/Female based on gender. \
                4. Adjust <prosody> for emotion if context suggests. \
                5. Return ONLY JSON: [ \"<speak>...</speak>\", ... ] \
                \n\nText:\n{}",
                self.config.audio.default_male_voice.as_deref().unwrap_or(""),
                self.config.audio.default_female_voice.as_deref().unwrap_or(""),
                self.config.audio.narrator_voice.as_deref().unwrap_or(""),
                characters_json,
                text
            );

            let ssml_json = self.llm.chat("You are an SSML generator. Return valid JSON only.", &ssml_prompt).await?;
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
            let audio_data = EdgeTtsClient::synthesize(ssml).await?;
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
            
            if user.contains("Analyze the following text") {
                return Ok(r#"{"characters": [{"name": "Hero", "gender": "Male"}]}"#.to_string());
            } else if user.contains("Convert the following novel text into SSML") {
                return Ok(r#"["<speak>Test audio</speak>"]"#.to_string());
            }
            
            Ok("{}".to_string())
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
            },
            audio: crate::config::AudioConfig::default(),
        };

        let filename = "chapter_1.txt";
        let chapter_path = input_dir.join(filename);
        fs::write(&chapter_path, "Some story text.")?;

        let mock_llm = Box::new(MockLlmClient::new());
        let call_count = mock_llm.call_count.clone();

        let mut workflow = WorkflowManager::new(config.clone(), mock_llm)?;

        // Run process_chapter
        // We expect it to fail at synthesis step due to network, but generate SSML before that.
        let result = workflow.process_chapter(&chapter_path, filename).await;
        
        // Assertions
        // Expect error due to synthesis network fail, but SSML should be generated.
        assert!(result.is_err(), "Expected synthesis failure due to no network");
        
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

        let mut workflow = WorkflowManager::new(config.clone(), mock_llm)?;

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
