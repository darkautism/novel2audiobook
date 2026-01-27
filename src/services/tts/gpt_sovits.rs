use crate::core::state::CharacterMap;
use crate::services::llm::LlmClient;
use crate::services::script::{AudioSegment, GptSovitsScriptGenerator, ScriptGenerator};
use crate::services::tts::gpt_sovits_config::{
    load_or_refresh_metadata, GptSovitsConfig, GptSovitsVoiceMap,
};
use crate::services::tts::{
    TtsClient, Voice, VOICE_ID_CHAPTER_MOB_FEMALE, VOICE_ID_CHAPTER_MOB_MALE, VOICE_ID_MOB_FEMALE,
    VOICE_ID_MOB_MALE, VOICE_ID_MOB_NEUTRAL,
};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use rand::seq::IndexedRandom;
use serde_json::json;

#[derive(serde::Deserialize)]
struct GptSovitsDownloadResponse {
    msg: String,
    audio_url: String,
}

pub async fn list_voices(
    config: &GptSovitsConfig,
    language: &str,
    llm: Option<&dyn LlmClient>,
) -> Result<Vec<Voice>> {
    let metadata = load_or_refresh_metadata(config, language, llm).await?;
    Ok(metadata_to_voices(&metadata))
}

fn metadata_to_voices(metadata: &GptSovitsVoiceMap) -> Vec<Voice> {
    metadata
        .iter()
        .map(|(name, meta)| Voice {
            name: name.clone(),
            short_name: name.clone(),
            gender: meta.gender.clone(),
            locale: "zh".to_string(),
            friendly_name: Some(format!("{} {:?}", name, meta.tags)),
        })
        .collect()
}

pub struct GptSovitsClient {
    config: GptSovitsConfig,
    metadata: GptSovitsVoiceMap,
}

impl GptSovitsClient {
    pub async fn new(
        config: GptSovitsConfig,
        language: &str,
        llm: Option<&dyn LlmClient>,
    ) -> Result<Self> {
        let metadata = load_or_refresh_metadata(&config, language, llm).await?;

        Ok(Self { config, metadata })
    }

    fn pick_random_voice(
        &self,
        gender: Option<&str>,
        excluded_voices: &[String],
    ) -> Result<String> {
        let mut rng = rand::rng();
        let candidates: Vec<&String> = self
            .metadata
            .iter()
            .filter_map(|(name, meta)| {
                if excluded_voices.contains(name) {
                    return None;
                }
                if let Some(g) = gender {
                    if !meta.gender.eq_ignore_ascii_case(g) {
                        return None;
                    }
                }
                Some(name)
            })
            .collect();

        if let Some(v) = candidates.choose(&mut rng) {
            Ok(v.to_string())
        } else {
            // Fallback to any voice not excluded?
            let fallback: Vec<&String> = self
                .metadata
                .keys()
                .filter(|k| !excluded_voices.contains(k))
                .collect();
            if let Some(v) = fallback.choose(&mut rng) {
                Ok(v.to_string())
            } else {
                // Absolute fallback
                self.metadata
                    .keys()
                    .next()
                    .cloned()
                    .ok_or_else(|| anyhow!("No GPT-SoVITS voices available"))
            }
        }
    }

    async fn resolve_voice(
        &self,
        speaker: &str,
        char_map: &CharacterMap,
        excluded_voices: &[String],
    ) -> Result<String> {
        let gpt_sovits_config = &self.config;

        // 1. Narrator
        if speaker == "旁白" || speaker.eq_ignore_ascii_case("Narrator") {
            if let Some(v) = &gpt_sovits_config.narrator_voice {
                return Ok(v.clone());
            }
            // If no narrator set, use random female?
            return self.pick_random_voice(Some("Female"), excluded_voices);
        }

        // 2. Character Map
        if let Some(info) = char_map.characters.get(speaker) {
            if let Some(voice_id) = &info.voice_id {
                // Check placeholders
                match voice_id.as_str() {
                    VOICE_ID_MOB_MALE | VOICE_ID_CHAPTER_MOB_MALE => {
                        return self.pick_random_voice(Some("Male"), excluded_voices);
                    }
                    VOICE_ID_MOB_FEMALE | VOICE_ID_CHAPTER_MOB_FEMALE => {
                        return self.pick_random_voice(Some("Female"), excluded_voices);
                    }
                    VOICE_ID_MOB_NEUTRAL => {
                        return self.pick_random_voice(None, excluded_voices);
                    }
                    _ => return Ok(voice_id.clone()),
                }
            }

            // 3. Gender default - REMOVED
            // Random based on gender
            return self.pick_random_voice(Some(&info.gender), excluded_voices);
        }

        // 4. Fallback
        self.pick_random_voice(None, excluded_voices)
    }
}

#[async_trait]
impl TtsClient for GptSovitsClient {
    async fn list_voices(&self) -> Result<Vec<Voice>> {
        Ok(metadata_to_voices(&self.metadata))
    }

    async fn get_voice_styles(&self, voice_id: &str) -> Result<Vec<String>> {
        if let Some(meta) = self.metadata.get(voice_id) {
            Ok(meta.emotion.clone())
        } else {
            Ok(Vec::new())
        }
    }

    async fn check_and_fix_segments(
        &self,
        segments: &mut Vec<AudioSegment>,
        char_map: &CharacterMap,
        excluded_voices: &[String],
        llm: &dyn LlmClient,
    ) -> Result<()> {
        let gpt_sovits_config = &self.config;

        // 1. Resolve Voice IDs & Validate
        // Store indices of invalid segments
        #[derive(serde::Serialize)]
        struct InvalidSegment {
            index: usize,
            text: String,
            current_style: String,
            voice: String,
            valid_styles: Vec<String>,
        }

        let mut invalid_emotion_segments = Vec::new();
        let mut validation_errors = Vec::new();

        // Pass 1: Resolution and Validation
        for (i, segment) in segments.iter_mut().enumerate() {
            // A. Resolve Voice ID if missing
            if segment.voice_id.is_none() {
                if let Some(speaker) = &segment.speaker {
                    match self.resolve_voice(speaker, char_map, excluded_voices).await {
                        Ok(vid) => segment.voice_id = Some(vid),
                        Err(_) => {
                            validation_errors.push(format!(
                                "Segment {}: Unable to resolve voice for speaker '{}'",
                                i, speaker
                            ));
                            continue;
                        }
                    }
                } else {
                    validation_errors.push(format!("Segment {}: Missing both voice_id and speaker", i));
                    continue;
                }
            }

            let voice_id = segment.voice_id.as_ref().unwrap();

            // B. Validate Voice ID Existence
            if !self.metadata.contains_key(voice_id) {
                validation_errors.push(format!(
                    "Segment {}: Voice ID '{}' does not exist in metadata",
                    i, voice_id
                ));
                continue;
            }

            // C. Validate Emotion (Style)
            if let Some(style) = &segment.style {
                if !style.is_empty() {
                    let valid_styles = &self.metadata[voice_id].emotion;
                    if !valid_styles.contains(style) {
                        invalid_emotion_segments.push(InvalidSegment {
                            index: i,
                            text: segment.text.clone(),
                            current_style: style.clone(),
                            voice: voice_id.clone(),
                            valid_styles: valid_styles.clone(),
                        });
                    }
                }
            }
        }

        // 2. Autofix
        if !invalid_emotion_segments.is_empty() && gpt_sovits_config.autofix {
            println!(
                "Found {} segments with invalid emotions. Attempting autofix via LLM...",
                invalid_emotion_segments.len()
            );

            let prompt_payload = serde_json::to_string_pretty(&invalid_emotion_segments)?;
            let prompt = format!(
                "The following audio segments have invalid emotions (styles) for their assigned voices.\n\
                 Please correct the 'style' field for each segment to one of the provided 'valid_styles'.\n\
                 Do NOT change the text or index.\n\
                 Select the most appropriate emotion from the valid list based on the text context and the original invalid style.\n\
                 If no emotion fits perfectly, pick 'default' or the most neutral option available in valid_styles.\n\
                 \n\
                 Input Data:\n\
                 {}\n\
                 \n\
                 Return a JSON list of objects with the following structure:\n\
                 [ {{ \"index\": 0, \"style\": \"CorrectedStyle\" }}, ... ]",
                prompt_payload
            );

            let response = llm
                .chat("You are a helpful assistant fixing JSON data.", &prompt)
                .await?;

            let clean_json = crate::services::script::strip_code_blocks(&response);
            #[derive(serde::Deserialize)]
            struct Fix {
                index: usize,
                style: String,
            }
            let fixes: Vec<Fix> = serde_json::from_str(&clean_json)
                .map_err(|e| anyhow!("Failed to parse autofix response: {}", e))?;

            for fix in fixes {
                if fix.index < segments.len() {
                    segments[fix.index].style = Some(fix.style);
                }
            }

            println!("Autofix applied. Re-validating...");
        } else if !invalid_emotion_segments.is_empty() {
            println!(
                "Autofix is disabled. Found {} segments with invalid emotions.",
                invalid_emotion_segments.len()
            );
        }

        // 3. Final Re-validation
        let mut final_errors = validation_errors; // Carry over structural errors
        for (i, segment) in segments.iter().enumerate() {
            // We only need to check emotions again, as structural stuff was checked/resolved or already in final_errors
            if let Some(voice_id) = &segment.voice_id {
                 if let Some(meta) = self.metadata.get(voice_id) {
                     if let Some(style) = &segment.style {
                         if !style.is_empty() && !meta.emotion.contains(style) {
                             final_errors.push(format!(
                                 "Segment {}: Invalid emotion '{}' for voice '{}'. Valid: {:?}",
                                 i, style, voice_id, meta.emotion
                             ));
                         }
                     }
                 }
            }
        }

        if !final_errors.is_empty() {
            eprintln!("Manual Fix Required:");
            for err in &final_errors {
                eprintln!(" - {}", err);
            }
            anyhow::bail!("GPT-SoVITS Validation Failed: {} errors found.", final_errors.len());
        }

        Ok(())
    }

    async fn synthesize(
        &self,
        segment: &AudioSegment,
        char_map: &CharacterMap,
        excluded_voices: &[String],
    ) -> Result<Vec<u8>> {
        let voice_id = if let Some(vid) = &segment.voice_id {
            vid.clone()
        } else if let Some(speaker) = &segment.speaker {
            self.resolve_voice(speaker, char_map, excluded_voices)
                .await?
        } else {
            panic!("No speaker or voice_id specified for segment");
        };
        let gpt_sovits_config = &self.config;

        let payload = json!({
          "batch_size": 10,
          "batch_threshold": 0.75,
          "emotion": segment.style.clone().unwrap_or_default(),
          "fragment_interval": 0.3,
          "if_sr": false,
          "media_type": "mp3",
          "model_name": voice_id,
          "parallel_infer": true,
          "prompt_text_lang": "中文",
          "repetition_penalty": gpt_sovits_config.repetition_penalty,
          "sample_steps": 16,
          "seed": format!("{}", rand::random::<u32>()),
          "speed_facter": gpt_sovits_config.speed_factor,
          "split_bucket": true,
          "version": "v4",
          "text": segment.text,
          "text_lang": "中文",
          "top_k": gpt_sovits_config.top_k,
          "top_p": gpt_sovits_config.top_p,
          "temperature": gpt_sovits_config.temperature,
          "text_split_method": "按标点符号切",
          //"text_split_method": "凑四句一切",
        });

        let client = reqwest::Client::new();

        let mut retry = gpt_sovits_config.retry;
        let mut download_url = String::new();
        while retry > 0 {
            let mut req = client
                .post(format!("{}infer_single", gpt_sovits_config.base_url))
                .json(&payload);

            if !gpt_sovits_config.token.is_empty() {
                req = req.header(
                    "Authorization",
                    format!("Bearer {}", gpt_sovits_config.token),
                );
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                let txt = resp.text().await?;
                return Err(anyhow!("GPT-SoVITS synthesis failed: {}", txt));
            }

            let body_text = resp.text().await?;

            // Handle cases where it might be quoted
            let response = body_text.trim().trim_matches('"').to_string();
            let download_response: GptSovitsDownloadResponse =
                serde_json::from_str(&response).unwrap();
            if download_response.msg != "合成成功" {
                if retry == 1 {
                    return Err(anyhow!(
                        "GPT-SoVITS synthesis failed: {}",
                        download_response.msg
                    ));
                } else {
                    println!(
                        "GPT-SoVITS synthesis failed: {}, retrying...\nPayload: {:?}",
                        payload, download_response.msg
                    );
                    retry -= 1;
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                    continue;
                }
            } else {
                retry = -1;
                download_url = download_response.audio_url;
            }
        }

        let base_url = gpt_sovits_config.base_url.clone();
        let mut durl = url::Url::parse(&download_url)?;
        let burl = url::Url::parse(&base_url)?;
        durl.set_host(burl.host_str())?;
        
        let _ = durl.set_port(burl.port());
        tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
        // Download WAV
        let wav_resp = client.get(durl.as_str()).send().await?;
        let wav_bytes = wav_resp.bytes().await?;

        Ok(wav_bytes.into())
    }

    async fn get_random_voice(
        &self,
        gender: Option<&str>,
        excluded_voices: &[String],
    ) -> Result<String> {
        self.pick_random_voice(gender, excluded_voices)
    }

    fn get_narrator_voice_id(&self) -> String {
        self.config
            .narrator_voice
            .clone()
            .unwrap_or_else(|| "zh-TW-HsiaoChenNeural".to_string())
    }

    fn is_mob_enabled(&self) -> bool {
        false
    }

    fn format_voice_list_for_analysis(&self, voices: &[Voice]) -> String {
        voices
            .iter()
            .map(|v| {
                format!(
                    "{{ \"id\": \"{}\", \"gender\": \"{}\", \"info\": \"{}\" }}",
                    v.short_name,
                    v.gender,
                    v.friendly_name.as_deref().unwrap_or("")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn get_script_generator(&self) -> Box<dyn ScriptGenerator> {
        Box::new(GptSovitsScriptGenerator::new(self.get_narrator_voice_id()))
    }
}
