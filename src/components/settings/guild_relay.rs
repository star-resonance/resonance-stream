use crate::relay_view::status_line;
use crate::store::{AppActions, AppSignals};
use leptos::prelude::*;

/// The guild relay: the switch, the broker URL (empty: the built-in default) and the status line.
#[component]
pub fn GuildRelaySection() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("AppSignals missing");
    let actions = use_context::<AppActions>().expect("AppActions missing");
    let relay = signals.config.guild_relay;
    let set_relay = signals.config.set_guild_relay;

    let set_broker_url = move |url: String| {
        if relay.with_untracked(|r| r.broker_url != url) {
            set_relay.update(|r| r.broker_url = url);
            actions.save_config.dispatch(());
        }
    };

    view! {
        <section class="space-y-4">
            <div class="form-control bg-base-200 p-3 rounded-lg border border-base-content/5">
                <label class="label cursor-pointer p-0 w-full justify-between gap-3">
                    <div class="flex flex-col">
                        <span class="label-text text-xs font-bold text-base-content/80">"길드 채팅을 디스코드로 보내기"</span>
                        <span class="text-[9px] text-base-content/60 mt-1">
                            "GUILD 채널의 실시간 메시지만, 번역 없이 원문과 닉네임 그대로 보냅니다. 차단 목록과 관계없이 보냅니다."
                        </span>
                    </div>
                    <input type="checkbox" class="toggle toggle-success toggle-sm"
                        prop:checked=move || relay.get().enabled
                        on:change=move |ev| {
                            let on = event_target_checked(&ev);
                            set_relay.update(|r| r.enabled = on);
                            actions.save_config.dispatch(());
                        }
                    />
                </label>
            </div>

            <div class="space-y-2 mt-4 pt-4 border-t border-base-content/10">
                <div class="text-[11px] font-bold text-base-content/80">"브로커 주소 (Broker URL)"</div>
                <div class="flex gap-2">
                    <input type="url" class="input input-sm input-bordered flex-1 min-w-0 font-mono text-xs"
                        placeholder="기본 브로커 사용"
                        spellcheck="false"
                        prop:value=move || relay.get().broker_url
                        on:change=move |ev| set_broker_url(event_target_value(&ev).trim().to_string())
                    />
                    <button class="btn btn-sm btn-ghost border border-base-content/10"
                        disabled=move || relay.get().broker_url.is_empty()
                        on:click=move |_| set_broker_url(String::new())
                    >
                        "기본값으로"
                    </button>
                </div>
                <div class="text-[9px] text-base-content/50">
                    "비워 두면 기본 브로커를 사용합니다. 다른 길드는 자신의 브로커 주소(https://)를 입력할 수 있습니다."
                </div>
            </div>

            <div class="flex items-start gap-2 bg-base-200 p-3 rounded-lg border border-base-content/5">
                <span class="text-[11px] font-bold text-base-content/80 shrink-0">"상태"</span>
                {move || {
                    let (text, tone) = status_line(
                        signals.service.guild_relay_state.get(),
                        &relay.get().broker_url,
                    );
                    view! { <span class=format!("text-[11px] font-semibold {}", tone.class())>{text}</span> }
                }}
            </div>
        </section>
    }
}
