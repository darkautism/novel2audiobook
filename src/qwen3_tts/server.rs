use anyhow::{Context, Result};
use log::{info, warn};
use std::env;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio as StdStdio};
use std::sync::Arc;
use tokio::process::{Command, Child};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::RwLock;

// 定義 Python 版本常數
const PYTHON_RELEASE_TAG: &str = "20260114";
const PYTHON_VERSION_TAG: &str = "3.10.19";

#[derive(Clone)]
pub struct Qwen3Server {
    child: Arc<RwLock<Option<Child>>>,
}

impl Qwen3Server {
    pub fn new() -> Self {
        Self {
            child: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn start(&self) -> Result<()> {
        let runtime_dir = Path::new(".").join(".runtime");

        if !runtime_dir.exists() {
            info!(">>> 偵測到環境未建立，正在下載 Portable Python...");
            setup_python_env(&runtime_dir).await?;
        } else {
            info!(">>> 環境已存在：{:?}", runtime_dir);
        }

        let python_bin = get_python_executable(&runtime_dir);
        info!(">>> 使用 Python 直譯器：{:?}", python_bin);

        // 安裝依賴 (Blocking)
        let python_bin_clone = python_bin.clone();
        tokio::task::spawn_blocking(move || {
            install_dependencies(&python_bin_clone)
        }).await??;

        info!(">>> 正在啟動 Qwen-TTS Demo...");
        let scripts_dir = python_bin.parent().unwrap().join("Scripts");
        let demo = scripts_dir.join("qwen-tts-demo.exe");

        let mut cmd = Command::new(&demo);
        cmd.arg("Qwen/Qwen3-TTS-12Hz-1.7B-Base")
           .arg("--ip")
           .arg("0.0.0.0")
           .arg("--port")
           .arg("8000")
           .env("PYTHONUNBUFFERED", "1")
           .stdout(StdStdio::piped())
           .stderr(StdStdio::piped())
           .kill_on_drop(true);

        let mut child = cmd.spawn().context("執行 qwen-tts 失敗")?;

        let stdout = child.stdout.take().expect("Failed to open stdout");
        let stderr = child.stderr.take().expect("Failed to open stderr");

        // Save child handle
        {
            let mut lock = self.child.write().await;
            *lock = Some(child);
        }

        // Wait for readiness
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut tx = Some(tx);

        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                info!("[Qwen3-Server] {}", line);
                if line.contains("Running on local URL:") {
                    if let Some(t) = tx.take() {
                        let _ = t.send(());
                    }
                }
            }
        });

        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                warn!("[Qwen3-Server Error] {}", line);
            }
        });

        info!(">>> 等待 Qwen-TTS Server 啟動...");
        rx.await.context("Server failed to start or closed unexpectedly")?;
        info!(">>> Qwen-TTS Server 已啟動！");

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn stop(&self) {
         let mut lock = self.child.write().await;
         if let Some(mut child) = lock.take() {
             let _ = child.kill().await;
         }
    }
}

// --- Helper Functions ---

fn install_dependencies(python_bin: &Path) -> Result<()> {
    // 4.1 安裝 PyTorch (CUDA 12.4)
    info!(">>> [1/3] 檢查並安裝 PyTorch 2.6.0 (CUDA 12.4)...");
    install_torch_cuda_124(python_bin)?;

    // 4.2 安裝 Flash Attention (Windows 特規)
    info!(">>> [2/3] 檢查並安裝 Flash Attention...");
    install_flash_attn_conditional(python_bin)?;

    // 4.3 安裝 Qwen-TTS
    info!(">>> [3/3] 檢查並安裝 Qwen-TTS...");
    install_package(python_bin, "qwen-tts", &[], &[])?;

    info!(">>> ✅ 完成！環境已準備就緒。");
    Ok(())
}

fn get_download_url() -> Result<String> {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    let base_url = format!(
        "https://github.com/astral-sh/python-build-standalone/releases/download/{}",
        PYTHON_RELEASE_TAG
    );

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
    info!(">>> 正在解壓縮...");
    let tar = flate2::read::GzDecoder::new(Cursor::new(downloaded_data));
    let mut archive = tar::Archive::new(tar);

    archive.unpack(target_dir.parent().unwrap_or(Path::new(".")))?;

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

fn is_package_installed(python_bin: &Path, package_name: &str) -> bool {
    let status = StdCommand::new(python_bin)
        .arg("-I")
        .arg("-m")
        .arg("pip")
        .arg("show")
        .arg(package_name)
        .stdout(StdStdio::null())
        .stderr(StdStdio::null())
        .status();

    match status {
        Ok(s) => s.success(),
        Err(_) => false,
    }
}

fn install_package(
    python_bin: &Path,
    package_name: &str,
    extra_args: &[&str],
    env_vars: &[(&str, &str)],
) -> Result<()> {
    let is_url_or_file = package_name.contains("http") || package_name.ends_with(".whl");

    if !is_url_or_file && is_package_installed(python_bin, package_name) {
        info!("套件 '{}' 已存在，跳過安裝。", package_name);
        return Ok(());
    }

    info!("正在安裝 '{}'...", package_name);

    let mut cmd = StdCommand::new(python_bin);
    cmd.arg("-I")
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg(package_name);

    for (key, val) in env_vars {
        cmd.env(*key, *val);
    }

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

fn install_torch_cuda_124(python_bin: &Path) -> Result<()> {
    if is_package_installed(python_bin, "torch") {
        info!("Torch 已安裝，跳過。");
        return Ok(());
    }

    info!("正在下載並安裝 PyTorch 2.6.0 (CUDA 12.4)...");
    let status = StdCommand::new(python_bin)
        .arg("-I")
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("torch==2.6.0")
        .arg("torchvision")
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

fn install_flash_attn_conditional(python_bin: &Path) -> Result<()> {
    if is_package_installed(python_bin, "flash-attn") {
        info!("Flash Attention 已安裝，跳過。");
        return Ok(());
    }

    if cfg!(windows) {
        info!(">>> 偵測到 Windows，使用自定義 Wheel 安裝 Flash Attention...");
        let wheel_url = "https://github.com/kingbri1/flash-attention/releases/download/v2.8.2/flash_attn-2.8.2+cu124torch2.6.0cxx11abiFALSE-cp310-cp310-win_amd64.whl";
        install_package(python_bin, wheel_url, &["--no-deps"], &[])?;
    } else {
        info!(">>> 偵測到 Linux/Mac，使用標準 pip 安裝 Flash Attention...");
        install_package(python_bin, "flash-attn", &["--no-build-isolation"], &[])?;
    }
    Ok(())
}
