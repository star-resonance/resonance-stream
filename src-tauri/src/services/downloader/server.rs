use crate::{FolderStatus, ProgressPayload};
use resonance_core::download::{part_path, publish_dir};
use std::fs;
use tauri::{AppHandle, Emitter};

pub const AI_SERVER_FOLDER: &str = "ai-server";
pub const AI_SERVER_ZIP_URL: &str = "https://github.com/enjay27/resonance-stream/releases/download/v0.2.0/llama-b8157-bin-win-vulkan-x64.zip";
/// SHA-256 of the zip above (computed 2026-09-29). Change both together.
const AI_SERVER_ZIP_SHA256: &str =
    "8144cf0a765f6a69c8bf62acb70e9d224bcfef28b3ffd2260fe4cf5b8bde20dc";
pub const AI_SERVER_FILENAME: &str = "llama-server.exe";

#[tauri::command]
pub async fn check_ai_server_status(app: tauri::AppHandle) -> Result<FolderStatus, String> {
    // Check exactly one path for the .gguf file
    let model_path = crate::app_dirs::data(&app)
        .map_err(|e| e.to_string())?
        .join("bin")
        .join(AI_SERVER_FOLDER)
        .join(AI_SERVER_FILENAME);

    Ok(FolderStatus {
        exists: model_path.exists(),
        path: model_path.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn download_ai_server(app: AppHandle) -> Result<(), String> {
    let ai_server_dir = crate::app_dirs::data(&app)
        .map_err(|e| e.to_string())?
        .join("bin")
        .join(AI_SERVER_FOLDER);

    // Skip if already downloaded and extracted. The folder only ever appears whole (below), so
    // the exe being there means its DLLs are too.
    if ai_server_dir.join(AI_SERVER_FILENAME).exists() {
        return Ok(());
    }

    // Everything is built in `ai-server.part` and moved into place at the end: a kill or a full
    // disk half-way leaves a folder nobody looks at, not an "installed" server without its DLLs.
    let staging = part_path(&ai_server_dir);
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;

    let result = stage_ai_server(&app, &staging).await;
    if let Err(e) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }
    publish_dir(&staging, &ai_server_dir).map_err(|e| {
        let _ = fs::remove_dir_all(&staging);
        format!("Could not install the AI engine: {e}")
    })
}

/// Downloads the zip into `staging` and extracts it there (flat), then deletes the zip.
async fn stage_ai_server(app: &AppHandle, staging: &std::path::Path) -> Result<(), String> {
    let zip_path = staging.join("server_temp.zip");

    // 1. Download the ZIP file (Streaming, verified against the pinned hash)
    super::fetch::download_file(
        app,
        AI_SERVER_ZIP_URL,
        &zip_path,
        "AI 엔진 다운로드 중...",
        Some(AI_SERVER_ZIP_SHA256),
    )
    .await?;

    // 2. Extract the ZIP file
    let _ = app.emit(
        "download-progress",
        ProgressPayload {
            current_file: "압축 해제 중...".to_string(),
            percent: 100,
            total_percent: 100,
        },
    );

    let zip_file = fs::File::open(&zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(zip_file).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = match file.enclosed_name() {
            Some(path) => staging.join(path.file_name().unwrap_or(path.as_os_str())), // Flattens the folder structure
            None => continue,
        };

        if file.is_dir() {
            continue; // Skip directories
        }

        let mut outfile = fs::File::create(&outpath).map_err(|e| e.to_string())?;
        std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
    }

    // 3. Clean up the temp zip file
    let _ = fs::remove_file(zip_path);

    Ok(())
}
