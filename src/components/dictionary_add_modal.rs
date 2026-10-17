//! "사전에 추가": a term from a chat message goes into the custom dictionary.
//! The dialog is open while `ui.dict_draft` is set (the row menu sets it).

use crate::dictionary_edit::{add_entry, categories, existing, DEFAULT_CATEGORY};
use crate::store::AppSignals;
use crate::tauri_bridge::call;
use leptos::prelude::*;
use leptos::task::spawn_local;
use resonance_types::ipc::{GetLocalDictionary, SaveLocalDictionary};

#[component]
pub fn AddToDictionaryModal() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    let draft = signals.ui.dict_draft;
    let set_draft = signals.ui.set_dict_draft;

    // Created here, outside the <Show>, so a typed value is not lost to a re-render.
    let (key, set_key) = signal(String::new());
    let (value, set_value) = signal(String::new());
    let (category, set_category) = signal(DEFAULT_CATEGORY.to_string());
    // The dictionary file as it is on disk, read when the dialog opens.
    let (dict_json, set_dict_json) = signal(String::new());
    let (error, set_error) = signal(None::<String>);
    let (saving, set_saving) = signal(false);

    Effect::new(move |_| {
        let Some(d) = draft.get() else { return };
        set_key.set(d.key);
        set_value.set(d.value);
        set_category.set(DEFAULT_CATEGORY.to_string());
        set_error.set(None);
        set_saving.set(false);
        spawn_local(async move {
            if let Ok(json) = call(&GetLocalDictionary {}).await {
                set_dict_json.set(json);
            }
        });
    });

    // What the key already means in that category, if anything.
    let old_value =
        Memo::new(move |_| dict_json.with(|json| existing(json, &category.get(), &key.get())));

    let close = move || set_draft.set(None);

    let save = move || {
        let text = match add_entry(
            &dict_json.get_untracked(),
            &category.get_untracked(),
            &key.get_untracked(),
            &value.get_untracked(),
        ) {
            Ok(text) => text,
            Err(e) => {
                set_error.set(Some(e.message()));
                return;
            }
        };
        set_saving.set(true);
        spawn_local(async move {
            // The backend swaps the dictionary in place: the next translation uses it.
            match call(&SaveLocalDictionary { content: text }).await {
                Ok(()) => set_draft.set(None),
                Err(why) => {
                    set_saving.set(false);
                    set_error.set(Some(format!("저장하지 못했습니다: {}", why)));
                }
            }
        });
    };

    view! {
        <Show when=move || draft.get().is_some()>
            <div class="modal modal-open backdrop-blur-sm z-[30000]">
                <div class="modal-box bg-base-300 border border-base-content/10 w-11/12 max-w-md p-0 overflow-hidden shadow-2xl flex flex-col">
                    <div class="flex items-center justify-between p-3 border-b border-base-content/5 bg-base-200">
                        <h2 class="text-sm font-black tracking-widest text-base-content">"사전에 추가"</h2>
                        <button class="btn btn-ghost btn-xs text-xl" on:click=move |_| close()>"✕"</button>
                    </div>

                    <div class="flex flex-col gap-2 p-3">
                        <label class="flex flex-col gap-1">
                            <span class="text-[10px] font-bold text-base-content/70">"원문 (일본어)"</span>
                            <input type="text" class="input input-bordered input-sm w-full"
                                prop:value=move || key.get()
                                on:input=move |ev| { set_key.set(event_target_value(&ev)); set_error.set(None); }
                            />
                        </label>
                        <label class="flex flex-col gap-1">
                            <span class="text-[10px] font-bold text-base-content/70">"번역 (한국어)"</span>
                            <input type="text" class="input input-bordered input-sm w-full"
                                prop:value=move || value.get()
                                on:input=move |ev| { set_value.set(event_target_value(&ev)); set_error.set(None); }
                                on:keydown=move |ev| if ev.key() == "Enter" { save() }
                            />
                        </label>
                        <label class="flex flex-col gap-1">
                            <span class="text-[10px] font-bold text-base-content/70">"분류"</span>
                            <input type="text" list="dictionary-categories" class="input input-bordered input-xs w-full"
                                prop:value=move || category.get()
                                on:input=move |ev| set_category.set(event_target_value(&ev))
                            />
                            <datalist id="dictionary-categories">
                                {move || dict_json.with(|json| categories(json))
                                    .into_iter()
                                    .map(|c| view! { <option value=c></option> })
                                    .collect_view()}
                            </datalist>
                        </label>

                        {move || old_value.get().map(|old| view! {
                            <div class="text-[11px] text-warning bg-warning/10 p-1.5 rounded">
                                {format!("이미 있는 단어입니다: {old} (저장하면 바뀝니다)")}
                            </div>
                        })}
                        {move || error.get().map(|e| view! {
                            <div class="text-[10px] text-error bg-error/10 p-1.5 rounded">{e}</div>
                        })}
                        <Show when=move || signals.config.auto_sync_latest_dict.get()>
                            <div class="text-[10px] text-base-content/60">
                                "사전 자동 동기화가 켜져 있어, 다음 동기화 때 추가한 단어가 사라질 수 있습니다. (설정 > 데이터)"
                            </div>
                        </Show>

                        <div class="flex justify-end gap-1 pt-1">
                            <button class="btn btn-ghost btn-xs" on:click=move |_| close()>"취소"</button>
                            <button class="btn btn-success btn-xs" prop:disabled=move || saving.get() on:click=move |_| save()>
                                {move || if old_value.get().is_some() { "덮어쓰기" } else { "추가" }}
                            </button>
                        </div>
                    </div>
                </div>
                <div class="modal-backdrop" on:click=move |_| close()></div>
            </div>
        </Show>
    }
}
