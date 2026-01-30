use async_trait::async_trait;
use anyhow::Result;

#[cfg(target_arch = "wasm32")]
pub trait StorageBounds {}
#[cfg(target_arch = "wasm32")]
impl<T> StorageBounds for T {}

#[cfg(not(target_arch = "wasm32"))]
pub trait StorageBounds: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync> StorageBounds for T {}

#[cfg_attr(target_arch = "wasm32", async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait)]
pub trait Storage: StorageBounds {
    async fn read(&self, path: &str) -> Result<Vec<u8>>;
    async fn write(&self, path: &str, content: &[u8]) -> Result<()>;
    async fn delete(&self, path: &str) -> Result<()>;
    async fn exists(&self, path: &str) -> Result<bool>;
    async fn list(&self, prefix: &str) -> Result<Vec<String>>;
    async fn clear_all(&self) -> Result<()>;
    async fn usage(&self) -> Result<u64>;
}

// --- Native Implementation ---

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

// --- Web Implementation ---

#[cfg(target_arch = "wasm32")]
use idb::{Factory, ObjectStoreParams, TransactionMode, DatabaseEvent};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;
#[cfg(target_arch = "wasm32")]
use anyhow::anyhow;

#[cfg(target_arch = "wasm32")]
const DB_NAME: &str = "novel2audiobook_db";
#[cfg(target_arch = "wasm32")]
const STORE_NAME: &str = "files";

#[cfg(target_arch = "wasm32")]
pub struct WebStorage {
    db: idb::Database,
}

#[cfg(target_arch = "wasm32")]
impl WebStorage {
    pub async fn new() -> Result<Self> {
        let factory = Factory::new().map_err(|e| anyhow!("Failed to create factory: {:?}", e))?;
        let mut open_request = factory.open(DB_NAME, Some(1)).map_err(|e| anyhow!("Failed to open DB: {:?}", e))?;

        open_request.on_upgrade_needed(|event| {
            let db = event.database().unwrap();
            if db.store_names().iter().find(|n| n.as_str() == STORE_NAME).is_none() {
                db.create_object_store(STORE_NAME, ObjectStoreParams::new()).unwrap();
            }
        });

        let db = open_request.await.map_err(|e| anyhow!("Failed to await DB open: {:?}", e))?;
        Ok(Self { db })
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait(?Send)]
impl Storage for WebStorage {
    async fn read(&self, path: &str) -> Result<Vec<u8>> {
        let transaction = self.db.transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| anyhow!("Tx error: {:?}", e))?;
        let store = transaction.object_store(STORE_NAME).map_err(|e| anyhow!("Store error: {:?}", e))?;

        let value = store.get(JsValue::from_str(path))
            .map_err(|e| anyhow!("Get error: {:?}", e))?
            .await
            .map_err(|e| anyhow!("Get await error: {:?}", e))?;

        match value {
            Some(v) => {
                let array = js_sys::Uint8Array::new(&v);
                Ok(array.to_vec())
            },
            None => Err(anyhow!("File not found: {}", path)),
        }
    }

    async fn write(&self, path: &str, content: &[u8]) -> Result<()> {
        let transaction = self.db.transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| anyhow!("Tx error: {:?}", e))?;
        let store = transaction.object_store(STORE_NAME).map_err(|e| anyhow!("Store error: {:?}", e))?;

        let array = js_sys::Uint8Array::from(content);

        store.put(&array, Some(&JsValue::from_str(path)))
            .map_err(|e| anyhow!("Put error: {:?}", e))?
            .await
            .map_err(|e| anyhow!("Put await error: {:?}", e))?;

        transaction.commit()
            .map_err(|e| anyhow!("Commit error: {:?}", e))?
            .await
            .map_err(|e| anyhow!("Commit await error: {:?}", e))?;
        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let transaction = self.db.transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| anyhow!("Tx error: {:?}", e))?;
        let store = transaction.object_store(STORE_NAME).map_err(|e| anyhow!("Store error: {:?}", e))?;

        store.delete(JsValue::from_str(path))
            .map_err(|e| anyhow!("Delete error: {:?}", e))?
            .await
            .map_err(|e| anyhow!("Delete await error: {:?}", e))?;

        transaction.commit()
            .map_err(|e| anyhow!("Commit error: {:?}", e))?
            .await
            .map_err(|e| anyhow!("Commit await error: {:?}", e))?;
        Ok(())
    }

    async fn exists(&self, path: &str) -> Result<bool> {
        let transaction = self.db.transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| anyhow!("Tx error: {:?}", e))?;
        let store = transaction.object_store(STORE_NAME).map_err(|e| anyhow!("Store error: {:?}", e))?;

        let key = store.get_key(JsValue::from_str(path))
            .map_err(|e| anyhow!("GetKey error: {:?}", e))?
            .await
            .map_err(|e| anyhow!("GetKey await error: {:?}", e))?;

        Ok(key.is_some())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let transaction = self.db.transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| anyhow!("Tx error: {:?}", e))?;
        let store = transaction.object_store(STORE_NAME).map_err(|e| anyhow!("Store error: {:?}", e))?;

        let keys = store.get_all_keys(None, None)
            .map_err(|e| anyhow!("GetAllKeys error: {:?}", e))?
            .await
            .map_err(|e| anyhow!("GetAllKeys await error: {:?}", e))?;

        let mut results = Vec::new();
        for key in keys {
            if let Some(k) = key.as_string() {
                if k.starts_with(prefix) {
                    results.push(k);
                }
            }
        }
        Ok(results)
    }

    async fn clear_all(&self) -> Result<()> {
        let transaction = self.db.transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| anyhow!("Tx error: {:?}", e))?;
        let store = transaction.object_store(STORE_NAME).map_err(|e| anyhow!("Store error: {:?}", e))?;

        store.clear()
            .map_err(|e| anyhow!("Clear error: {:?}", e))?
            .await
            .map_err(|e| anyhow!("Clear await error: {:?}", e))?;

        transaction.commit()
            .map_err(|e| anyhow!("Commit error: {:?}", e))?
            .await
            .map_err(|e| anyhow!("Commit await error: {:?}", e))?;
        Ok(())
    }

    async fn usage(&self) -> Result<u64> {
        if let Some(window) = web_sys::window() {
             let navigator = window.navigator();
             let storage = navigator.storage();

             let promise = storage.estimate().map_err(|e| anyhow!("Estimate error: {:?}", e))?;
             let result = wasm_bindgen_futures::JsFuture::from(promise).await
                 .map_err(|e| anyhow!("JsFuture error: {:?}", e))?;

             let usage = js_sys::Reflect::get(&result, &JsValue::from_str("usage"))
                 .map_err(|e| anyhow!("Reflect error: {:?}", e))?;

             if let Some(u) = usage.as_f64() {
                 return Ok(u as u64);
             }
        }
        Ok(0)
    }
}
