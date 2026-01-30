use async_trait::async_trait;
use anyhow::Result;

#[async_trait]
pub trait Storage: Send + Sync {
    async fn read(&self, path: &str) -> Result<Vec<u8>>;
    async fn write(&self, path: &str, content: &[u8]) -> Result<()>;
    async fn delete(&self, path: &str) -> Result<()>;
    async fn exists(&self, path: &str) -> Result<bool>;
    async fn list(&self, prefix: &str) -> Result<Vec<String>>;
    async fn clear_all(&self) -> Result<()>;
    async fn usage(&self) -> Result<u64>;
}

#[cfg(not(target_arch = "wasm32"))]
pub struct NativeStorage;

#[cfg(not(target_arch = "wasm32"))]
impl NativeStorage {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait]
impl Storage for NativeStorage {
    async fn read(&self, path: &str) -> Result<Vec<u8>> {
        Ok(tokio::fs::read(path).await?)
    }
    
    async fn write(&self, path: &str, content: &[u8]) -> Result<()> {
        if let Some(parent) = std::path::Path::new(path).parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        tokio::fs::write(path, content).await?;
        Ok(())
    }
    
    async fn delete(&self, path: &str) -> Result<()> {
        if tokio::fs::try_exists(path).await? {
            if std::path::Path::new(path).is_dir() {
                 tokio::fs::remove_dir_all(path).await?;
            } else {
                 tokio::fs::remove_file(path).await?;
            }
        }
        Ok(())
    }
    
    async fn exists(&self, path: &str) -> Result<bool> {
        Ok(tokio::fs::try_exists(path).await?)
    }
    
    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let path = std::path::Path::new(prefix);
        let mut entries = Vec::new();
        
        if path.exists() {
            if path.is_dir() {
                let mut dir = tokio::fs::read_dir(path).await?;
                while let Some(entry) = dir.next_entry().await? {
                    entries.push(entry.path().to_string_lossy().to_string());
                }
            } else {
                 entries.push(prefix.to_string());
            }
        }
        
        Ok(entries)
    }
    
    async fn clear_all(&self) -> Result<()> {
        log::warn!("clear_all called on NativeStorage - ignoring for safety");
        Ok(())
    }
    
    async fn usage(&self) -> Result<u64> {
        Ok(0)
    }
}
