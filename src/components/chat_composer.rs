use crate::composer_view::{
    can_send, channel_options, counter_over, counter_text, enter_sends, module_line, preselect,
    send_request, COUNTER_TITLE,
};
use crate::store::AppSignals;
use crate::tauri_bridge::{call, listen};
use crate::ui_types::ProgressPayload;
use leptos::html::Input;
use leptos::prelude::*;
use leptos::task::spawn_local;
use resonance_types::ipc::{event, SendChat};
use resonance_types::SendChatChannel;
use wasm_bindgen::prelude::*;

/// The composer under the chat list: channel picker, draft, live counter, Send
/// and one error line. The draft stays on a failed send. Mounted under a
/// `<Show>` on `ingame_send`, so turning the setting off drops draft and error.
#[component]
pub fn ChatComposer() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    let active_tab = signals.config.active_tab;

    let draft = RwSignal::new(String::new());
    let channel = RwSignal::new(SendChatChannel::ALL[0]);
    let sending = RwSignal::new(false);
    let error = RwSignal::new(String::new());
    let input_ref = NodeRef::<Input>::new();
    // The module's download, while it runs: its progress label is the only thing this listens for.
    let module = RwSignal::new(None::<String>);
    spawn_local(async move {
        let closure = Closure::wrap(Box::new(move |event_obj: JsValue| {
            if let Ok(ev) = serde_wasm_bindgen::from_value::<serde_json::Value>(event_obj) {
                if let Ok(payload) =
                    serde_json::from_value::<ProgressPayload>(ev["payload"].clone())
                {
                    if payload.current_file.starts_with("게임 내 전송 모듈") {
                        // The signal is gone once the composer is (setting off).
                        let _ = module.try_set(module_line(&payload.current_file, payload.percent));
                    }
                }
            }
        }) as Box<dyn FnMut(JsValue)>);
        listen(event::DOWNLOAD_PROGRESS, &closure).await;
        closure.forget();
    });

    // A tab with a chat channel moves the picker to it; any other tab leaves it.
    Effect::new(move |_| {
        if let Some(c) = preselect(&active_tab.get()) {
            channel.set(c);
        }
    });

    let send = move || {
        if !can_send(&draft.get_untracked(), sending.get_untracked()) {
            return;
        }
        sending.set(true);
        let request = send_request(channel.get_untracked(), &draft.get_untracked());
        spawn_local(async move {
            match call::<SendChat>(&request).await {
                Ok(()) => {
                    draft.set(String::new());
                    error.set(String::new());
                }
                Err(e) => error.set(e),
            }
            sending.set(false);
            if let Some(input) = input_ref.get_untracked() {
                let _ = input.focus();
            }
        });
    };

    view! {
        <div class="flex flex-col shrink-0 border-t border-base-content/10">
            <div class="flex items-center gap-1 px-2 py-1">
                <select
                    class="select select-xs w-auto"
                    on:change=move |ev| {
                        let value = event_target_value(&ev);
                        if let Some(c) = SendChatChannel::ALL.into_iter().find(|c| c.as_str() == value) {
                            channel.set(c);
                        }
                    }
                >
                    {channel_options()
                        .into_iter()
                        .map(|(value, label)| {
                            view! {
                                <option value=value selected=move || channel.get().as_str() == value>
                                    {label}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
                <input
                    node_ref=input_ref
                    type="text"
                    class="input input-xs flex-1 min-w-0"
                    placeholder="메시지 입력 (Enter: 보내기)"
                    prop:value=move || draft.get()
                    prop:readOnly=move || sending.get()
                    on:input=move |ev| draft.set(event_target_value(&ev))
                    on:keydown=move |ev| {
                        if enter_sends(&ev.key(), ev.shift_key(), ev.is_composing(), ev.key_code()) {
                            ev.prevent_default();
                            send();
                        }
                    }
                />
                <span
                    class="text-[10px] tabular-nums"
                    class=("text-error", move || counter_over(&draft.get()))
                    class=("text-base-content/50", move || !counter_over(&draft.get()))
                    title=COUNTER_TITLE
                >
                    {move || counter_text(&draft.get())}
                </span>
                <button
                    class="btn btn-xs btn-success"
                    disabled=move || !can_send(&draft.get(), sending.get())
                    on:click=move |_| send()
                >
                    "보내기"
                </button>
            </div>
            <div
                class="px-2 pb-1 text-[10px] text-base-content/70 truncate"
                class:hidden=move || module.get().is_none()
            >
                {move || module.get().unwrap_or_default()}
            </div>
            <div
                class="px-2 pb-1 text-[10px] text-error truncate"
                class:hidden=move || error.get().is_empty()
                title=move || error.get()
            >
                {move || error.get()}
            </div>
        </div>
    }
}
