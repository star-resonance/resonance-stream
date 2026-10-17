use crate::store::{AppActions, AppSignals};
use crate::tauri_bridge::call;
use crate::ui_types::SnifferState;
use leptos::context::use_context;
use leptos::control_flow::Show;
use leptos::prelude::{signal, ClassAttribute, Get, IntoAny, OnAttribute, Set};
use leptos::prelude::{ElementChild, GetUntracked};
use leptos::task::spawn_local;
use leptos::*;
use resonance_types::ipc::{EnsureFirewallRule, GetNetworkInterfaces, RestartSniffer};
use wasm_bindgen_futures::JsFuture;

// Helper to pause the async loop without blocking the UI
async fn delay(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(window) = web_sys::window() {
            // Tells the browser's window to wait 'ms' milliseconds, then call resolve()
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
        }
    });
    // Wait for the JS promise to finish!
    let _ = JsFuture::from(promise).await;
}

/// Where the adapter scan is; only this dialog reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScanStatus {
    Idle,
    Scanning,
    Success,
    Fail,
}

#[component]
pub fn Troubleshooter() -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("Signals missing");
    let actions = use_context::<AppActions>().expect("Actions missing");

    let (status, set_status) = signal(ScanStatus::Idle);
    let (current_test, set_current_test) = signal("".to_string());
    let (progress, set_progress) = signal(0.0);

    let start_scan = move |_| {
        spawn_local(async move {
            set_status.set(ScanStatus::Scanning);
            set_progress.set(0.0);

            if let Ok(list) = call(&GetNetworkInterfaces {}).await {
                for (i, iface) in list.iter().enumerate() {
                    set_current_test.set(format!("{} ({})", iface.name, iface.ip));
                    set_progress.set((i as f64 / list.len() as f64) * 100.0);

                    // 1. Temporarily save this adapter to config
                    signals.config.set_network_interface.set(iface.ip.clone());
                    actions.save_config.dispatch(());

                    // 2. Restart the backend sniffer!
                    let _ = call(&RestartSniffer {}).await;

                    // 3. Reset elapsed time
                    let mut elapsed = 0;
                    let timeout_ticks = 50; // 5 seconds

                    while elapsed < timeout_ticks {
                        delay(100).await;

                        // WOW! We just check if the state flipped to Active!
                        // As long as the game is running, background packets will trigger this instantly!
                        if signals.service.sniffer_state.get_untracked() == SnifferState::Active {
                            set_status.set(ScanStatus::Success);
                            set_progress.set(100.0);
                            return; // We found the working adapter!
                        }
                        elapsed += 1;
                    }
                }
            }

            // If we loop through everything and nothing worked...
            set_status.set(ScanStatus::Fail);
            set_progress.set(100.0);
            signals.config.set_network_interface.set("".to_string()); // Reset to auto
            actions.save_config.dispatch(());
            let _ = call(&RestartSniffer {}).await; // Restart in auto mode
        });
    };

    let fix_firewall = move |_| {
        spawn_local(async move {
            // 1. Call the backend to trigger the UAC prompt and overwrite the rule
            if call(&EnsureFirewallRule {}).await.is_ok() {
                // 2. If the user clicked "Yes" on the Admin prompt, restart the sniffer!
                let _ = call(&RestartSniffer {}).await;

                // 3. Close the troubleshooter so they can see the main UI reconnecting
                signals.ui.set_show_troubleshooter.set(false);
            }
        });
    };

    view! {
        <Show when=move || signals.ui.show_troubleshooter.get()>
            <div class="modal modal-open backdrop-blur-sm z-[30000]">
                <div class="modal-box bg-base-300 border border-base-content/10 shadow-2xl w-full max-w-md p-6">

                    {move || match status.get() {
                        ScanStatus::Idle => view! {
                            <div class="space-y-4">
                                // --- STEP 1: FIREWALL (Primary Action) ---
                                <div class="bg-error/10 p-3 rounded-lg border border-error/20">
                                    <h3 class="text-sm font-bold text-error mb-1">"1단계: 방화벽 규칙 초기화"</h3>
                                    <p class="text-[11px] text-base-content/80 mb-3 leading-relaxed">
                                        "가장 흔한 원인입니다. 윈도우 업데이트나 백신 프로그램에 의해 삭제된 방화벽 규칙을 다시 생성합니다."
                                    </p>
                                    <button class="btn btn-error btn-sm btn-block shadow-sm" on:click=fix_firewall>
                                        "방화벽 규칙 재설정 (관리자 권한 필요)"
                                    </button>
                                </div>

                                // --- STEP 2: ADAPTER SCAN (Secondary Action) ---
                                <div class="bg-warning/10 p-3 rounded-lg border border-warning/20">
                                    <h3 class="text-sm font-bold text-warning mb-1">"2단계: 네트워크 어댑터 검사"</h3>
                                    <p class="text-[11px] text-base-content/80 mb-3 leading-relaxed">
                                        "방화벽을 재설정해도 해결되지 않는다면, VPN이나 가상 어댑터와의 충돌일 수 있습니다. 게임이 켜진 상태에서 클릭하세요."
                                    </p>
                                    <button class="btn btn-warning btn-sm btn-block shadow-sm" on:click=start_scan>
                                        "어댑터 자동 스캔 시작"
                                    </button>
                                </div>
                            </div>
                        }.into_any(),

                        ScanStatus::Scanning => view! {
                            <div class="space-y-4 text-center py-4">
                                <div class="text-xs font-bold text-success animate-pulse mb-2">"⚠️ 중요: 게임이 켜져 있는지 확인해 주세요!"</div>

                                <progress class="progress progress-warning w-full" value=move || progress.get().to_string() max="100"></progress>

                                <div class="bg-base-200 p-3 rounded-lg border border-base-content/10">
                                    <div class="text-[10px] text-base-content/50 uppercase font-bold mb-1">"현재 테스트 중인 어댑터"</div>
                                    <div class="text-sm font-mono text-warning break-all">{move || current_test.get()}</div>
                                </div>
                                <p class="text-[10px] opacity-50">"각 어댑터마다 최대 5초씩 대기하며 게임 패킷을 감지합니다..."</p>
                            </div>
                        }.into_any(),

                        ScanStatus::Success => view! {
                            <div class="space-y-4 text-center py-2">
                                <div class="text-4xl mb-2">"🎉"</div>
                                <h3 class="text-lg font-bold text-success">"어댑터 복구 완료!"</h3>
                                <p class="text-xs">"성공적으로 게임 채팅을 감지했습니다. 설정이 자동으로 저장되었습니다."</p>
                                <div class="text-[10px] bg-base-200 p-2 rounded text-success font-mono">{move || current_test.get()}</div>
                                <button class="btn btn-success btn-block mt-4" on:click=move |_| signals.ui.set_show_troubleshooter.set(false)>"닫기"</button>
                            </div>
                        }.into_any(),

                        ScanStatus::Fail => view! {
                            <div class="space-y-4 text-center py-2">
                                <div class="text-4xl mb-2">"❌"</div>
                                <h3 class="text-lg font-bold text-error">"감지 실패"</h3>
                                <p class="text-xs leading-relaxed">"모든 네트워크 어댑터를 확인했지만 게임 트래픽을 찾지 못했습니다. 게임이 켜져있고 로그인 된 상태인지 확인해주세요."</p>

                                <div class="flex gap-2 mt-4">
                                    <button class="btn btn-ghost flex-1" on:click=move |_| signals.ui.set_show_troubleshooter.set(false)>"닫기"</button>
                                    <button class="btn btn-warning flex-1" on:click=start_scan>"다시 시도"</button>
                                </div>
                            </div>
                        }.into_any(),

                    }}
                </div>
                <div class="modal-backdrop bg-black/50" on:click=move |_| {
                    if status.get() != ScanStatus::Scanning { signals.ui.set_show_troubleshooter.set(false); }
                }></div>
            </div>
        </Show>
    }
}
