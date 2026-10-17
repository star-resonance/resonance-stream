use crate::store::{AppActions, AppSignals};
use crate::tauri_bridge::call;
use leptos::prelude::*;
use leptos::reactive::spawn_local;
use resonance_types::ipc::UnblockUser;

/// Hide-blocked toggle and the unblock list.
#[component]
pub fn BlockedUsersSection() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    let actions = use_context::<AppActions>().expect("AppActions missing");

    view! {
        // --- SECTION: BLOCKED USERS ---
        <div class="space-y-2">
            // HIDE BLOCKED MESSAGES TOGGLE
            <div class="flex items-center justify-between bg-base-200/50 p-2 rounded-lg border border-base-content/5 mb-2">
                <div class="flex flex-col">
                    <span class="text-xs font-bold text-base-content/80">"차단된 메시지 완전 숨기기"</span>
                    <span class="text-[9px] text-base-content/50">"활성화 시 '(차단된 사용자의 메시지입니다)' 문구도 표시하지 않습니다."</span>
                </div>
                <input type="checkbox" class="toggle toggle-error toggle-sm"
                    prop:checked=move || signals.config.hide_blocked_messages.get()
                    on:change=move |ev| {
                        signals.config.set_hide_blocked_messages.set(event_target_checked(&ev));
                        actions.save_config.dispatch(());
                    }
                />
            </div>

            <div class="bg-base-200/50 rounded-lg p-2 max-h-40 overflow-y-auto border border-base-content/5">
                {move || {
                    let blocked = signals.config.blocked_users.get();
                    if blocked.is_empty() {
                        view! { <div class="text-[10px] text-base-content/50 italic text-center py-2">"차단된 사용자가 없습니다."</div> }.into_any()
                    } else {
                        // Convert the HashMap into a viewable list
                        let blocked_list = blocked.into_iter().collect::<Vec<_>>();

                        blocked_list.into_iter().map(|(uid, nickname)| {
                            let uid_clone = uid;
                            view! {
                                <div class="flex items-center justify-between p-1.5 hover:bg-base-content/5 rounded transition-colors group">
                                    <span class="text-xs font-bold text-base-content/80">{nickname}</span>
                                    <button
                                        class="btn btn-ghost btn-xs text-error opacity-50 group-hover:opacity-100 h-6 min-h-0 px-2"
                                        on:click=move |_| {
                                            // 1. Tell Backend to unblock and save
                                            spawn_local(async move {
                                                let _ = call(&UnblockUser { uid: uid_clone }).await;
                                            });

                                            // 2. Instantly remove from frontend UI state
                                            signals.config.set_blocked_users.update(|map| {
                                                map.remove(&uid_clone);
                                            });
                                        }>
                                        "차단 해제"
                                    </button>
                                </div>
                            }
                        }).collect_view().into_any()
                    }
                }}
            </div>
        </div>
    }
}
