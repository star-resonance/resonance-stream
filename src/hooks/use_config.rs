use crate::tauri_bridge::call;
use crate::ui_types::{AppConfig, SystemLogLevel};
use crate::utils::add_system_log;
use resonance_types::ipc::SaveConfig;

pub async fn save_app_config(config: AppConfig) {
    // The backend keeps the setting for this run even when the file cannot be written; it
    // answers with why, and the user is told it will not survive a restart.
    if let Err(why) = call(&SaveConfig { config }).await {
        add_system_log(SystemLogLevel::Error, "Settings", &why);
    }
}
