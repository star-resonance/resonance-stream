use chrono::Local;
use resonance_core::history;
use std::fs;
use std::io::Write;
use tauri_plugin_opener::OpenerExt;

// Adjust this import path if ExportMessage is located elsewhere!
use crate::protocol::types::ExportMessage;

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

#[tauri::command]
pub async fn export_chat_log(
    app: tauri::AppHandle,
    logs: Vec<ExportMessage>,
) -> Result<String, String> {
    // 1. Get the AppData directory
    let app_dir = crate::app_dirs::data(&app).map_err(|e| e.to_string())?;

    if !app_dir.exists() {
        fs::create_dir_all(&app_dir).map_err(|e| e.to_string())?;
    }

    // 2. Create a unique filename based on the current time
    let timestamp_now = Local::now().format("%Y%m%d_%H%M%S");
    let file_path = app_dir.join(format!("chat_export_{}.txt", timestamp_now));

    // 3. Open the file for writing
    let mut file = fs::File::create(&file_path).map_err(|e| e.to_string())?;

    // 4. Write the header
    let now = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    write!(file, "{}", history::export_header(&now)).map_err(|e| e.to_string())?;

    // 5. Format and write each message
    for log in logs {
        // The timestamp is a raw value from the wire: one out of range must not end the export.
        let time_str = export_time(log.timestamp);
        let line = history::export_line(
            &time_str,
            &log.channel,
            &log.nickname,
            &log.message,
            log.translated.as_deref(),
        );
        writeln!(file, "{}", line).map_err(|e| e.to_string())?;
    }

    // Return the path so we could theoretically show it to the user
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
