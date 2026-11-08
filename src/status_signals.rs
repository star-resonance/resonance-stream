//! App-wide signals that are not settings, grouped by what they describe
//! (the settings are `config_signals.rs`). Each group is one `AppSignals`
//! field, so a component asks for `signals.service.model_ready`, not one of
//! a hundred flat names.

use crate::ui_types::{GistMetadata, SnifferState, TranslatorState};
use leptos::prelude::{signal, ReadSignal, WriteSignal};
use resonance_types::RelayStatus;

/// What the backend services report: the translator and the sniffer, whether the model is there, and what the user has to do about it.
#[derive(Copy, Clone, Debug)]
pub struct ServiceSignals {
    pub translator_state: ReadSignal<TranslatorState>,
    pub set_translator_state: WriteSignal<TranslatorState>,
    pub translator_error: ReadSignal<String>,
    pub set_translator_error: WriteSignal<String>,
    pub sniffer_state: ReadSignal<SnifferState>,
    pub set_sniffer_state: WriteSignal<SnifferState>,
    pub sniffer_error: ReadSignal<String>,
    pub set_sniffer_error: WriteSignal<String>,
    pub is_sniffer_active: ReadSignal<bool>,
    pub set_is_sniffer_active: WriteSignal<bool>,
    pub model_ready: ReadSignal<bool>,
    pub set_model_ready: WriteSignal<bool>,
    pub restart_required: ReadSignal<bool>,
    pub set_restart_required: WriteSignal<bool>,
    pub dict_update_available: ReadSignal<bool>,
    pub set_dict_update_available: WriteSignal<bool>,
    /// What the guild relay is doing (`guild-relay-state`).
    pub guild_relay_state: ReadSignal<RelayStatus>,
    pub set_guild_relay_state: WriteSignal<RelayStatus>,
}

impl ServiceSignals {
    pub fn new() -> Self {
        let (translator_state, set_translator_state) =
            signal::<TranslatorState>(TranslatorState::Off);
        let (translator_error, set_translator_error) = signal::<String>(String::new());
        let (sniffer_state, set_sniffer_state) = signal::<SnifferState>(SnifferState::Off);
        let (sniffer_error, set_sniffer_error) = signal::<String>(String::new());
        let (is_sniffer_active, set_is_sniffer_active) = signal::<bool>(false);
        let (model_ready, set_model_ready) = signal::<bool>(false);
        let (restart_required, set_restart_required) = signal::<bool>(false);
        let (dict_update_available, set_dict_update_available) = signal::<bool>(false);
        let (guild_relay_state, set_guild_relay_state) = signal(RelayStatus::Off);
        ServiceSignals {
            translator_state,
            set_translator_state,
            translator_error,
            set_translator_error,
            sniffer_state,
            set_sniffer_state,
            sniffer_error,
            set_sniffer_error,
            is_sniffer_active,
            set_is_sniffer_active,
            model_ready,
            set_model_ready,
            restart_required,
            set_restart_required,
            dict_update_available,
            set_dict_update_available,
            guild_relay_state,
            set_guild_relay_state,
        }
    }
}

/// The first-run wizard and the download progress it shows.
#[derive(Copy, Clone, Debug)]
pub struct SetupSignals {
    pub wizard_step: ReadSignal<i32>,
    pub set_wizard_step: WriteSignal<i32>,
    pub status_text: ReadSignal<String>,
    pub set_status_text: WriteSignal<String>,
    pub downloading: ReadSignal<bool>,
    pub set_downloading: WriteSignal<bool>,
    pub progress: ReadSignal<u8>,
    pub set_progress: WriteSignal<u8>,
    /// Why the last "start downloading" stopped, shown under the button in the wizard's last step.
    pub setup_error: ReadSignal<Option<String>>,
    pub set_setup_error: WriteSignal<Option<String>>,
}

impl SetupSignals {
    pub fn new() -> Self {
        let (wizard_step, set_wizard_step) = signal::<i32>(0);
        let (status_text, set_status_text) = signal::<String>(String::new());
        let (downloading, set_downloading) = signal::<bool>(false);
        let (progress, set_progress) = signal::<u8>(0);
        let (setup_error, set_setup_error) = signal::<Option<String>>(None);
        SetupSignals {
            wizard_step,
            set_wizard_step,
            status_text,
            set_status_text,
            downloading,
            set_downloading,
            progress,
            set_progress,
            setup_error,
            set_setup_error,
        }
    }
}

/// The app and model update dialogs: which is open, what it found, how far along it is.
#[derive(Copy, Clone, Debug)]
pub struct UpdateSignals {
    pub show_app_update_modal: ReadSignal<bool>,
    pub set_show_app_update_modal: WriteSignal<bool>,
    pub show_model_update_modal: ReadSignal<bool>,
    pub set_show_model_update_modal: WriteSignal<bool>,
    pub pending_update_data: ReadSignal<Option<GistMetadata>>,
    pub set_pending_update_data: WriteSignal<Option<GistMetadata>>,
    pub app_update_step: ReadSignal<i32>,
    pub set_app_update_step: WriteSignal<i32>,
    pub app_update_progress: ReadSignal<u8>,
    pub set_app_update_progress: WriteSignal<u8>,
    /// Why the app update failed (download or signature check); shown in the
    /// dialog's error step (`app_update_step` 3).
    pub app_update_error: ReadSignal<String>,
    pub set_app_update_error: WriteSignal<String>,
    pub model_update_step: ReadSignal<i32>,
    pub set_model_update_step: WriteSignal<i32>,
    pub model_update_progress: ReadSignal<u8>,
    pub set_model_update_progress: WriteSignal<u8>,
}

impl UpdateSignals {
    pub fn new() -> Self {
        let (show_app_update_modal, set_show_app_update_modal) = signal::<bool>(false);
        let (show_model_update_modal, set_show_model_update_modal) = signal::<bool>(false);
        let (pending_update_data, set_pending_update_data) = signal::<Option<GistMetadata>>(None);
        let (app_update_step, set_app_update_step) = signal::<i32>(0);
        let (app_update_progress, set_app_update_progress) = signal::<u8>(0);
        let (app_update_error, set_app_update_error) = signal::<String>(String::new());
        let (model_update_step, set_model_update_step) = signal::<i32>(0);
        let (model_update_progress, set_model_update_progress) = signal::<u8>(0);
        UpdateSignals {
            show_app_update_modal,
            set_show_app_update_modal,
            show_model_update_modal,
            set_show_model_update_modal,
            pending_update_data,
            set_pending_update_data,
            app_update_step,
            set_app_update_step,
            app_update_progress,
            set_app_update_progress,
            app_update_error,
            set_app_update_error,
            model_update_step,
            set_model_update_step,
            model_update_progress,
            set_model_update_progress,
        }
    }
}
