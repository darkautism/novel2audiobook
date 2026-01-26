use anyhow::{Context, Result};
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

// 定義 Python 版本常數
// 這裡使用 20260114 發布的 Python 3.10.19 (配合 Flash Attention 的 cp310)
const PYTHON_RELEASE_TAG: &str = "20260114";
const PYTHON_VERSION_TAG: &str = "3.10.19";

#[tokio::main]
async fn main() -> Result<()> {
    // 1. 定義安裝路徑
    let runtime_dir = Path::new(".").join(".runtime");

    // 2. 檢查是否已安裝
    if !runtime_dir.exists() {
        println!(">>> 偵測到環境未建立，正在下載 Portable Python...");
        setup_python_env(&runtime_dir).await?;
    } else {
        println!(">>> 環境已存在：{:?}", runtime_dir);
    }

    // 3. 取得 python 執行檔路徑
    let python_bin = get_python_executable(&runtime_dir);
    println!(">>> 使用 Python 直譯器：{:?}", python_bin);

    // 4. 安裝依賴 (順序很重要：先 Torch -> 再 Flash Attn -> 最後 Qwen-TTS)

    // 4.1 安裝 PyTorch (CUDA 12.4)
    // 必須先裝這個，否則後面裝 qwen-tts 會拉到預設的 cpu 版本
    println!(">>> [1/3] 檢查並安裝 PyTorch 2.6.0 (CUDA 12.4)...");
    install_torch_cuda_124(&python_bin)?;

    // 4.2 安裝 Flash Attention (Windows 特規)
    println!(">>> [2/3] 檢查並安裝 Flash Attention...");
    install_flash_attn_conditional(&python_bin)?;

    // 4.3 安裝 Qwen-TTS
    println!(">>> [3/3] 檢查並安裝 Qwen-TTS...");
    // 這裡使用 git+https 安裝最新版，或者您可以改成 "qwen-tts" 從 PyPI 安裝
    // 注意：如果有特定的依賴衝突，可以加入 "--no-deps"
    install_package(&python_bin, "qwen-tts", &[], &[])?;

    println!(">>> ✅ 完成！環境已準備就緒。");

    // 5. 執行 Qwen-TTS Demo
    println!(">>> 正在啟動 Qwen-TTS Demo...");

    let scripts_dir = python_bin.parent().unwrap().join("Scripts");
    let demo = scripts_dir.join("qwen-tts-demo.exe");

    let status = Command::new(&demo)
        .arg("Qwen/Qwen3-TTS-12Hz-1.7B-Base")
        .arg("--ip")
        .arg("0.0.0.0")
        .arg("--port")
        .arg("8000")
        .status()
        .context("執行 qwen-tts 失敗")?;

    if !status.success() {
        eprintln!("程式異常退出");
    }

    Ok(())
}

// --- 下載與環境建置相關 ---

fn get_download_url() -> Result<String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    // 基礎 URL 結構
    let base_url = format!(
        "https://github.com/astral-sh/python-build-standalone/releases/download/{}",
        PYTHON_RELEASE_TAG
    );

    // 根據 OS/Arch 組合檔名
    // 注意：Astral 的檔名規則是 cpython-<ver>+<date>-<arch>-<os>-<libc/compiler>-<options>.tar.gz
    let filename = match (os, arch) {
        ("windows", "x86_64") => format!(
            "cpython-{}+{}-x86_64-pc-windows-msvc-install_only_stripped.tar.gz",
            PYTHON_VERSION_TAG, PYTHON_RELEASE_TAG
        ),
        ("linux", "x86_64") => format!(
            "cpython-{}+{}-x86_64-unknown-linux-gnu-install_only_stripped.tar.gz",
            PYTHON_VERSION_TAG, PYTHON_RELEASE_TAG
        ),
        ("macos", "aarch64") => format!(
            "cpython-{}+{}-aarch64-apple-darwin-install_only_stripped.tar.gz",
            PYTHON_VERSION_TAG, PYTHON_RELEASE_TAG
        ),
        ("macos", "x86_64") => format!(
            "cpython-{}+{}-x86_64-apple-darwin-install_only_stripped.tar.gz",
            PYTHON_VERSION_TAG, PYTHON_RELEASE_TAG
        ),
        _ => return Err(anyhow::anyhow!("不支援的作業系統或架構: {} {}", os, arch)),
    };

    Ok(format!("{}/{}", base_url, filename))
}

async fn setup_python_env(target_dir: &Path) -> Result<()> {
    let url = get_download_url()?;

    // 下載
    let mut response = reqwest::get(url).await?;
    let total_size = response.content_length().unwrap_or(0);

    let pb = indicatif::ProgressBar::new(total_size);
    pb.set_style(indicatif::ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")?
        .progress_chars("#>-"));

    let mut downloaded_data = Vec::new();

    while let Some(chunk) = response.chunk().await? {
        pb.inc(chunk.len() as u64);
        downloaded_data.extend_from_slice(&chunk);
    }
    pb.finish_with_message("下載完成");

    // 解壓縮
    println!(">>> 正在解壓縮...");
    let tar = flate2::read::GzDecoder::new(Cursor::new(downloaded_data));
    let mut archive = tar::Archive::new(tar);

    // 解壓到當前目錄
    archive.unpack(target_dir.parent().unwrap_or(Path::new(".")))?;

    // 處理資料夾重新命名 (通常解壓出來是 'python')
    let extracted_folder = target_dir.parent().unwrap().join("python");
    if extracted_folder.exists() && extracted_folder != *target_dir {
        if target_dir.exists() {
            fs::remove_dir_all(target_dir)?;
        }
        fs::rename(extracted_folder, target_dir)?;
    }

    Ok(())
}

fn get_python_executable(runtime_dir: &Path) -> PathBuf {
    if cfg!(windows) {
        runtime_dir.join("python.exe")
    } else {
        runtime_dir.join("bin").join("python3")
    }
}

// --- 套件管理相關 ---

/// 檢查套件是否已安裝
fn is_package_installed(python_bin: &Path, package_name: &str) -> bool {
    let status = Command::new(python_bin)
        .arg("-I") // 保持隔離
        .arg("-m")
        .arg("pip")
        .arg("show")
        .arg(package_name)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match status {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

/// 通用安裝套件函數
fn install_package(
    python_bin: &Path,
    package_name: &str,
    extra_args: &[&str],
    env_vars: &[(&str, &str)],
) -> Result<()> {
    // 簡單判斷：如果名字裡包含 url (例如 http) 或是 .whl，我們就不檢查 is_installed，直接執行 install
    let is_url_or_file = package_name.contains("http") || package_name.ends_with(".whl");

    if !is_url_or_file && is_package_installed(python_bin, package_name) {
        println!("套件 '{}' 已存在，跳過安裝。", package_name);
        return Ok(());
    }

    println!("正在安裝 '{}'...", package_name);

    let mut cmd = Command::new(python_bin);
    cmd.arg("-I")
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg(package_name);

    // 加入環境變數
    for (key, val) in env_vars {
        cmd.env(*key, *val);
    }

    // 加入額外參數
    cmd.args(extra_args);

    let status = cmd
        .status()
        .context(format!("無法執行 pip 安裝命令: {}", package_name))?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("套件 '{}' 安裝失敗", package_name))
    }
}

/// 安裝 PyTorch 2.6.0 + CUDA 12.4
fn install_torch_cuda_124(python_bin: &Path) -> Result<()> {
    // 檢查 torch 是否存在且版本正確 (這裡只簡單檢查存在與否，嚴謹一點可以用 python script 檢查 version)
    if is_package_installed(python_bin, "torch") {
        println!("Torch 已安裝，跳過。");
        return Ok(());
    }

    println!("正在下載並安裝 PyTorch 2.6.0 (CUDA 12.4)...");
    let status = Command::new(python_bin)
        .arg("-I")
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("torch==2.6.0")
        .arg("torchvision") // 通常不需要 version locking 除非很嚴格
        .arg("torchaudio")
        .arg("--index-url")
        .arg("https://download.pytorch.org/whl/cu124")
        .status()
        .context("PyTorch 安裝失敗")?;

    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("PyTorch 安裝失敗"))
    }
}

/// 根據 OS 決定 Flash Attention 安裝策略
fn install_flash_attn_conditional(python_bin: &Path) -> Result<()> {
    if is_package_installed(python_bin, "flash-attn") {
        println!("Flash Attention 已安裝，跳過。");
        return Ok(());
    }

    if cfg!(windows) {
        println!(">>> 偵測到 Windows，使用自定義 Wheel 安裝 Flash Attention...");

        // v2.8.2, torch 2.6.0, cp310
        let wheel_url = "https://github.com/kingbri1/flash-attention/releases/download/v2.8.2/flash_attn-2.8.2+cu124torch2.6.0cxx11abiFALSE-cp310-cp310-win_amd64.whl";

        install_package(python_bin, wheel_url, &["--no-deps"], &[])?;
    } else {
        println!(">>> 偵測到 Linux/Mac，使用標準 pip 安裝 Flash Attention...");
        install_package(python_bin, "flash-attn", &["--no-build-isolation"], &[])?;
    }
    Ok(())
}
