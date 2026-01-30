use crate::core::config::Config;
use crate::core::state::{CharacterInfo, CharacterMap, WorkflowState};
use crate::services::llm::LlmClient;
use crate::services::script::{strip_code_blocks, AudioSegment, ScriptGenerator};
use crate::services::tts::{
    TtsClient, VOICE_ID_CHAPTER_MOB_FEMALE, VOICE_ID_CHAPTER_MOB_MALE, VOICE_ID_MOB_FEMALE,
    VOICE_ID_MOB_MALE, VOICE_ID_MOB_NEUTRAL,
};
use crate::core::io::Storage;
use anyhow::{Context, Result};
use serde::Deserialize;
use futures_util::StreamExt;
#[cfg(not(target_arch = "wasm32"))]
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
#[cfg(target_arch = "wasm32")]
use std::path::Path;
use std::sync::Arc;

pub struct WorkflowManager {
    config: Config,
    llm: Box<dyn LlmClient>,
    state: WorkflowState,
    character_map: CharacterMap,
    tts: Box<dyn TtsClient>,
    script_generator: Box<dyn ScriptGenerator>,
    storage: Arc<dyn Storage>,
}

impl WorkflowManager {
    pub async fn new(config: Config, llm: Box<dyn LlmClient>, tts: Box<dyn TtsClient>, storage: Arc<dyn Storage>) -> Result<Self> {
        let state = Self::load_state(&config.build_folder, storage.as_ref()).await?;
        let mut character_map = Self::load_character_map(&config.build_folder, storage.as_ref()).await?;

        let enable_mobs = tts.is_mob_enabled();

        // Ensure mob characters exist (Only if enabled)
        if enable_mobs {
            let mut map_updated = false;

            if !character_map.characters.contains_key("路人(男)") {
                character_map.characters.insert(
                    "路人(男)".to_string(),
                    CharacterInfo {
                        gender: "Male".to_string(),
                        voice_id: Some(VOICE_ID_MOB_MALE.to_string()),
                        description: Some("一般男性路人".to_string()),
                        ..Default::default()
                    },
                );
                map_updated = true;
            }
            if !character_map.characters.contains_key("章節路人(男)") {
                character_map.characters.insert(
                    "章節路人(男)".to_string(),
                    CharacterInfo {
                        gender: "Male".to_string(),
                        voice_id: Some(VOICE_ID_CHAPTER_MOB_MALE.to_string()),
                        description: Some("本章節內的男性路人，聲音在該章節內固定".to_string()),
                        ..Default::default()
                    },
                );
                map_updated = true;
            }
            if !character_map.characters.contains_key("章節路人(女)") {
                character_map.characters.insert(
                    "章節路人(女)".to_string(),
                    CharacterInfo {
                        gender: "Female".to_string(),
                        voice_id: Some(VOICE_ID_CHAPTER_MOB_FEMALE.to_string()),
                        description: Some("本章節內的女性路人，聲音在該章節內固定".to_string()),
                        ..Default::default()
                    },
                );
                map_updated = true;
            }
            if !character_map.characters.contains_key("路人(女)") {
                character_map.characters.insert(
                    "路人(女)".to_string(),
                    CharacterInfo {
                        gender: "Female".to_string(),
                        voice_id: Some(VOICE_ID_MOB_FEMALE.to_string()),
                        description: Some("一般女性路人".to_string()),
                        ..Default::default()
                    },
                );
                map_updated = true;
            }
            if !character_map.characters.contains_key("路人") {
                character_map.characters.insert(
                    "路人".to_string(),
                    CharacterInfo {
                        gender: "Neutral".to_string(),
                        voice_id: Some(VOICE_ID_MOB_NEUTRAL.to_string()),
                        description: Some("一般路人".to_string()),
                        ..Default::default()
                    },
                );
                map_updated = true;
            }

            if map_updated {
                let path = Path::new(&config.build_folder).join("character_map.json");
                let content = serde_json::to_string_pretty(&character_map)?;
                storage.write(path.to_str().unwrap(), content.as_bytes()).await?;
            }
        }

        let script_generator = tts.get_script_generator();

        Ok(Self {
            config,
            llm,
            state,
            character_map,
            tts,
            script_generator,
            storage,
        })
    }

    async fn load_state(build_dir: &str, storage: &dyn Storage) -> Result<WorkflowState> {
        let path = Path::new(build_dir).join("state.json");
        let path_str = path.to_str().unwrap();
        if storage.exists(path_str).await? {
            let bytes = storage.read(path_str).await?;
            let content = String::from_utf8(bytes)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(WorkflowState::default())
        }
    }

    async fn save_state(&self) -> Result<()> {
        let path = Path::new(&self.config.build_folder).join("state.json");
        let content = serde_json::to_string_pretty(&self.state)?;
        self.storage.write(path.to_str().unwrap(), content.as_bytes()).await?;
        Ok(())
    }

    async fn load_character_map(build_dir: &str, storage: &dyn Storage) -> Result<CharacterMap> {
        let path = Path::new(build_dir).join("character_map.json");
        let path_str = path.to_str().unwrap();
        if storage.exists(path_str).await? {
            let bytes = storage.read(path_str).await?;
            let content = String::from_utf8(bytes)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            Ok(CharacterMap {
                characters: HashMap::new(),
            })
        }
    }

    async fn save_character_map(&self) -> Result<()> {
        let path = Path::new(&self.config.build_folder).join("character_map.json");
        let content = serde_json::to_string_pretty(&self.character_map)?;
        self.storage.write(path.to_str().unwrap(), content.as_bytes()).await?;
        Ok(())
    }

    pub async fn run(&mut self) -> Result<()> {
        // List input files
        let entries = self.storage.list(&self.config.input_folder).await?;
        let mut txt_entries: Vec<String> = entries.into_iter()
            .filter(|e| e.ends_with(".txt"))
            .collect();

        txt_entries.sort();
        let total_chapters = txt_entries.len();

        for (i, path_str) in txt_entries.iter().enumerate() {
            let path = Path::new(path_str);
            let filename = path.file_name().unwrap().to_string_lossy().to_string();

            if self.state.completed_chapters.contains(&filename) {
                println!("Skipping completed chapter: {}", filename);
                continue;
            }

            println!("Processing chapter: {}", filename);
            self.process_chapter(path_str, &filename).await?;

            self.state.completed_chapters.push(filename);
            self.save_state().await?;

            if !self.config.unattended && i < total_chapters - 1 {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let ans = inquire::Confirm::new("Continue to next chapter?")
                        .with_default(true)
                        .prompt();

                    match ans {
                        Ok(true) => {}
                        Ok(false) => {
                            println!("Stopping as requested.");
                            break;
                        }
                        Err(_) => {
                            println!("Error reading input, stopping.");
                            break;
                        }
                    }
                }
            }
        }

        println!("All chapters processed!");
        Ok(())
    }

    async fn process_chapter(&mut self, path_str: &str, filename: &str) -> Result<()> {
        let bytes = self.storage.read(path_str).await?;
        let text = String::from_utf8(bytes)?;

        let chapter_build_dir =
            Path::new(&self.config.build_folder).join(filename.replace(".", "_"));
        
        let segments_path = chapter_build_dir.join("segments.json");
        let segments_path_str = segments_path.to_str().unwrap();

        // Prepare voices
        let mut voices = self.tts.list_voices().await?;
        voices.retain(|v| {
            v.locale.starts_with(&self.config.audio.language)
                && !self.config.audio.exclude_locales.contains(&v.locale)
        });

        let mut segments: Vec<AudioSegment> = if self.storage.exists(segments_path_str).await? {
            println!("Loading cached segments from {:?}", segments_path);
            let bytes = self.storage.read(segments_path_str).await?;
            let content = String::from_utf8(bytes)?;
            serde_json::from_str(&content)?
        } else {
            println!("Analyzing characters...");

            let existing_chars_str = self
                .character_map
                .characters
                .keys()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(", ");

            let voice_list_str = self.tts.format_voice_list_for_analysis(&voices);
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
                #[serde(default)]
                voice_id: Option<String>,
                #[serde(default)]
                is_protagonist: bool,
            }

            let clean_json = strip_code_blocks(&analysis_json);
            let analysis: AnalysisResult = serde_json::from_str(&clean_json)
                .context(format!("Failed to parse analysis JSON: {}", clean_json))?;

            let mut chapter_local_chars = HashMap::new();
            let mut updated_global_map = false;

            for char in analysis.characters {
                let should_persist = if enable_mobs {
                    true
                } else {
                    char.important || char.is_protagonist || char.voice_id.is_some()
                };

                if should_persist {
                    let entry = self.character_map.characters.entry(char.name.clone());
                    match entry {
                        std::collections::hash_map::Entry::Vacant(e) => {
                            e.insert(CharacterInfo {
                                gender: char.gender,
                                voice_id: char.voice_id,
                                description: char.description,
                                is_protagonist: char.is_protagonist,
                            });
                            updated_global_map = true;
                        }
                        std::collections::hash_map::Entry::Occupied(mut e) => {
                            if e.get().voice_id.is_none() && char.voice_id.is_some() {
                                e.get_mut().voice_id = char.voice_id;
                                updated_global_map = true;
                            }
                        }
                    }
                } else {
                    chapter_local_chars.insert(
                        char.name.clone(),
                        CharacterInfo {
                            gender: char.gender,
                            voice_id: char.voice_id,
                            description: char.description,
                            is_protagonist: char.is_protagonist,
                        },
                    );
                }
            }
            if updated_global_map {
                self.save_character_map().await?;
            }

            let mut combined_map = self.character_map.clone();
            for (k, v) in chapter_local_chars {
                combined_map.characters.insert(k, v);
            }

            println!("Generating Script...");

            let mut voice_styles = HashMap::new();
            for info in combined_map.characters.values() {
                if let Some(vid) = &info.voice_id {
                    if let Ok(styles) = self.tts.get_voice_styles(vid).await {
                        voice_styles.insert(vid.clone(), styles);
                    }
                }
            }
            if self.config.audio.provider == "gpt_sovits" {
                for v in &voices {
                    if !voice_styles.contains_key(&v.short_name) {
                        if let Ok(styles) = self.tts.get_voice_styles(&v.short_name).await {
                            voice_styles.insert(v.short_name.clone(), styles);
                        }
                    }
                }
            }

            let prompt = self.script_generator.generate_prompt(
                &text,
                &combined_map,
                &voice_styles,
                &voices,
            )?;
            let system_instruction = self.script_generator.get_system_prompt();

            let script_json = self.llm.chat(&system_instruction, &prompt).await?;
            let segments = self.script_generator.parse_response(&script_json)?;

            self.storage.write(segments_path_str, serde_json::to_string_pretty(&segments)?.as_bytes()).await?;

            segments
        };

        println!("Synthesizing audio ({} segments)...", segments.len());

        let mut excluded_voices = Vec::new();
        let narrator_voice_id = self.tts.get_narrator_voice_id();

        excluded_voices.push(narrator_voice_id);

        for char_info in self.character_map.characters.values() {
            if char_info.is_protagonist {
                if let Some(vid) = &char_info.voice_id {
                    if !excluded_voices.contains(vid) {
                        excluded_voices.push(vid.clone());
                    }
                }
            }
        }

        let mut working_map = self.character_map.clone();
        let enable_mobs = self.tts.is_mob_enabled();

        if enable_mobs {
            if let Ok(vid) = self
                .tts
                .get_random_voice(Some("Male"), &excluded_voices)
                .await
            {
                if let Some(info) = working_map.characters.get_mut("章節路人(男)") {
                    info.voice_id = Some(vid);
                }
            }

            if let Ok(vid) = self
                .tts
                .get_random_voice(Some("Female"), &excluded_voices)
                .await
            {
                if let Some(info) = working_map.characters.get_mut("章節路人(女)") {
                    info.voice_id = Some(vid);
                }
            }
        }

        let mut segments_mut = segments.clone();
        self.tts
            .check_and_fix_segments(
                &mut segments_mut,
                &working_map,
                &excluded_voices,
                self.llm.as_ref(),
            )
            .await?;

        segments = segments_mut;
        self.storage.write(segments_path_str, serde_json::to_string_pretty(&segments)?.as_bytes()).await?;

        #[cfg(not(target_arch = "wasm32"))]
        let pb = ProgressBar::new(segments.len() as u64);
        #[cfg(not(target_arch = "wasm32"))]
        pb.set_style(ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta})")?
            .progress_chars("#>-"));

        let tts = &self.tts;
        let working_map_ref = &working_map;
        let excluded_voices_ref = &excluded_voices;
        let storage = &self.storage;

        let max_concurrency = tts.max_concurrency();
        let results: Vec<Result<(usize, String)>> = futures_util::stream::iter(segments.iter().enumerate())
            .map(|(i, segment)| {
                let chunk_path = chapter_build_dir.join(format!("chunk_{:04}.mp3", i));
                let chunk_path_str = chunk_path.to_str().unwrap().to_string();
                #[cfg(not(target_arch = "wasm32"))]
                let pb = pb.clone();
                let storage = storage.clone();
                async move {
                    if !storage.exists(&chunk_path_str).await? {
                        let audio_data = tts.synthesize(segment, working_map_ref, excluded_voices_ref).await?;
                        storage.write(&chunk_path_str, &audio_data).await?;
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    pb.inc(1);
                    Ok((i, chunk_path_str))
                }
            })
            .buffer_unordered(max_concurrency)
            .collect()
            .await;

        #[cfg(not(target_arch = "wasm32"))]
        pb.finish_with_message("Synthesis complete");
        #[cfg(target_arch = "wasm32")]
        println!("Synthesis complete");

        let mut audio_files = vec![String::new(); segments.len()];
        for res in results {
            let (i, path) = res?;
            audio_files[i] = path;
        }

        println!("Merging audio...");
        let output_filename = Path::new(filename)
            .with_extension("mp3")
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let final_audio_path = Path::new(&self.config.output_folder).join(output_filename);
        let final_audio_path_str = final_audio_path.to_str().unwrap();

        self.tts
            .merge_audio_files(&audio_files, final_audio_path_str, self.storage.as_ref())
            .await?;
        
        // Cleanup logic
        println!("Cleaning up temporary chunks...");
        for chunk in audio_files {
             self.storage.delete(&chunk).await?;
        }

        println!("Chapter complete: {:?}", final_audio_path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::script::JsonScriptGenerator;
    use async_trait::async_trait;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use crate::core::io::NativeStorage;

    #[test]
    fn test_strip_code_blocks() {
        assert_eq!(strip_code_blocks("json"), "json");
        assert_eq!(strip_code_blocks("```json\n{}\n```"), "{}");
        assert_eq!(strip_code_blocks("```\n{}\n```"), "{}");
        assert_eq!(strip_code_blocks("  ```json  \n  {}  \n  ```  "), "{}");
    }

    // Mock LLM Client
    #[derive(Debug)]
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
        async fn list_voices(&self) -> Result<Vec<crate::services::tts::Voice>> {
            Ok(vec![])
        }
        async fn synthesize(
            &self,
            _segment: &AudioSegment,
            _map: &CharacterMap,
            _excluded_voices: &[String],
        ) -> Result<Vec<u8>> {
            if self.should_fail {
                Err(anyhow::anyhow!("Mock TTS error"))
            } else {
                Ok(vec![0u8; 10])
            }
        }
        async fn get_random_voice(
            &self,
            _gender: Option<&str>,
            _excluded_voices: &[String],
        ) -> Result<String> {
            Ok("mock_voice_id".to_string())
        }
        fn get_narrator_voice_id(&self) -> String {
            "mock_narrator".to_string()
        }
        fn is_mob_enabled(&self) -> bool {
            true
        }
        fn format_voice_list_for_analysis(&self, _voices: &[crate::services::tts::Voice]) -> String {
            "mock voice list".to_string()
        }
        fn get_script_generator(&self) -> Box<dyn ScriptGenerator> {
            Box::new(JsonScriptGenerator::new())
        }
    }

    #[tokio::test]
    async fn test_cache_miss_generates_segments_file() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let test_root = temp_dir.path();

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
            llm: crate::services::llm::LlmConfig {
                provider: "mock".to_string(),
                retry_count: 0,
                retry_delay_seconds: 0,
                gemini: None,
                ollama: None,
                openai: None,
            },
            audio: crate::core::config::AudioConfig {
                provider: "edge-tts".to_string(),
                edge_tts: Some(Default::default()),
                ..crate::core::config::AudioConfig::default()
            },
        };

        let filename = "chapter_1.txt";
        let chapter_path = input_dir.join(filename);
        fs::write(&chapter_path, "Some story text.")?;

        let mock_llm = Box::new(MockLlmClient::new());
        let call_count = mock_llm.call_count.clone();

        let mock_tts = Box::new(MockTtsClient { should_fail: true });

        // Use NativeStorage for tests
        let storage = Arc::new(NativeStorage::new());

        let mut workflow = WorkflowManager::new(config.clone(), mock_llm, mock_tts, storage).await?;

        let result = workflow.process_chapter(chapter_path.to_str().unwrap(), filename).await;

        assert!(
            result.is_err(),
            "Expected synthesis failure due to mock error"
        );

        assert_eq!(
            *call_count.lock().unwrap(),
            2,
            "Should call LLM twice (Analysis + Script)"
        );

        let segments_path = build_dir.join("chapter_1_txt").join("segments.json");
        assert!(segments_path.exists(), "segments.json should be created");

        let content = fs::read_to_string(segments_path)?;
        assert!(content.contains("Test audio"));

        Ok(())
    }

    #[tokio::test]
    async fn test_flattened_output_structure() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let test_root = temp_dir.path();

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
            llm: crate::services::llm::LlmConfig {
                provider: "mock".to_string(),
                retry_count: 0,
                retry_delay_seconds: 0,
                gemini: None,
                ollama: None,
                openai: None,
            },
            audio: crate::core::config::AudioConfig {
                provider: "edge-tts".to_string(),
                edge_tts: Some(Default::default()),
                ..crate::core::config::AudioConfig::default()
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
            speaker: Some("Narrator".to_string()),
            text: "Audio".to_string(),
            style: None,
            voice_id: None,
        }];
        fs::write(&segments_path, serde_json::to_string(&cached_segments)?)?;

        let mock_llm = Box::new(MockLlmClient::new());
        let mock_tts = Box::new(MockTtsClient { should_fail: false });
        let storage = Arc::new(NativeStorage::new());

        let mut workflow = WorkflowManager::new(config, mock_llm, mock_tts, storage).await?;
        workflow.process_chapter(chapter_path.to_str().unwrap(), filename).await?;

        // Check output
        let output_file = output_dir.join("chapter_flat.mp3");
        assert!(
            output_file.exists(),
            "Output file should exist at root of output folder"
        );

        let sub_dir = output_dir.join("chapter_flat_txt");
        assert!(
            !sub_dir.exists(),
            "Subdirectory should NOT exist in output folder"
        );

        let build_chapter_dir = build_dir.join("chapter_flat_txt");
        assert!(build_chapter_dir.exists(), "Build dir should exist");
        assert!(build_chapter_dir.join("segments.json").exists(), "Segments json should exist");
        
        // Chunk should not exist
        let chunk_file = build_chapter_dir.join("chunk_0000.mp3");
        assert!(!chunk_file.exists(), "Chunk should be deleted");

        Ok(())
    }

    #[tokio::test]
    async fn test_cache_hit_skips_llm() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let test_root = temp_dir.path();

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
            llm: crate::services::llm::LlmConfig {
                provider: "mock".to_string(),
                retry_count: 0,
                retry_delay_seconds: 0,
                gemini: None,
                ollama: None,
                openai: None,
            },
            audio: crate::core::config::AudioConfig {
                provider: "edge-tts".to_string(),
                edge_tts: Some(Default::default()),
                ..crate::core::config::AudioConfig::default()
            },
        };

        let filename = "chapter_2.txt";
        let chapter_path = input_dir.join(filename);
        fs::write(&chapter_path, "Some story text.")?;

        let chapter_build_dir = build_dir.join("chapter_2_txt");
        fs::create_dir_all(&chapter_build_dir)?;
        let segments_path = chapter_build_dir.join("segments.json");

        let cached_segments = vec![AudioSegment {
            speaker: Some("Narrator".to_string()),
            text: "Cached audio".to_string(),
            style: None,
            voice_id: None,
        }];
        fs::write(&segments_path, serde_json::to_string(&cached_segments)?)?;

        let chunk_path = chapter_build_dir.join("chunk_0000.mp3");
        fs::write(&chunk_path, b"fake mp3 data")?;

        let mock_llm = Box::new(MockLlmClient::new());
        let call_count = mock_llm.call_count.clone();

        let mock_tts = Box::new(MockTtsClient { should_fail: false });
        let storage = Arc::new(NativeStorage::new());

        let mut workflow = WorkflowManager::new(config.clone(), mock_llm, mock_tts, storage).await?;

        let result = workflow.process_chapter(chapter_path.to_str().unwrap(), filename).await;

        assert!(result.is_ok(), "Should complete successfully");

        assert_eq!(
            *call_count.lock().unwrap(),
            0,
            "Should use cache and NOT call LLM"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_voice_filtering_in_analysis_prompt() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let test_root = temp_dir.path();

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
            llm: crate::services::llm::LlmConfig {
                provider: "mock".to_string(),
                retry_count: 0,
                retry_delay_seconds: 0,
                gemini: None,
                ollama: None,
                openai: None,
            },
            audio: crate::core::config::AudioConfig {
                provider: "edge-tts".to_string(),
                language: "zh".to_string(),
                exclude_locales: vec!["zh-HK".to_string()],
                edge_tts: Some(Default::default()),
                ..crate::core::config::AudioConfig::default()
            },
        };

        let filename = "chapter_filter.txt";
        let chapter_path = input_dir.join(filename);
        fs::write(&chapter_path, "Text")?;

        // Setup Mock LLM to capture prompt
        #[derive(Debug)]
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
        let mock_llm = Box::new(CapturingLlmClient {
            prompts: prompts_store.clone(),
        });

        // Setup Mock TTS with voices
        struct MockTts {
            voices: Vec<crate::services::tts::Voice>,
        }
        #[async_trait]
        impl TtsClient for MockTts {
            async fn list_voices(&self) -> Result<Vec<crate::services::tts::Voice>> {
                Ok(self.voices.clone())
            }
            async fn synthesize(
                &self,
                _: &AudioSegment,
                _: &CharacterMap,
                _: &[String],
            ) -> Result<Vec<u8>> {
                Ok(vec![])
            }
            async fn get_random_voice(&self, _: Option<&str>, _: &[String]) -> Result<String> {
                Ok("mock".to_string())
            }
            fn get_narrator_voice_id(&self) -> String {
                "mock_narrator".to_string()
            }
            fn is_mob_enabled(&self) -> bool {
                true
            }
            fn format_voice_list_for_analysis(&self, voices: &[crate::services::tts::Voice]) -> String {
                voices
                    .iter()
                    .map(|v| v.short_name.clone())
                    .collect::<Vec<_>>()
                    .join(" ")
            }
            fn get_script_generator(&self) -> Box<dyn ScriptGenerator> {
                Box::new(JsonScriptGenerator::new())
            }
        }

        let voices = vec![
            crate::services::tts::Voice {
                short_name: "zh-TW-A".to_string(),
                gender: "Male".to_string(),
                locale: "zh-TW".to_string(),
                name: "A".to_string(),
                friendly_name: None,
            },
            crate::services::tts::Voice {
                short_name: "zh-HK-B".to_string(),
                gender: "Female".to_string(),
                locale: "zh-HK".to_string(),
                name: "B".to_string(),
                friendly_name: None,
            },
            crate::services::tts::Voice {
                short_name: "zh-CN-C".to_string(),
                gender: "Male".to_string(),
                locale: "zh-CN".to_string(),
                name: "C".to_string(),
                friendly_name: None,
            },
        ];
        let mock_tts = Box::new(MockTts { voices });
        let storage = Arc::new(NativeStorage::new());

        let mut workflow = WorkflowManager::new(config, mock_llm, mock_tts, storage).await?;
        let _ = workflow.process_chapter(chapter_path.to_str().unwrap(), filename).await;

        let prompts = prompts_store.lock().unwrap();
        let analysis_prompt = &prompts[0];

        // Assertions
        assert!(analysis_prompt.contains("zh-TW-A"));
        assert!(analysis_prompt.contains("zh-CN-C"));
        assert!(
            !analysis_prompt.contains("zh-HK-B"),
            "Excluded locale voice should not be in prompt"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_protagonist_exclusion_and_chapter_mob() -> Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let test_root = temp_dir.path();

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
            llm: crate::services::llm::LlmConfig {
                provider: "mock".to_string(),
                retry_count: 0,
                retry_delay_seconds: 0,
                gemini: None,
                ollama: None,
                openai: None,
            },
            audio: crate::core::config::AudioConfig {
                provider: "edge-tts".to_string(),
                edge_tts: Some(crate::services::tts::edge::EdgeTtsConfig {
                    narrator_voice: Some("Voice_Narrator".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        };

        let filename = "chapter_test.txt";
        fs::write(input_dir.join(filename), "Text")?;

        // Mock LLM: Returns Protag
        #[derive(Debug)]
        struct ProtagLlm;
        #[async_trait]
        impl LlmClient for ProtagLlm {
            async fn chat(&self, _: &str, user: &str) -> Result<String> {
                if user.contains("請分析以下文本") {
                    return Ok(r#"{
                        "characters": [
                            { "name": "Hero", "gender": "Male", "is_protagonist": true, "voice_id": "Voice_Hero" },
                            { "name": "章節路人(男)", "gender": "Male", "voice_id": "placeholder_chapter_mob_male" }
                        ]
                    }"#.to_string());
                }
                // Script gen
                Ok(r#"[
                    {"speaker": "Hero", "text": "I am hero.", "voice_id": null},
                    {"speaker": "章節路人(男)", "text": "I am mob.", "voice_id": null}
                ]"#
                .to_string())
            }
        }

        // Mock TTS: Captures exclusions
        struct VerifyingTts {
            exclusions: Arc<Mutex<Vec<String>>>,
        }
        #[async_trait]
        impl TtsClient for VerifyingTts {
            async fn list_voices(&self) -> Result<Vec<crate::services::tts::Voice>> {
                Ok(vec![])
            }
            async fn synthesize(
                &self,
                segment: &AudioSegment,
                map: &CharacterMap,
                excluded: &[String],
            ) -> Result<Vec<u8>> {
                let mut ex = self.exclusions.lock().unwrap();
                *ex = excluded.to_vec();

                // Verify Chapter Mob resolution
                if matches!(segment.speaker.as_deref(), Some("章節路人(男)")) {
                    let info = map.characters.get("章節路人(男)").unwrap();
                    assert_eq!(info.voice_id.as_deref(), Some("Voice_Mob_Male_Fixed"));
                }

                Ok(vec![])
            }
            async fn get_random_voice(
                &self,
                gender: Option<&str>,
                excluded: &[String],
            ) -> Result<String> {
                // Verify exclusion list is passed here too
                assert!(excluded.contains(&"Voice_Narrator".to_string()));
                assert!(excluded.contains(&"Voice_Hero".to_string()));

                if gender == Some("Male") {
                    Ok("Voice_Mob_Male_Fixed".to_string())
                } else {
                    Ok("Voice_Mob_Female_Fixed".to_string())
                }
            }
            fn get_narrator_voice_id(&self) -> String {
                "Voice_Narrator".to_string()
            }
            fn is_mob_enabled(&self) -> bool {
                true
            }
            fn format_voice_list_for_analysis(&self, _voices: &[crate::services::tts::Voice]) -> String {
                "".to_string()
            }
            fn get_script_generator(&self) -> Box<dyn ScriptGenerator> {
                Box::new(JsonScriptGenerator::new())
            }
        }

        let exclusions = Arc::new(Mutex::new(Vec::new()));
        let mock_tts = Box::new(VerifyingTts {
            exclusions: exclusions.clone(),
        });
        let mock_llm = Box::new(ProtagLlm);
        let storage = Arc::new(NativeStorage::new());

        let mut workflow = WorkflowManager::new(config, mock_llm, mock_tts, storage).await?;
        workflow
            .process_chapter(input_dir.join(filename).to_str().unwrap(), filename)
            .await?;

        let ex = exclusions.lock().unwrap();
        assert!(ex.contains(&"Voice_Narrator".to_string()));
        assert!(ex.contains(&"Voice_Hero".to_string()));

        Ok(())
    }
}
