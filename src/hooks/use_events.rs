use crate::chat_view::{is_muted, Tab};
use crate::service_state::SeqGate;
use crate::store::AppSignals;
use crate::tauri_bridge::{call, listen};
use crate::ui_types::{
    ChatMessage, SnifferState, SnifferStatePayload, SystemMessage, TranslationResult,
    TranslatorState, TranslatorStatePayload,
};
use crate::view_signals::ChatSignals;
use leptos::logging::log;
use leptos::prelude::*;
use resonance_types::ipc::{event, ClearChatHistory, GetServiceStates};
use wasm_bindgen::prelude::*;

pub async fn clear_backend_history() {
    let _ = call(&ClearChatHistory {}).await;
}

/// Listeners are registered once per page load: a second registration (the
/// setup wizard re-opening after a firewall error, then finishing) would
/// handle every event twice.
static LISTENERS_REGISTERED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub async fn setup_event_listeners(signals: AppSignals) {
    if LISTENERS_REGISTERED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    // 1. Create the closures using our new helper functions
    let packet_closure = create_packet_handler(signals);
    let system_closure = create_system_handler(signals);
    let translator_state_closure = create_translator_state_handler(signals);
    let sniffer_state_closure = create_sniffer_state_handler(signals);
    let translation_closure = create_translation_handler(signals);
    let update_message_closure = create_update_message_handler(signals);
    let firewall_closure = create_firewall_missing_handler(signals);

    crate::favorites_sync::listen_for_changes(signals.config).await;

    // 2. Register all listeners
    listen(event::PACKET_EVENT, &packet_closure).await;
    listen(event::SYSTEM_EVENT, &system_closure).await;
    listen(event::TRANSLATOR_STATE, &translator_state_closure).await;
    listen(event::SNIFFER_STATE, &sniffer_state_closure).await;
    listen(event::TRANSLATION_EVENT, &translation_closure).await;
    listen(event::CHAT_MESSAGE_UPDATE, &update_message_closure).await;
    listen(event::FIREWALL_MISSING, &firewall_closure).await;

    // 3. Prevent memory leaks / keep closures alive
    packet_closure.forget();
    system_closure.forget();
    translator_state_closure.forget();
    sniffer_state_closure.forget();
    translation_closure.forget();
    update_message_closure.forget();
    firewall_closure.forget();

    // 4. States emitted before we listened (the translator starts with the
    // app; a reloaded page missed everything): ask once, now that no later
    // event can be missed. The seq gates drop it if an event overtook it.
    if let Ok(states) = call(&GetServiceStates {}).await {
        apply_sniffer_state(signals, states.sniffer);
        apply_translator_state(signals, states.translator);
    }
}

static SNIFFER_SEQ: SeqGate = SeqGate::new();
static TRANSLATOR_SEQ: SeqGate = SeqGate::new();

fn apply_translator_state(signals: AppSignals, payload: TranslatorStatePayload) {
    if !TRANSLATOR_SEQ.accept(payload.seq) {
        return;
    }
    signals.service.set_translator_state.set(payload.state);
    if payload.state == TranslatorState::Error {
        signals.service.set_translator_error.set(payload.message);
    }
}

fn apply_sniffer_state(signals: AppSignals, payload: SnifferStatePayload) {
    if !SNIFFER_SEQ.accept(payload.seq) {
        return;
    }
    signals.service.set_sniffer_state.set(payload.state);
    // If it's an error, save the message so the user can click the badge to read it
    if payload.state == SnifferState::Error {
        signals.service.set_sniffer_error.set(payload.message);
    }
}

// --- EXTRACTED HANDLER FUNCTIONS ---

#[derive(serde::Deserialize)]
struct TauriEvent<T> {
    payload: T,
}

/// Deserializes an event's payload straight from the JS value -- no
/// intermediate `serde_json::Value`, no copy of the payload.
fn payload<T: serde::de::DeserializeOwned>(event_obj: JsValue) -> Option<T> {
    serde_wasm_bindgen::from_value::<TauriEvent<T>>(event_obj)
        .ok()
        .map(|event| event.payload)
}

fn create_packet_handler(signals: AppSignals) -> Closure<dyn FnMut(JsValue)> {
    Closure::wrap(Box::new(move |event_obj: JsValue| {
        // Stickers/emotes arrive already normalized by the backend.
        let Some(packet) = payload::<ChatMessage>(event_obj) else {
            return;
        };

        let limits = signals.config.tab_limits.get_untracked();
        // Blocked senders (and WORLD chat below the minimum level) never
        // ping and are not counted as unread.
        let muted = is_muted(&packet, signals.config.min_sender_level.get_untracked());
        let alert = !muted
            && signals
                .config
                .alert_keywords
                .with_untracked(|kws| kws.iter().any(|kw| packet.message.contains(kw.as_str())));
        let channel = packet.channel;
        let message_for_log = alert.then(|| packet.message.clone());

        let pid = packet.pid;
        signals.config.custom_tab_filters.with_untracked(|filters| {
            signals.chat.set_chat.update(|store| {
                store.add(pid, channel, ArcRwSignal::new(packet), filters, &limits);
            });
        });

        let tab = Tab::from_label(&signals.config.active_tab.get_untracked());
        let is_visible = signals
            .config
            .custom_tab_filters
            .with_untracked(|filters| tab.shows_channel(channel, filters));

        // Only increment if the message belongs to the tab we are currently looking at
        if muted {
            // not unread: no badge for blocked or low-level senders
        } else if is_visible && !signals.chat.is_at_bottom.get_untracked() {
            signals.chat.set_unread_count.update(|c| *c += 1);
        } else if !is_visible {
            // Inactive tab -> Increment Tab Badge
            signals.chat.set_unread_counts.update(|counts| {
                *counts.entry(channel.to_string()).or_insert(0) += 1;
            });
        }

        let volume = signals.config.alert_volume.get_untracked();
        if let Some(message) = message_for_log {
            // Fire and forget the audio ping
            if volume > 0.0 {
                log!("audio ping by keyword {:?}", message);
                if let Ok(audio) = web_sys::HtmlAudioElement::new_with_src("public/ping.mp3") {
                    // Convert f32 to f64 for the Web Audio API
                    audio.set_volume(volume as f64);
                    let _ = audio.play();
                }
            }
        }
    }) as Box<dyn FnMut(JsValue)>)
}

fn create_translation_handler(signals: AppSignals) -> Closure<dyn FnMut(JsValue)> {
    Closure::wrap(Box::new(move |event_obj: JsValue| {
        if let Some(payload) = payload::<TranslationResult>(event_obj) {
            land_translation(&signals.chat, payload);
        }
    }) as Box<dyn FnMut(JsValue)>)
}

/// Puts a translation into its row. Only this row re-renders; the list itself is untouched, so the
/// list is told (`rows_changed`) that a row may have grown.
fn land_translation(chat: &ChatSignals, result: TranslationResult) {
    let row = chat
        .chat
        .with_untracked(|store| store.get(result.pid).cloned());
    match row {
        Some(chat_rw) => {
            chat_rw.update(|c| c.translated = Some(result.translated));
            chat.set_rows_changed.update(|n| *n += 1);
        }
        // The row is not here yet: its history is still being fetched.
        // Keep the text; hydration applies it after the merge.
        None => chat
            .set_chat
            .update(|store| store.hold_translation(result.pid, result.translated)),
    }
}

fn create_system_handler(signals: AppSignals) -> Closure<dyn FnMut(JsValue)> {
    Closure::wrap(Box::new(move |event_obj: JsValue| {
        let Some(packet) = payload::<SystemMessage>(event_obj) else {
            return;
        };
        signals.chat.set_system_log.update(|log| {
            if log.len() >= 200 {
                log.remove(0);
            }
            log.push(ArcRwSignal::new(packet));
        });

        let active_tab = signals.config.active_tab.get_untracked();
        if active_tab != Tab::All.label() && active_tab != Tab::System.label() {
            signals.chat.set_unread_counts.update(|counts| {
                *counts.entry(Tab::System.key().to_string()).or_insert(0) += 1;
            });
        }
    }) as Box<dyn FnMut(JsValue)>)
}

fn create_translator_state_handler(signals: AppSignals) -> Closure<dyn FnMut(JsValue)> {
    Closure::wrap(Box::new(move |event_obj: JsValue| {
        if let Some(payload) = payload::<TranslatorStatePayload>(event_obj) {
            apply_translator_state(signals, payload);
        }
    }) as Box<dyn FnMut(JsValue)>)
}

fn create_sniffer_state_handler(signals: AppSignals) -> Closure<dyn FnMut(JsValue)> {
    Closure::wrap(Box::new(move |event_obj: JsValue| {
        if let Some(payload) = payload::<SnifferStatePayload>(event_obj) {
            apply_sniffer_state(signals, payload);
        }
    }) as Box<dyn FnMut(JsValue)>)
}

fn create_update_message_handler(signals: AppSignals) -> Closure<dyn FnMut(JsValue)> {
    Closure::wrap(Box::new(move |event_obj: JsValue| {
        // The fully updated ChatMessage sent from the backend
        let Some(updated_msg) = payload::<ChatMessage>(event_obj) else {
            return;
        };
        land_message_update(&signals.chat, updated_msg);
    }) as Box<dyn FnMut(JsValue)>)
}

/// Overwrites the row of a message the backend changed (blocked, translated), if it is in the list.
fn land_message_update(chat: &ChatSignals, updated: ChatMessage) {
    let row = chat
        .chat
        .with_untracked(|store| store.get(updated.pid).cloned());
    if let Some(chat_rw) = row {
        chat_rw.set(updated);
        chat.set_rows_changed.update(|n| *n += 1);
    }
}

fn create_firewall_missing_handler(signals: AppSignals) -> Closure<dyn FnMut(JsValue)> {
    Closure::wrap(Box::new(move |_| {
        // 1. Force the Setup Wizard to appear
        signals.config.set_init_done.set(false);

        // 2. Make sure it starts on Step 0 (the Firewall Agreement page)
        signals.setup.set_wizard_step.set(0);
    }) as Box<dyn FnMut(JsValue)>)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui_types::Channel;
    use std::collections::HashMap;

    fn with_row(signals: &AppSignals, pid: u64) -> ArcRwSignal<ChatMessage> {
        let row = ArcRwSignal::new(ChatMessage {
            pid,
            message: "こんにちは".into(),
            ..Default::default()
        });
        signals.chat.set_chat.update(|store| {
            store.add(pid, Channel::World, row.clone(), &[], &HashMap::new());
        });
        row
    }

    fn result(pid: u64, text: &str) -> TranslationResult {
        TranslationResult {
            pid,
            translated: text.into(),
        }
    }

    #[test]
    fn a_translation_for_a_row_in_the_list_fills_it_and_says_a_row_changed() {
        let signals = AppSignals::new();
        let row = with_row(&signals, 7);
        assert_eq!(signals.chat.rows_changed.get_untracked(), 0);
        land_translation(&signals.chat, result(7, "안녕하세요"));
        assert_eq!(
            row.get_untracked().translated.as_deref(),
            Some("안녕하세요")
        );
        assert_eq!(signals.chat.rows_changed.get_untracked(), 1);
    }

    #[test]
    fn a_translation_for_a_row_not_here_yet_is_held_and_changes_no_row() {
        let signals = AppSignals::new();
        land_translation(&signals.chat, result(9, "나중에"));
        assert_eq!(signals.chat.rows_changed.get_untracked(), 0);
        // Its row arrives later (the history merge): only then is the held text handed out.
        with_row(&signals, 9);
        let held = signals
            .chat
            .set_chat
            .try_update(|store| store.take_held_translations());
        assert_eq!(held, Some(vec![(9, "나중에".to_string())]));
    }

    #[test]
    fn an_updated_message_replaces_its_row_and_says_a_row_changed() {
        let signals = AppSignals::new();
        let row = with_row(&signals, 3);
        land_message_update(
            &signals.chat,
            ChatMessage {
                pid: 3,
                is_blocked: true,
                ..Default::default()
            },
        );
        assert!(row.get_untracked().is_blocked);
        assert_eq!(signals.chat.rows_changed.get_untracked(), 1);
    }

    #[test]
    fn an_update_for_a_message_that_is_not_in_the_list_changes_nothing() {
        let signals = AppSignals::new();
        land_message_update(
            &signals.chat,
            ChatMessage {
                pid: 99,
                ..Default::default()
            },
        );
        assert_eq!(signals.chat.rows_changed.get_untracked(), 0);
    }
}
