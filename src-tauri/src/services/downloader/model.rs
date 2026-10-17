use super::{FolderStatus, ProgressPayload};
use crate::{inject_system_message, SystemLogLevel};
use resonance_core::download::{replace_file, sha256_file};
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub const MODEL_FOLDER: &str = "translation-model";
pub const MODEL_FILENAME: &str = "model.gguf";

fn get_model_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let base_models_dir = crate::app_dirs::data(app)
        .map_err(|e| e.to_string())?
        .join("models");

    let new_dir = base_models_dir.join(MODEL_FOLDER); // "translation-model"
    let old_dir = base_models_dir.join("Qwen3-Blue-Protocol-Translator-JA-KO");

    // MIGRATION: If the old folder exists but the new one doesn't, rename it!
    if old_dir.exists() && !new_dir.exists() {
        let _ = fs::rename(&old_dir, &new_dir);

        // Also rename the specific .gguf file to the generic model.gguf
        let old_file = new_dir.join("qwen3-4b-blueprotocol-ja2ko-q4_k_m.gguf");
        let new_file = new_dir.join(MODEL_FILENAME);
        if old_file.exists() {
            let _ = fs::rename(&old_file, &new_file);
        }
    }

    Ok(new_dir)
}

pub fn get_model_path(app: &tauri::AppHandle) -> PathBuf {
    crate::app_dirs::data(app)
        .expect("Failed to resolve AppData directory")
        .join("models")
        .join(MODEL_FOLDER)
        .join(MODEL_FILENAME)
}

#[tauri::command]
pub async fn check_model_status(app: tauri::AppHandle) -> Result<FolderStatus, String> {
    let model_path = get_model_path(&app);

    Ok(FolderStatus {
        exists: model_path.exists(),
        path: model_path.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub async fn download_model(app: AppHandle) -> Result<(), String> {
    // The url, version and hash come from the signed metadata, checked now -- never from the caller.
    let verified = super::gist::verified_metadata_fresh(&app).await?;
    let resonance_core::signed_metadata::ModelSource {
        url: download_url,
        version,
        sha256: expected_hash,
    } = resonance_core::signed_metadata::model_source(&verified)?;
    let model_dir = get_model_dir(&app)?;
    fs::create_dir_all(&model_dir).map_err(|e| e.to_string())?;
    let dest_path = model_dir.join(MODEL_FILENAME);

    // 1. FAST PATH: the installed model may already be this exact file.
    if dest_path.exists() {
        inject_system_message(
            &app,
            SystemLogLevel::Info,
            "Model",
            "Checking existing model integrity before downloading...",
        );
        let path = dest_path.clone();
        let local_hash = tauri::async_runtime::spawn_blocking(move || sha256_file(&path))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        if local_hash.eq_ignore_ascii_case(&expected_hash) {
            inject_system_message(
                &app,
                SystemLogLevel::Success,
                "Model",
                "Existing model is a perfect match! Skipping download.",
            );
            let _ = app.emit(
                "download-progress",
                ProgressPayload {
                    current_file: "로컬 AI 모델 확인 완료 (Skipped download)".to_string(),
                    percent: 100,
                    total_percent: 100,
                },
            );
            crate::config::modify_metadata(&app, |metadata| {
                metadata.current_model_version = version;
                true
            });
            return Ok(());
        }
        inject_system_message(
            &app,
            SystemLogLevel::Warning,
            "Model",
            "Existing model is outdated or corrupted. Starting fresh download.",
        );
    }

    inject_system_message(
        &app,
        SystemLogLevel::Info,
        "Model",
        format!("Download Model version {}", version),
    );

    // 2. DOWNLOAD + VERIFY: hashed while streaming; the current model stays
    // in place until the new one is complete and matches.
    let staged = model_dir.join(format!("{}.new", MODEL_FILENAME));
    super::fetch::download_file(
        &app,
        &download_url,
        &staged,
        "AI 모델 다운로드 중...",
        Some(&expected_hash),
    )
    .await?;

    // llama-server maps the current model file, and Windows will not replace
    // a mapped file: stop the translator for the swap, then bring it back.
    let state = app.state::<crate::AppState>();
    let was_running = state.services.stop_translator();
    if was_running {
        crate::services::translator::server_manager::kill_orphaned_servers(&app);
    }
    let (from, to) = (staged.clone(), dest_path.clone());
    let replaced = tauri::async_runtime::spawn_blocking(move || {
        replace_file(&from, &to, 20, Duration::from_millis(250))
    })
    .await
    .map_err(|e| e.to_string())?;
    if was_running {
        state.services.start_translator(&app);
    }
    if let Err(e) = replaced {
        let _ = fs::remove_file(&staged);
        return Err(format!("Could not replace the model file: {}", e));
    }

    // Remove leftovers of older models (several GB each)
    if let Ok(entries) = fs::read_dir(&model_dir) {
        for entry in entries.flatten() {
            if entry.file_name() != MODEL_FILENAME {
                let path = entry.path();
                let _ = if path.is_dir() {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
            }
        }
    }

    let _ = app.emit(
        "download-progress",
        ProgressPayload {
            current_file: "완료".into(),
            percent: 100,
            total_percent: 100,
        },
    );

    // 3. Commit the new version to metadata so the update checker knows we have it
    crate::config::modify_metadata(&app, |metadata| {
        metadata.current_model_version = version;
        true
    });

    Ok(())
}
