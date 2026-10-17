//! The two menus a chat row opens at the pointer: the sender's (copy name, filter, block) and the
//! message's (copy, favorite, add to the dictionary). The row owns the state they share (where the
//! menu is, the selection, the "done" mark) and passes it in; each menu is shown while it is the
//! one open (`open`).

use crate::dictionary_edit::draft;
use crate::favorites::add_from_chat;
use crate::favorites_sync;
use crate::store::AppSignals;
use crate::tauri_bridge::call;
use crate::ui_types::ChatMessage;
use crate::use_context;
use crate::utils::copy_to_clipboard;
use crate::view_signals::DictDraft;
use leptos::portal::Portal;
use leptos::prelude::*;
use leptos::reactive::spawn_local;
use leptos::{component, view, IntoView};
use resonance_types::ipc::BlockUser;

/// Click on a sender's name: copy it, filter the chat to it, or block it.
#[component]
pub(super) fn SenderMenu(
    sig: RwSignal<ChatMessage>,
    open: Memo<bool>,
    pos: ReadSignal<(i32, i32)>,
) -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    view! {
        <Show when=move || open.get()>
            <Portal>
                <div class="fixed z-50 bg-base-300 border border-white/10 rounded-lg shadow-2xl p-1 flex flex-col min-w-[130px] animate-in fade-in zoom-in-95 duration-100"
                     style=move || {
                         let (x, y) = pos.get();
                         format!("top: {}px; left: {}px;", y + 8, x + 8)
                     }
                     on:click=move |ev| ev.stop_propagation()>

                    <button class="btn btn-ghost btn-sm justify-start text-xs font-normal h-8 min-h-0 px-2"
                        on:click=move |_| {
                            sig.with_untracked(|m| copy_to_clipboard(&m.nickname));
                            signals.ui.set_active_menu.set(None);
                        }>
                        "📋 Copy Name"
                    </button>

                    <button class="btn btn-ghost btn-sm justify-start text-xs font-normal h-8 min-h-0 px-2"
                        on:click=move |_| {
                            let n = sig.with_untracked(|m| m.nickname.clone());
                            if signals.chat.search_term.get_untracked() == n {
                                signals.chat.set_search_term.set("".into());
                            } else {
                                signals.chat.set_search_term.set(n);
                            }
                            signals.ui.set_active_menu.set(None);
                        }>
                        "🔍 Filter Chat"
                    </button>

                    <button class="btn btn-ghost btn-sm justify-start text-xs font-normal h-8 min-h-0 px-2 text-error"
                        on:click=move |_| {
                            let target_uid = sig.with_untracked(|m| m.uid);
                            let target_name = sig.with_untracked(|m| m.nickname.clone());
                            let blocked_name = sig.with_untracked(|m| m.nickname.clone());

                            spawn_local(async move {
                                let _ = call(&BlockUser { uid: target_uid, nickname: target_name }).await;
                            });

                            signals.config.set_blocked_users.update(|map| { map.insert(target_uid, blocked_name); });
                            signals.ui.set_active_menu.set(None);
                        }>
                        "🚫 Block User"
                    </button>
                </div>
            </Portal>
        </Show>
    }
}

/// Click on a message's text: copy the message or its translation, save it to the favorites, or put
/// a word of it in the dictionary.
#[component]
pub(super) fn MessageMenu(
    sig: RwSignal<ChatMessage>,
    open: Memo<bool>,
    pos: ReadSignal<(i32, i32)>,
    done: ReadSignal<Option<&'static str>>,
    set_done: WriteSignal<Option<&'static str>>,
    selection: ReadSignal<String>,
) -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");

    // Star: save this message (translation as the note) to the favorites.
    let save_favorite = move || {
        let (text, translated) = sig.with_untracked(|m| (m.message.clone(), m.translated.clone()));
        let mut added = false;
        signals
            .config
            .set_favorite_messages
            .update(|list| added = add_from_chat(list, &text, translated.as_deref()));
        if added {
            favorites_sync::save(signals.config);
        }
    };

    // Runs `action`, shows `label` as done, and closes the menu a moment later.
    let finish = move |label: &'static str| {
        set_done.set(Some(label));
        spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(700).await;
            if done.get_untracked() == Some(label) {
                set_done.set(None);
                if open.get_untracked() {
                    signals.ui.set_active_menu.set(None);
                }
            }
        });
    };
    let item = "btn btn-ghost btn-sm justify-start text-xs font-normal h-8 min-h-0 px-2";
    view! {
        <Show when=move || open.get()>
            <Portal>
                <div class="fixed z-50 bg-base-300 border border-white/10 rounded-lg shadow-2xl p-1 flex flex-col min-w-[150px] animate-in fade-in zoom-in-95 duration-100"
                     style=move || {
                         let (x, y) = pos.get();
                         format!("top: {}px; left: {}px;", y + 8, x + 8)
                     }
                     on:click=move |ev| ev.stop_propagation()>
                    <button class=item
                        on:click=move |_| {
                            sig.with_untracked(|m| copy_to_clipboard(&m.message));
                            finish("copy");
                        }>
                        {move || if done.get() == Some("copy") { "✓ 복사됨" } else { "📋 메시지 복사" }}
                    </button>
                    <Show when=move || sig.with(|m| m.translated.is_some())>
                        <button class=item
                            on:click=move |_| {
                                sig.with_untracked(|m| copy_to_clipboard(m.translated.as_deref().unwrap_or_default()));
                                finish("copy-translation");
                            }>
                            {move || if done.get() == Some("copy-translation") { "✓ 복사됨" } else { "📋 번역 복사" }}
                        </button>
                    </Show>
                    <button class=item
                        on:click=move |_| {
                            save_favorite();
                            finish("favorite");
                        }>
                        {move || if done.get() == Some("favorite") { "✓ 추가됨" } else { "⭐ 자주 쓰는 메시지에 추가" }}
                    </button>
                    <button class=item
                        on:click=move |_| {
                            let (key, value) = sig.with_untracked(|m| {
                                draft(&selection.get_untracked(), &m.message, m.translated.as_deref())
                            });
                            signals.ui.set_dict_draft.set(Some(DictDraft { key, value }));
                            signals.ui.set_active_menu.set(None);
                        }>
                        "📖 사전에 추가"
                    </button>
                </div>
            </Portal>
        </Show>
    }
}
