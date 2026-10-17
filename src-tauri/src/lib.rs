use parking_lot::{Mutex, RwLock};
use resonance_core::history::ChatHistory;
use resonance_types::PopupKind;
use std::collections::VecDeque;
use std::sync::Arc;
use tauri::Manager;

pub mod app_dirs;
pub mod bridge;
pub mod commands;
pub mod config;
pub mod events;
pub mod io;
pub mod logging;
pub mod panic_hook;
pub mod protocol;
pub mod services;
pub mod shortcut;
pub mod test_env;
pub mod tray;
pub mod window;

pub use commands::*;
pub use config::*;
pub use events::*;
pub use io::*;
pub use protocol::types::*;
pub use services::downloader::*;
pub use services::sniffer::*;
pub use services::translator::*;
pub use shortcut::*;
pub use tray::*;
pub use window::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // The test flags first: they may move the data folders and name the log file.
    test_env::init();
    logging::init_logger(test_env::log_file());
    panic_hook::install();

    // A popup is placed by `show_popup`, hidden, just before it is shown: the
    // plugin restoring it at creation would show it first at the default place.
    let window_state_plugin = PopupKind::ALL
        .into_iter()
        .fold(
            tauri_plugin_window_state::Builder::default(),
            |plugin, kind| plugin.skip_initial_state(kind.label()),
        )
        .build();

    let mut builder = tauri::Builder::default()
        .on_window_event(window::on_window_event)
        .setup(|app| {
            let handle = app.handle().clone();
            if let Ok(data_dir) = app_dirs::data(&handle) {
                panic_hook::set_log_dir(&data_dir);
            }
            window::prewarm_popups(handle.clone());
            // --- STATE FIRST: everything below logs through it and reads its config ---
            let mut config = read_config_file(&handle);
            config.init_done = test_env::init_done_for_run(config.init_done);
            let dictionary = resonance_core::text::Dictionary::load(&dictionary_path(&handle));
            let load_notice = take_load_notice();
            app.manage(AppState {
                config: RwLock::new(config.clone()),
                config_lock: Mutex::new(()),
                metadata_lock: Mutex::new(()),
                chat_history: Mutex::new(ChatHistory::new(
                    resonance_core::history::ChannelLimits::new(&config.tab_limits),
                )),
                system_history: Mutex::new(VecDeque::with_capacity(200)),
                next_pid: 1.into(),
                nickname_cache: Mutex::new(std::collections::HashMap::new()),
                dictionary: RwLock::new(Arc::new(dictionary)),
                services: Default::default(),
                translation_ledger: Mutex::new(Default::default()),
                data_factory_tx: Mutex::new(None),
                service_states: Mutex::new(Default::default()),
                updates: Default::default(),
                shortcuts: Mutex::new(shortcut::GlobalShortcuts {
                    tab_modifier: config.tab_switch_modifier,
                    tab_key: config.tab_switch_key.clone(),
                    favorites: config.favorite_messages.clone(),
                }),
            });
            let state = app.state::<AppState>();

            // Before anything that emits, so the bridge hears all of it: the translator's first states, the first system
            // messages (it queues until connected). Its commands wait for `mark_ready` below.
            bridge::start(&handle);

            // What the start-up read found wrong with config.json (the state did not exist to log it).
            if let Some(notice) = load_notice {
                inject_system_message(&handle, SystemLogLevel::Error, "Settings", notice);
            }

            // Old daily logs past the retention setting go before the reload.
            crate::io::prune_chat_logs(&handle);

            // Chat saved by earlier runs (daily chat logs), newest last; new
            // pids continue after them so the list stays in order.
            let mut restored = resonance_core::history::load_recent(
                &crate::io::chat_logs_dir(&handle),
                &resonance_core::history::ChannelLimits::new(&config.tab_limits),
            );
            // Each row has the blocked flag it was saved with; the block list may have changed since.
            resonance_core::history::apply_block_list(&mut restored, |uid| {
                config.blocked_users.contains_key(&uid)
            });
            state.next_pid.fetch_max(
                restored.len() as u64 + 1,
                std::sync::atomic::Ordering::SeqCst,
            );
            {
                let mut history = state.chat_history.lock();
                for message in restored {
                    history.push(message);
                }
            }

            inject_system_message(
                &handle,
                SystemLogLevel::Info,
                "Backend",
                "Initializing Resonance Stream...",
            );

            let is_admin = is_elevated::is_elevated();
            inject_system_message(
                &handle,
                SystemLogLevel::Info,
                "Backend",
                format!("Admin Privileges: {}", is_admin),
            );

            if !is_admin {
                inject_system_message(
                    &handle,
                    SystemLogLevel::Warning,
                    "Backend",
                    "Sniffer may fail without Admin rights.",
                );
            }

            apply_global_shortcuts(&handle);

            // --- START AI IF NEEDED ---
            if config.use_translation {
                state.services.start_translator(&handle);
            }

            // --- START THE CHAT ARCHIVE (each tab decides what it takes) ---
            *state.data_factory_tx.lock() =
                Some(crate::io::start_data_factory_worker(handle.clone()));

            crate::tray::setup_tray(app)?;

            test_env::mark_ready(&handle);
            crate::services::sniffer::replay::start(handle.clone());
            Ok(())
        })
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(bridge::tap_commands(tauri::generate_handler![
            check_model_status,
            download_model,
            check_ai_server_status,
            download_ai_server,
            start_sniffer_command,
            open_captures_folder,
            get_chat_history,
            annotate_furigana,
            get_system_history,
            get_service_states,
            check_all_updates,
            ignore_update,
            download_app_update,
            cancel_app_update,
            restart_to_apply_update,
            sync_dictionary,
            get_dict_version,
            get_dictionary_status,
            get_local_dictionary,
            save_local_dictionary,
            clear_chat_history,
            set_always_on_top,
            open_popup,
            load_config,
            save_config,
            save_favorites,
            minimize_window,
            close_window,
            open_app_data_folder,
            export_chat_log,
            open_browser,
            get_network_interfaces,
            set_click_through,
            grow_window,
            restore_window,
            update_tray_menu,
            launch_translator,
            block_user_command,
            unblock_user_command,
            ai_server_health_check,
            ui_system_message,
            update_global_tab_shortcut,
            ensure_firewall_rule_command,
            restart_sniffer_command,
        ]));
    // `--no-window-state`: neither restore nor save the windows' size and place.
    if !test_env::no_window_state() {
        builder = builder.plugin(window_state_plugin);
    }
    let app = builder
        .build(tauri::generate_context!())
        .expect("error while running tauri application");

    app.run(|_app_handle, event| {
        if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event {
            log::info!("Application closing. Cleaning AI Server...");
            // The tray's Quit ends the app without a close request.
            window::restore_grown_window(_app_handle);

            // Explicitly kill the llama-server to prevent zombie processes
            #[cfg(target_os = "windows")]
            {
                services::translator::server_manager::kill_orphaned_servers(&_app_handle);
            }
        }
    });
}
