# novel2audiobook

`novel2audiobook` is a powerful CLI tool written in Rust that converts text novels into high-quality audiobooks. It leverages Large Language Models (LLMs) to analyze characters and generate expressive speech synthesis markup (SSML) or audio scripts, which are then synthesized using Microsoft Edge TTS or GPT-SoVITS.

## Features

- **LLM-Powered Character Analysis**: Automatically identifies characters, their gender, and importance from the text using LLMs (Gemini, Ollama, or OpenAI).
- **Context-Aware Voice Assignment**: Assigns specific voices to characters based on analysis and maintains consistency across chapters.
- **Multiple TTS Providers**:
  - **Edge TTS**: Uses high-quality, free-to-use voices from Microsoft Edge TTS with SSML support.
  - **GPT-SoVITS**: Supports custom voice models via GPT-SoVITS API for even more realistic and emotional speech.
- **Unattended Mode**: Option to run the entire conversion process without user intervention between chapters.
- **Robust Error Handling**: Configurable retry mechanisms for LLM API calls to handle transient failures gracefully.
- **Smart Caching**: Caches analysis results and SSML/scripts to avoid redundant LLM calls and speed up reprocessing.
- **Resume Capability**: Skips already synthesized audio chunks if the process is interrupted.
- **Multi-Provider Support**: Supports Google Gemini, Ollama (local), and OpenAI.

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (latest stable version)
- An API key for your chosen LLM provider (Gemini, OpenAI) or a local Ollama instance.
- (Optional) A running GPT-SoVITS API server if you plan to use that provider.

## Installation

1. Clone the repository:
   ```bash
   git clone https://github.com/yourusername/novel2audiobook.git
   cd novel2audiobook
   ```

2. Build the project:
   ```bash
   cargo build --release
   ```

## Configuration

The application is configured via a `config.yml` file in the project root. If voice settings are missing, the tool will interactively ask you to select them on the first run.

Example `config.yml`:

```yaml
input_folder: ./input_chapters
output_folder: ./output_audio
build_folder: ./build

# Run without asking for confirmation between chapters
unattended: false 

# LLM Configuration
llm:
  # Choose your provider: gemini, ollama, or openai
  provider: gemini 
  retry_count: 3
  retry_delay_seconds: 10
  
  gemini:
    api_key: "YOUR_GEMINI_API_KEY"
    model: "gemini-3-flash-preview"
    
  ollama:
    base_url: "http://localhost:11434"
    model: "llama3:latest"
    
  openai:
    api_key: "YOUR_OPENAI_API_KEY"
    model: "gpt-4o"
    base_url: "https://api.openai.com/v1" # Optional

# Audio Configuration
audio:
  # Choose provider: edge-tts or gpt_sovits
  provider: edge-tts
  language: zh # Filter voices by language (e.g., zh, en, ja)
  
  # Exclude specific locales (e.g. HK voices for Chinese often sound different)
  exclude_locales:
    - zh-HK

  # --- Edge TTS Settings ---
  edge-tts:
    narrator_voice: zh-CN-XiaoxiaoNeural
    default_male_voice: zh-TW-YunJheNeural
    default_female_voice: zh-TW-HsiaoYuNeural
    style: false # Enable to use SSML styles (may be restricted in free version)

  # --- GPT-SoVITS Settings ---
  gpt_sovits:
    token: "YOUR_API_TOKEN" # If authentication is required
    base_url: "http://127.0.0.1:9000/" # Your GPT-SoVITS API endpoint
    enable_mobs: false # Use random voices for mobs?
    autofix: true # Automatically retry with LLM on syntax errors
    retry: 5
    narrator_voice: "星穹铁道-中文-丹恒" # Exact name of the voice model
    
    # Inference parameters
    top_k: 10
    top_p: 1
    temperature: 1
    speed_factor: 1
    repetition_penalty: 1.35
```

## Usage

1. **Prepare Input**: Place your novel chapters as `.txt` files in the `input_chapters` directory (or whatever you configured as `input_folder`). 
   - Note: The tool processes files in alphabetical order. It's recommended to name them like `001.txt`, `002.txt`, etc.

2. **Run the Tool**:
   ```bash
   cargo run --release
   ```
   
   If `unattended` is `false` (default), the tool will pause after each chapter is completed and ask if you want to proceed to the next one.

3. **First Run Setup**: If your `config.yml` lacks voice selections, the tool will prompt you to select a narrator, a default male voice, and a default female voice.

4. **Output**: The generated audiobooks will be saved in the `output_audio` directory, organized by chapter.

## Utility Tools

### gpt_sovits-remove-unneed.py

A Python script is included to help clean up GPT-SoVITS voice model JSON files. This is useful if you have a large model file with many low-quality or unwanted voices.

**Features:**
- Filters out voices with too few emotion samples.
- Removes generic or unwanted characters based on name/tag blocklists (e.g., "Villager", "Soldier", "Unknown").
- Merges tags for duplicate entries.

**Usage:**
1. Edit the script to point to your input JSON file:
   ```python
   input_filename = 'your_voice_list.json'
   output_filename = 'cleaned_voice_list.json'
   ```
2. Run the script:
   ```bash
   python gpt_sovits-remove-unneed.py
   ```

## Directory Structure

- `input_chapters/`: Put your text files here.
- `output_audio/`: Resulting MP3 files will be saved here.
- `build/`: Stores intermediate files (SSML, character maps, state) and cache. **Do not delete this if you want to resume progress or keep character voice consistency.**

## License

This project is licensed under the MIT License.



## Support the Project

If this project has saved you time or helped you in your workflow, consider supporting its continued development. Your contribution helps me keep the project maintained and feature-rich!

[![][ko-fi-shield]][ko-fi-link]
[![][paypal-shield]][paypal-link]


<!-- Link Definitions -->
[ko-fi-shield]: https://img.shields.io/badge/Ko--fi-F16061?style=for-the-badge&logo=ko-fi&logoColor=white
[ko-fi-link]: https://ko-fi.com/kautism
[paypal-shield]: https://img.shields.io/badge/PayPal-00457C?style=for-the-badge&logo=paypal&logoColor=white
[paypal-link]: https://paypal.me/kautism

