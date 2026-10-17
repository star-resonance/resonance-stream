use crate::store::AppSignals;
use crate::tauri_bridge::call;
use leptos::prelude::*;
use leptos::task::spawn_local;
use resonance_types::ipc::{GetDictVersion, GetLocalDictionary, SaveLocalDictionary};
use std::collections::BTreeMap;

#[component]
pub fn DictionaryModal() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");

    let (dict, set_dict) = signal(BTreeMap::<String, BTreeMap<String, String>>::new());
    let (active_category, set_active_category) = signal("chat".to_string());
    let (version, set_version) = signal("0.0.0".to_string());

    // Signals for adding a new word
    let (new_key, set_new_key) = signal(String::new());
    let (new_val, set_new_val) = signal(String::new());

    // Signals for the Inline Editing feature
    // editing_target: Option<(Category, OriginalKey, "key" | "value")>
    let (editing_target, set_editing_target) = signal(None::<(String, String, String)>);
    let (edit_value, set_edit_value) = signal(String::new());

    let (is_adding_category, set_is_adding_category) = signal(false);
    let (new_category_name, set_new_category_name) = signal(String::new());

    let (is_editing_categories, set_is_editing_categories) = signal(false);
    let (renaming_category, set_renaming_category) = signal(None::<String>);
    let (edit_category_name, set_edit_category_name) = signal(String::new());

    // Fetch the dictionary data when the modal opens
    Effect::new(move |_| {
        if signals.ui.show_dictionary.get() {
            spawn_local(async move {
                if let Ok(v_str) = call(&GetDictVersion {}).await {
                    set_version.set(v_str);
                }

                if let Ok(json_str) = call(&GetLocalDictionary {}).await {
                    let parsed: serde_json::Value =
                        serde_json::from_str(&json_str).unwrap_or(serde_json::json!({}));

                    let mut new_dict = BTreeMap::new();

                    if let Some(obj) = parsed.as_object() {
                        for (cat, items) in obj {
                            let mut cat_map = BTreeMap::new();
                            if let Some(items_obj) = items.as_object() {
                                for (k, val) in items_obj {
                                    if let Some(val_str) = val.as_str() {
                                        cat_map.insert(k.clone(), val_str.to_string());
                                    }
                                }
                            }
                            new_dict.insert(cat.clone(), cat_map);
                        }
                    }
                    set_dict.set(new_dict);
                }
            });
        }
    });

    // Save the changes to the disk
    let save_dict = move |_| {
        let current_dict = dict.get_untracked();
        let json_payload = serde_json::to_string_pretty(&current_dict).unwrap();

        spawn_local(async move {
            // The backend swaps the dictionary in place: the next translation
            // uses it, no restart needed.
            let _ = call(&SaveLocalDictionary {
                content: json_payload,
            })
            .await;

            signals.ui.set_show_dictionary.set(false);
        });
    };

    let delete_word = move |category: String, key: String| {
        set_dict.update(|d| {
            if let Some(cat) = d.get_mut(&category) {
                cat.remove(&key);
            }
        });
    };

    // Commit the inline edit and update the dictionary state
    let commit_edit = move || {
        if let Some((cat, orig_key, field)) = editing_target.get_untracked() {
            let new_text = edit_value.get_untracked().trim().to_string();

            // Clear editing state immediately to prevent blur/enter conflict
            set_editing_target.set(None);

            if new_text.is_empty() {
                return;
            }

            set_dict.update(|d| {
                if let Some(cat_map) = d.get_mut(&cat) {
                    if field == "key" && orig_key != new_text {
                        if let Some(val) = cat_map.remove(&orig_key) {
                            cat_map.insert(new_text, val);
                        }
                    } else if field == "value" {
                        cat_map.insert(orig_key, new_text);
                    }
                }
            });
        }
    };

    let add_category = move || {
        let name = new_category_name.get_untracked().trim().to_string();
        if !name.is_empty() {
            set_dict.update(|d| {
                // If the category doesn't exist, insert it with an empty BTreeMap
                d.entry(name.clone()).or_insert_with(BTreeMap::new);
            });
            // Automatically switch to the newly created category
            set_active_category.set(name);
        }
        // Reset the input state
        set_new_category_name.set(String::new());
        set_is_adding_category.set(false);
    };

    let delete_category = move |cat: String| {
        if let Some(w) = web_sys::window() {
            if !w
                .confirm_with_message(&format!(
                    "'{}' 카테고리와 내부의 모든 단어를 삭제하시겠습니까?",
                    cat
                ))
                .unwrap_or(false)
            {
                return;
            }
        }
        set_dict.update(|d| {
            d.remove(&cat);
        });

        // Ensure we don't stay on a deleted category
        if active_category.get_untracked() == cat {
            let first_cat = dict
                .get_untracked()
                .keys()
                .next()
                .cloned()
                .unwrap_or_else(|| "chat".to_string());
            set_active_category.set(first_cat);
        }
    };

    let commit_category_rename = move || {
        if let Some(old_cat) = renaming_category.get_untracked() {
            let new_cat = edit_category_name.get_untracked().trim().to_string();
            set_renaming_category.set(None);

            if new_cat.is_empty() || new_cat == old_cat {
                return;
            }

            set_dict.update(|d| {
                // Prevent merging or overwriting an existing category
                if !d.contains_key(&new_cat) {
                    if let Some(vals) = d.remove(&old_cat) {
                        d.insert(new_cat.clone(), vals);
                    }
                }
            });

            // Keep the user on the category they just renamed
            if active_category.get_untracked() == old_cat {
                if dict.get_untracked().contains_key(&new_cat) {
                    set_active_category.set(new_cat);
                }
            }
        }
    };

    view! {
        <Show when=move || signals.ui.show_dictionary.get()>
            <div class="modal modal-open backdrop-blur-sm transition-all duration-300 z-[30000]">
                <div class="modal-box bg-base-300 border border-base-content/10 w-11/12 max-w-4xl p-0 overflow-hidden shadow-2xl flex flex-col h-[80vh] animate-in zoom-in duration-200">

                    // --- HEADER ---
                    <div class="flex items-center justify-between p-4 border-b border-base-content/5 bg-base-200">
                        <div class="flex items-center gap-3">
                            <h2 class="text-sm font-black tracking-widest text-base-content">"사용자 사전 편집기"</h2>
                            <span class="badge badge-info badge-sm font-mono opacity-80">{move || format!("v{}", version.get())}</span>
                        </div>
                        <button class="btn btn-ghost btn-xs text-xl"
                                on:click=move |_| signals.ui.set_show_dictionary.set(false)>"✕"</button>
                    </div>

                    // --- MAIN CONTENT AREA ---
                    <div class="flex flex-1 overflow-hidden">
                        // --- SIDEBAR: CATEGORIES ---
                        <div class="w-48 bg-base-200/50 border-r border-base-content/5 flex flex-col p-2">
                            <div class="flex-1 overflow-y-auto custom-scrollbar">
                                <ul class="menu menu-xs w-full gap-1 p-0">
                                    <For
                                        each={move || dict.get().keys().cloned().collect::<Vec<_>>()}
                                        key=|cat| cat.clone()
                                        children=move |cat| {
                                            let cat_clone = cat.clone();
                                            let display_name = cat.clone().to_uppercase();

                                            let is_edit_mode = move || is_editing_categories.get();

                                            view! {
                                                <li>
                                                    <Show when=is_edit_mode fallback={
                                                        let cat_fallback = cat_clone.clone();
                                                        let display_fallback = display_name.clone();
                                                        move || {
                                                            let c_click = cat_fallback.clone();
                                                            let c_active = cat_fallback.clone();
                                                            view! {
                                                                // Standard View
                                                                <a class:active=move || active_category.get() == c_active
                                                                   class="font-bold tracking-widest text-[10px]"
                                                                   on:click=move |_| set_active_category.set(c_click.clone())>
                                                                    {display_fallback.clone()}
                                                                </a>
                                                            }.into_any()
                                                        }
                                                    }>
                                                        // Edit View
                                                        <Show when={
                                                            let c_rename_check = cat_clone.clone();
                                                            move || renaming_category.get().as_ref() == Some(&c_rename_check)
                                                        } fallback={
                                                            let cat_edit_fb = cat_clone.clone();
                                                            let display_edit_fb = display_name.clone();
                                                            move || {
                                                                let c_rename = cat_edit_fb.clone();
                                                                let c_del = cat_edit_fb.clone();
                                                                view! {
                                                                    // Renaming Action Buttons
                                                                    <div class="flex justify-between items-center w-full pr-1 bg-base-content/5">
                                                                        <span class="font-bold tracking-widest text-[10px] truncate">{display_edit_fb.clone()}</span>
                                                                        <div class="flex gap-1">
                                                                            <button class="btn btn-ghost btn-xs h-5 min-h-0 px-1 hover:bg-info/20 hover:text-info"
                                                                                title="이름 변경"
                                                                                on:click=move |_| {
                                                                                set_edit_category_name.set(c_rename.clone());
                                                                                set_renaming_category.set(Some(c_rename.clone()));
                                                                            }>"✏️"</button>
                                                                            <button class="btn btn-ghost btn-xs h-5 min-h-0 px-1 text-error/70 hover:bg-error/20 hover:text-error"
                                                                                title="삭제"
                                                                                on:click=move |_| delete_category(c_del.clone())>"🗑️"</button>
                                                                        </div>
                                                                    </div>
                                                                }.into_any()
                                                            }
                                                        }>
                                                            // Renaming Input Field
                                                            <input type="text" class="input input-xs input-bordered w-full bg-base-100 font-bold text-[10px]"
                                                                autofocus
                                                                prop:value=move || edit_category_name.get()
                                                                on:input=move |ev| set_edit_category_name.set(event_target_value(&ev))
                                                                on:blur=move |_| commit_category_rename()
                                                                on:keydown=move |ev| {
                                                                    if ev.key() == "Enter" { commit_category_rename(); }
                                                                    else if ev.key() == "Escape" { set_renaming_category.set(None); }
                                                                }
                                                            />
                                                        </Show>
                                                    </Show>
                                                </li>
                                            }
                                        }
                                    />
                                </ul>
                            </div>

                            // Bottom Action Area: Category Controls
                            <div class="mt-2 pt-2 border-t border-base-content/10 flex flex-col gap-1">
                                <Show when=move || is_adding_category.get() fallback=move || view! {
                                    <button class="btn btn-ghost btn-xs w-full text-base-content/70 hover:text-success border border-dashed border-base-content/20"
                                        disabled=move || is_editing_categories.get()
                                        on:click=move |_| set_is_adding_category.set(true)>
                                        "+ 카테고리 추가"
                                    </button>
                                    <button class="btn btn-ghost btn-xs w-full text-base-content/70 hover:text-warning border border-dashed border-base-content/20"
                                        class:bg-warning=move || is_editing_categories.get()
                                        class:text-warning-content=move || is_editing_categories.get()
                                        class:hover:text-warning-content=move || is_editing_categories.get()
                                        on:click=move |_| {
                                            let current = is_editing_categories.get_untracked();
                                            set_is_editing_categories.set(!current);
                                            set_renaming_category.set(None); // Ensure inline edits are closed when toggling
                                        }>
                                        {move || if is_editing_categories.get() { "편집 완료" } else { "카테고리 편집" }}
                                    </button>
                                }>
                                    <input type="text" class="input input-xs input-bordered w-full bg-base-100"
                                        autofocus
                                        placeholder="이름 (Enter)"
                                        prop:value=move || new_category_name.get()
                                        on:input=move |ev| set_new_category_name.set(event_target_value(&ev))
                                        on:blur=move |_| set_is_adding_category.set(false)
                                        on:keydown=move |ev| {
                                            if ev.key() == "Enter" { add_category(); }
                                            else if ev.key() == "Escape" { set_is_adding_category.set(false); }
                                        }
                                    />
                                </Show>
                            </div>
                        </div>

                        // MAIN PANEL: Words List
                        <div class="flex-1 flex flex-col bg-base-100 overflow-hidden">

                            // Add New Word Row
                            <div class="p-3 bg-base-200 border-b border-base-content/5 flex gap-2">
                                <input type="text" class="input input-xs input-bordered flex-1" placeholder="원문 (예: よろです)"
                                    prop:value=move || new_key.get()
                                    on:input=move |ev| set_new_key.set(event_target_value(&ev)) />
                                <input type="text" class="input input-xs input-bordered flex-1" placeholder="번역 (예: 잘부탁해요)"
                                    prop:value=move || new_val.get()
                                    on:input=move |ev| set_new_val.set(event_target_value(&ev)) />
                                <button class="btn btn-xs btn-success font-bold"
                                    on:click=move |_| {
                                        let k = new_key.get_untracked().trim().to_string();
                                        let v = new_val.get_untracked().trim().to_string();
                                        let cat = active_category.get_untracked();
                                        if !k.is_empty() && !v.is_empty() {
                                            set_dict.update(|d| {
                                                d.entry(cat).or_default().insert(k, v);
                                            });
                                            set_new_key.set(String::new());
                                            set_new_val.set(String::new());
                                        }
                                    }>
                                    "단어 추가"
                                </button>
                            </div>

                            // Word Table
                            <div class="flex-1 overflow-y-auto custom-scrollbar p-0">
                                <table class="table table-xs table-pin-rows table-fixed w-full">
                                    <thead class="bg-base-300">
                                        <tr>
                                            <th>"원문 (키)"</th>
                                            <th>"번역 (값)"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {move || {
                                            let current_cat = active_category.get();
                                            let current_dict = dict.get();

                                            if let Some(words) = current_dict.get(&current_cat) {
                                                words.iter().map(|(k, v)| {
                                                    let key_display = k.clone();
                                                    let val_display = v.clone();
                                                    let cat_del = current_cat.clone();
                                                    let key_del = k.clone();

                                                    // Closures to check if THIS specific row is being edited
                                                    let cat_k = current_cat.clone();
                                                    let orig_k = k.clone();
                                                    let is_editing_key = move || {
                                                        editing_target.get().map(|(c, ok, f)| c == cat_k && ok == orig_k && f == "key").unwrap_or(false)
                                                    };

                                                    let cat_v = current_cat.clone();
                                                    let orig_v = k.clone();
                                                    let is_editing_val = move || {
                                                        editing_target.get().map(|(c, ok, f)| c == cat_v && ok == orig_v && f == "value").unwrap_or(false)
                                                    };

                                                    view! {
                                                        <tr class="hover:bg-base-200/50 transition-colors">

                                                            // --- INLINE EDIT: KEY ---
                                                            <td class="font-mono text-xs max-w-xs truncate cursor-pointer hover:bg-base-content/10"
                                                                title="클릭하여 수정"
                                                                on:click={
                                                                    let c = current_cat.clone();
                                                                    let ok = k.clone();
                                                                    let text = k.clone();
                                                                    move |_| {
                                                                        let target = editing_target.get_untracked();
                                                                        let is_currently_editing = target.as_ref().map(|(cat, o_k, f)| cat == &c && o_k == &ok && f == "key").unwrap_or(false);

                                                                        if !is_currently_editing {
                                                                            set_edit_value.set(text.clone());
                                                                            set_editing_target.set(Some((c.clone(), ok.clone(), "key".to_string())));
                                                                        }
                                                                    }
                                                                }
                                                            >
                                                                <Show when=is_editing_key fallback=move || view! { {key_display.clone()} }>
                                                                    <input type="text" class="input input-xs input-bordered w-full bg-base-100"
                                                                        autofocus
                                                                        prop:value=move || edit_value.get()
                                                                        on:input=move |ev| set_edit_value.set(event_target_value(&ev))
                                                                        on:blur=move |_| commit_edit()
                                                                        on:keydown=move |ev| {
                                                                            if ev.key() == "Enter" { commit_edit(); }
                                                                            else if ev.key() == "Escape" { set_editing_target.set(None); }
                                                                        }
                                                                    />
                                                                </Show>
                                                            </td>

                                                            // --- INLINE EDIT: VALUE ---
                                                            <td class="text-xs max-w-xs truncate cursor-pointer hover:bg-base-content/10"
                                                                title="클릭하여 수정"
                                                                on:click={
                                                                    let c = current_cat.clone();
                                                                    let ok = k.clone();
                                                                    let text = v.clone();
                                                                    move |_| {
                                                                        let target = editing_target.get_untracked();
                                                                        let is_currently_editing = target.as_ref().map(|(cat, o_k, f)| cat == &c && o_k == &ok && f == "value").unwrap_or(false);

                                                                        if !is_currently_editing {
                                                                            set_edit_value.set(text.clone());
                                                                            set_editing_target.set(Some((c.clone(), ok.clone(), "value".to_string())));
                                                                        }
                                                                    }
                                                                }
                                                            >
                                                                <Show when=is_editing_val fallback=move || view! { {val_display.clone()} }>
                                                                    <input type="text" class="input input-xs input-bordered w-full bg-base-100"
                                                                        autofocus
                                                                        prop:value=move || edit_value.get()
                                                                        on:input=move |ev| set_edit_value.set(event_target_value(&ev))
                                                                        on:blur=move |_| commit_edit()
                                                                        on:keydown=move |ev| {
                                                                            if ev.key() == "Enter" { commit_edit(); }
                                                                            else if ev.key() == "Escape" { set_editing_target.set(None); }
                                                                        }
                                                                    />
                                                                </Show>
                                                            </td>

                                                            // --- ACTION: DELETE ---
                                                            <td class="text-center">
                                                                <button class="btn btn-ghost btn-xs text-error hover:bg-error/20"
                                                                    on:click=move |_| delete_word(cat_del.clone(), key_del.clone())>
                                                                    "삭제"
                                                                </button>
                                                            </td>
                                                        </tr>
                                                    }
                                                }).collect_view().into_any()
                                            } else {
                                                view! { <tr><td colspan="3" class="text-center py-4 opacity-50">"비어 있습니다."</td></tr> }.into_any()
                                            }
                                        }}
                                    </tbody>
                                </table>
                            </div>
                        </div>
                    </div>

                    // --- FOOTER ---
                    <div class="bg-base-200 p-3 border-t border-base-content/5 flex justify-between items-center">
                        <span class="text-[10px] text-warning">"⚠️ 서버에서 사전을 동기화하면 수정한 내용이 초기화될 수 있습니다."</span>
                        <div class="flex gap-2">
                            <button class="btn btn-ghost btn-sm" on:click=move |_| signals.ui.set_show_dictionary.set(false)>"취소"</button>
                            <button class="btn btn-success btn-sm font-bold shadow-lg" on:click=save_dict>"수정 사항 저장"</button>
                        </div>
                    </div>

                </div>
                <div class="modal-backdrop bg-black/40" on:click=move |_| signals.ui.set_show_dictionary.set(false)></div>
            </div>
        </Show>
    }
}
