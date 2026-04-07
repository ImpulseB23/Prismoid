mod message;

use tauri::Manager;
use tracing_subscriber::EnvFilter;

#[tauri::command]
fn get_platform() -> &'static str {
    std::env::consts::OS
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("prismoid=debug".parse().unwrap()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![get_platform])
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();
            tracing::info!("prismoid starting, window: {}", window.label());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("failed to run prismoid");
}
