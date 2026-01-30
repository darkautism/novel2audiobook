#[cfg(target_arch = "wasm32")]
use crate::core::io::Storage;
#[cfg(target_arch = "wasm32")]
use anyhow::{anyhow, Result};
#[cfg(target_arch = "wasm32")]
use async_trait::async_trait;
#[cfg(target_arch = "wasm32")]
use idb::{Factory, ObjectStoreParams, TransactionMode};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;

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
        let factory = Factory::new()?;
        let mut open_request = factory.open(DB_NAME, 1).map_err(|e| anyhow!("Failed to open DB: {:?}", e))?;

        open_request.on_upgrade_needed(|event| {
            let db = event.database().unwrap();
            if db.object_store_names().find(|n| n == STORE_NAME).is_none() {
                db.create_object_store(STORE_NAME, ObjectStoreParams::new()).unwrap();
            }
        });

        let db = open_request.await.map_err(|e| anyhow!("Failed to await DB open: {:?}", e))?;
        Ok(Self { db })
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait]
impl Storage for WebStorage {
    async fn read(&self, path: &str) -> Result<Vec<u8>> {
        let transaction = self.db.transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| anyhow!("Tx error: {:?}", e))?;
        let store = transaction.object_store(STORE_NAME).map_err(|e| anyhow!("Store error: {:?}", e))?;
        
        let value = store.get(JsValue::from_str(path)).await
            .map_err(|e| anyhow!("Get error: {:?}", e))?;
        
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
        
        store.put(&array, Some(&JsValue::from_str(path))).await
            .map_err(|e| anyhow!("Put error: {:?}", e))?;
            
        transaction.commit().await.map_err(|e| anyhow!("Commit error: {:?}", e))?;
        Ok(())
    }

    async fn delete(&self, path: &str) -> Result<()> {
        let transaction = self.db.transaction(&[STORE_NAME], TransactionMode::ReadWrite)
            .map_err(|e| anyhow!("Tx error: {:?}", e))?;
        let store = transaction.object_store(STORE_NAME).map_err(|e| anyhow!("Store error: {:?}", e))?;
        
        store.delete(JsValue::from_str(path)).await
            .map_err(|e| anyhow!("Delete error: {:?}", e))?;
            
        transaction.commit().await.map_err(|e| anyhow!("Commit error: {:?}", e))?;
        Ok(())
    }

    async fn exists(&self, path: &str) -> Result<bool> {
        let transaction = self.db.transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| anyhow!("Tx error: {:?}", e))?;
        let store = transaction.object_store(STORE_NAME).map_err(|e| anyhow!("Store error: {:?}", e))?;
        
        let key = store.get_key(JsValue::from_str(path)).await
            .map_err(|e| anyhow!("GetKey error: {:?}", e))?;
        
        Ok(key.is_some())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let transaction = self.db.transaction(&[STORE_NAME], TransactionMode::ReadOnly)
            .map_err(|e| anyhow!("Tx error: {:?}", e))?;
        let store = transaction.object_store(STORE_NAME).map_err(|e| anyhow!("Store error: {:?}", e))?;
        
        let keys = store.get_all_keys(None).await
            .map_err(|e| anyhow!("GetAllKeys error: {:?}", e))?;
        
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
        
        store.clear().await.map_err(|e| anyhow!("Clear error: {:?}", e))?;
        
        transaction.commit().await.map_err(|e| anyhow!("Commit error: {:?}", e))?;
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
