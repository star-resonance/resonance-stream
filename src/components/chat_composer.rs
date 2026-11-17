use crate::composer_view::{
    can_send, channel_options, clears_choice, clears_draft, client_from_value, client_options,
    counter_over, counter_text, enter_sends, module_line, send_plan, send_request, show_picker,
    stored_channel, tab_pick, verdict_line, SendPlan, COUNTER_TITLE,
};
use crate::store::{AppActions, AppSignals};
use crate::tauri_bridge::{call, listen};
use crate::ui_types::ProgressPayload;
use leptos::html::Input;
use leptos::prelude::*;
use leptos::task::spawn_local;
use resonance_types::ipc::{event, GameClient, GameClients, SendChat, CLIENT_GONE_LINE};
use resonance_types::SendChatChannel;
use wasm_bindgen::prelude::*;

/// The composer under the chat list: channel picker, draft, live counter, Send
/// and one error line. The draft stays on a failed send. Mounted under a
/// `<Show>` on `ingame_send`, so turning the setting off drops draft and error.
#[component]
pub fn ChatComposer() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    let actions = use_context::<AppActions>().expect("AppActions missing");
    let active_tab = signals.config.active_tab;
    let send_channel = signals.config.send_channel;
    let set_send_channel = signals.config.set_send_channel;
    // The picker shows, and sends to, the saved channel: one value for both.
    let channel = move || stored_channel(&send_channel.get());
    let choose = move |c: SendChatChannel| {
        if c.as_str() != send_channel.get_untracked() {
            set_send_channel.set(c.as_str().to_owned());
            actions.save_config.dispatch(());
        }
    };

    let draft = RwSignal::new(String::new());
    let sending = RwSignal::new(false);
    let error = RwSignal::new(String::new());
    let input_ref = NodeRef::<Input>::new();
    // The game clients last read and the one chosen (a pid means nothing after a restart, so
    // neither is saved). The picker shows with two or more clients.
    let clients = RwSignal::new(Vec::<GameClient>::new());
    let chosen = RwSignal::new(None::<u32>);
    // Reads the list (the old one stays on an error) and returns what is now known.
    let refresh = move || async move {
        if let Ok(list) = call::<GameClients>(&GameClients {}).await {
            // The signal is gone once the composer is (setting off).
            let _ = clients.try_set(list);
        }
        clients.try_get_untracked().unwrap_or_default()
    };
    spawn_local(async move {
        refresh().await;
    });
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

    // A tab change to a chat channel moves the picker to it; any other tab leaves it. Not on the
    // first run: on start the saved channel wins over the restored tab.
    Effect::new(move |prev: Option<()>| {
        if let Some(c) = tab_pick(
            prev.is_none(),
            &active_tab.get(),
            &send_channel.get_untracked(),
        ) {
            choose(c);
        }
    });

    let send = move || {
        if !can_send(&draft.get_untracked(), sending.get_untracked()) {
            return;
        }
        sending.set(true);
        let channel = channel();
        let text = draft.get_untracked();
        spawn_local(async move {
            // The decision uses the list this refresh read, not the signal.
            let list = refresh().await;
            match send_plan(&list, chosen.get_untracked()) {
                SendPlan::Gone => {
                    chosen.set(None);
                    error.set(CLIENT_GONE_LINE.to_string());
                }
                SendPlan::Send(client) => {
                    match call::<SendChat>(&send_request(channel, &text, client)).await {
                        Ok(verdict) => {
                            if clears_draft(&verdict) {
                                draft.set(String::new());
                            }
                            error.set(verdict_line(&verdict));
                        }
                        Err(e) => {
                            if clears_choice(&e) {
                                chosen.set(None);
                            }
                            error.set(e);
                        }
                    }
                }
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
                            choose(c);
                        }
                    }
                >
                    {channel_options()
                        .into_iter()
                        .map(|(value, label)| {
                            view! {
                                <option value=value selected=move || channel().as_str() == value>
                                    {label}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
                <Show when=move || show_picker(&clients.get())>
                    <select
                        class="select select-xs w-auto"
                        on:change=move |ev| chosen.set(client_from_value(&event_target_value(&ev)))
                    >
                        <option value="" selected=move || chosen.get().is_none()>
                            "게임 선택"
                        </option>
                        {move || {
                            client_options(&clients.get())
                                .into_iter()
                                .map(|(value, label)| {
                                    let target = client_from_value(&value);
                                    view! {
                                        <option value=value selected=move || chosen.get() == target>
                                            {label}
                                        </option>
                                    }
                                })
                                .collect_view()
                        }}
                    </select>
                </Show>
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
