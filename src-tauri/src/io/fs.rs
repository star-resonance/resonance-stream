use chrono::Local;
use resonance_core::history;
use std::fs;
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub async fn open_app_data_folder(app: tauri::AppHandle) -> Result<(), String> {
    // 1. Resolve the specific AppData/Roaming folder for this app
    let app_dir = crate::app_dirs::data(&app).map_err(|e| e.to_string())?;

    // 2. CRITICAL: Ensure the directory exists.
    // If Explorer is called on a non-existent path, it defaults to 'Documents'.
    if !app_dir.exists() {
        fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;
    }

    // 3. Open the folder using the system file explorer
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(app_dir.to_str().unwrap()) // Pass the absolute AppData path
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(app_dir.to_str().unwrap())
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Writes the backend's chat log, in arrival order, to `chat_export_<time>.txt` in the app's data
/// folder; the answer is the file's path. async: not on the main thread.
#[tauri::command]
pub async fn export_chat_log(app: tauri::AppHandle) -> Result<String, String> {
    // 1. Get the AppData directory
    let app_dir = crate::app_dirs::data(&app).map_err(|e| e.to_string())?;

    if !app_dir.exists() {
        fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;
    }

    // 2. A unique filename from the current time
    let now = Local::now();
    let file_path = app_dir.join(format!("chat_export_{}.txt", now.format("%Y%m%d_%H%M%S")));

    // 3. The text; a timestamp out of range (a raw value from the wire) must not end the export.
    let text = history::export_text(
        &now.format("%Y-%m-%d %H:%M:%S").to_string(),
        &crate::commands::chat_history(&app),
        export_time,
    );
    fs::write(&file_path, text).map_err(|e| e.to_string())?;

    Ok(file_path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn open_browser(app: tauri::AppHandle, url: String) -> Result<(), String> {
    resonance_core::download::check_open_url(&url)?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

/// A chat timestamp as local time; `????-??-?? ??:??:??` when it is not a date.
fn export_time(timestamp: u64) -> String {
    resonance_core::crash_log::unix_to_utc(timestamp)
        .map(|t| {
            t.with_timezone(&Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "????-??-?? ??:??:??".to_string())
}

#[cfg(test)]
mod tests {
    use super::export_time;

    #[test]
    fn an_out_of_range_timestamp_does_not_stop_the_export() {
        assert_eq!(export_time(u64::MAX), "????-??-?? ??:??:??");
        assert_eq!(export_time(1_700_000_000).len(), 19);
    }
}
