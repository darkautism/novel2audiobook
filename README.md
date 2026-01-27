# novel2audiobook

`novel2audiobook` is a high-performance Rust tool that converts text novels into high-quality audiobooks. It leverages Large Language Models (LLMs) to perform character analysis and generate expressive audio scripts, which are then synthesized using state-of-the-art TTS providers.

## Key Features

-   **LLM Character Analysis**: Automatically identifies characters, gender, and roles (Protagonist, Mob, etc.) using Gemini, Ollama, or OpenAI.
-   **Context-Aware Voices**: Assigns consistent voices to characters across chapters.
-   **Parallel Synthesis**: drastically speeds up generation by processing audio segments in parallel.
-   **Multiple TTS Providers**:
    -   **Edge TTS**: High-quality, free online voices.
    -   **GPT-SoVITS**: Support for custom emotional voice models (local/remote).
    -   **Qwen3-TTS**: Local TTS support.
-   **Resume & Cache**: Skips already synthesized chunks and caches LLM analysis to save costs/time.
-   **Unattended Mode**: Batch process multiple chapters without user intervention.

## Prerequisites

-   [Rust](https://www.rust-lang.org/tools/install) (latest stable)
-   LLM API Key (Gemini, OpenAI) or Local LLM (Ollama).
-   (Optional) Python environment for local GPT-SoVITS or Qwen3-TTS.

## Installation

```bash
git clone https://github.com/yourusername/novel2audiobook.git
cd novel2audiobook
cargo build --release
```

## Configuration

The tool uses `config.yml`. On the first run, it can interactively help you select voices.

Example `config.yml`:

```yaml
input_folder: ./input_chapters
output_folder: ./output_audio
build_folder: ./build

# Run continuously without asking for confirmation
unattended: false

llm:
  provider: gemini # gemini, ollama, openai
  retry_count: 3
  retry_delay_seconds: 10

  gemini:
    api_key: "YOUR_KEY"
    model: "gemini-2.0-flash"

  ollama:
    base_url: "http://localhost:11434"
    model: "llama3:latest"

audio:
  provider: edge-tts # edge-tts, gpt_sovits, qwen3_tts
  language: zh
  exclude_locales: ["zh-HK"]

  edge-tts:
    narrator_voice: zh-CN-XiaoxiaoNeural
    default_male_voice: zh-TW-YunJheNeural
    default_female_voice: zh-TW-HsiaoYuNeural
```

## Usage

1.  **Place Text Files**: Put `.txt` files in `input_chapters/`.
2.  **Run**:
    ```bash
    cargo run --release
    ```
3.  **Output**: MP3 files will appear in `output_audio/`.

## Directory Structure

-   `src/core`: Core logic (Config, State).
-   `src/services`: Service integrations (LLM, TTS, Workflow).
-   `src/utils`: Helper functions.

## License

MIT License.
