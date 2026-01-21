use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct WorkflowState {
    pub completed_chapters: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CharacterMap {
    pub characters: HashMap<String, CharacterInfo>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct CharacterInfo {
    pub gender: String, // "Male", "Female"
    pub voice_id: Option<String>,
    pub description: Option<String>, // Context for LLM
    #[serde(default)]
    pub is_protagonist: bool,
}
