use leptos::*;
use crate::core::io::Storage;
#[cfg(target_arch = "wasm32")]
use crate::core::io::WebStorage;
#[cfg(not(target_arch = "wasm32"))]
use crate::core::io::NativeStorage;
use std::sync::Arc;

#[component]
pub fn App() -> impl IntoView {
    // Initialize storage
    let (storage, set_storage) = create_signal(None::<Result<Arc<dyn Storage>, String>>);

    create_effect(move |_| {
        spawn_local(async move {
            #[cfg(target_arch = "wasm32")]
            let res = WebStorage::new().await
                .map(|s| Arc::new(s) as Arc<dyn Storage>)
                .map_err(|e| e.to_string());

            #[cfg(not(target_arch = "wasm32"))]
            let res = Ok(Arc::new(NativeStorage::new()) as Arc<dyn Storage>)
                .map_err(|e: anyhow::Error| e.to_string());

            set_storage.set(Some(res));
        });
    });

    view! {
        <div class="app-container">
            <h1>"Novel2Audiobook"</h1>
            {move || match storage.get() {
                Some(Ok(s)) => view! { <StorageControl storage=s/> }.into_view(),
                Some(Err(e)) => view! { <p>"Error loading storage: " {e}</p> }.into_view(),
                None => view! { <p>"Loading Storage..."</p> }.into_view()
            }}
        </div>
    }
}

#[component]
pub fn StorageControl(storage: Arc<dyn Storage>) -> impl IntoView {
    let (usage, set_usage) = create_signal(0u64);
    let storage_clone = storage.clone();
    let storage_for_usage = storage.clone();
    
    let fetch_usage = move || {
        let storage = storage_for_usage.clone();
        spawn_local(async move {
            if let Ok(u) = storage.usage().await {
                set_usage.set(u);
            }
        });
    };
    
    // Initial fetch
    let fetch_usage_effect = fetch_usage.clone();
    create_effect(move |_| {
        fetch_usage_effect();
    });

    let on_clear = move |_| {
        let storage = storage_clone.clone();
        let fetch = fetch_usage.clone();
        spawn_local(async move {
             if let Err(e) = storage.clear_all().await {
                 leptos::logging::error!("Failed to clear: {:?}", e);
             } else {
                 leptos::logging::log!("Cleared!");
                 fetch();
             }
        });
    };

    view! {
        <div style="border: 1px solid #ccc; padding: 10px; margin: 10px;">
            <h3>"Storage Management"</h3>
            <p>"Current Usage: " {move || usage.get() / 1024} " KB"</p>
            <button on:click=on_clear>"Clear Cache"</button>
        </div>
    }
}
