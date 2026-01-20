# novel2audiobook

`novel2audiobook` is a powerful CLI tool written in Rust that converts text novels into high-quality audiobooks. It leverages Large Language Models (LLMs) to analyze characters and generate expressive speech synthesis markup (SSML), which is then synthesized using Microsoft Edge TTS.

## Features

- **LLM-Powered Character Analysis**: Automatically identifies characters, their gender, and importance from the text using LLMs (Gemini, Ollama, or OpenAI).
- **Context-Aware Voice Assignment**: Assigns specific voices to characters based on analysis and maintains consistency across chapters.
- **Expressive Audio**: Generates SSML to control prosody and style, making the audiobook sound more natural.
- **Edge TTS Integration**: Uses high-quality, free-to-use voices from Microsoft Edge TTS.
- **Smart Caching**: Caches analysis results and SSML to avoid redundant LLM calls and speed up reprocessing.
- **Resume Capability**: Skips already synthesized audio chunks if the process is interrupted.
- **Multi-Provider Support**: Supports Google Gemini, Ollama (local), and OpenAI.

## Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (latest stable version)
- An API key for your chosen LLM provider (Gemini, OpenAI) or a local Ollama instance.

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

# LLM Configuration
llm:
  # Choose your provider: gemini, ollama, or openai
  provider: gemini 
  
  gemini:
    api_key: "YOUR_GEMINI_API_KEY"
    model: "gemini-1.5-flash" # or other available models
    
  ollama:
    base_url: "http://localhost:11434"
    model: "llama3:latest"
    
  openai:
    api_key: "YOUR_OPENAI_API_KEY"
    model: "gpt-4o"
    base_url: "https://api.openai.com/v1" # Optional

# Audio Configuration (Edge TTS)
audio:
  provider: edge-tts
  language: zh # Filter voices by language (e.g., zh, en, ja)
  # These can be left blank initially to trigger the interactive setup
  narrator_voice: zh-TW-HsiaoChenNeural
  default_male_voice: zh-TW-YunJheNeural
  default_female_voice: zh-TW-HsiaoYuNeural
```

## Usage

1. **Prepare Input**: Place your novel chapters as `.txt` files in the `input_chapters` directory (or whatever you configured as `input_folder`). 
   - Note: The tool processes files in alphabetical order. It's recommended to name them like `001.txt`, `002.txt`, etc.

2. **Run the Tool**:
   ```bash
   cargo run --release
   ```

3. **First Run Setup**: If your `config.yml` lacks voice selections, the tool will prompt you to select a narrator, a default male voice, and a default female voice from the available Edge TTS voices.

4. **Output**: The generated audiobooks will be saved in the `output_audio` directory, organized by chapter.

## Directory Structure

- `input_chapters/`: Put your text files here.
- `output_audio/`: Resulting MP3 files will be saved here.
- `build/`: Stores intermediate files (SSML, character maps, state) and cache. **Do not delete this if you want to resume progress or keep character voice consistency.**

## License

This project is licensed under the MIT License.
