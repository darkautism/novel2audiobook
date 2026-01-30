pub mod core;
pub mod services;
pub mod utils;
#[cfg(target_arch = "wasm32")]
pub mod ui;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Debug).unwrap_or(());
    
    leptos::mount_to_body(|| {
        use crate::ui::App;
        view! { <App/> }
    });
}
