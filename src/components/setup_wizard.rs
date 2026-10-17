use crate::store::AppSignals;
use crate::tauri_bridge::call;
use crate::ui_types::ComputeMode;
use leptos::prelude::*;
use leptos::task::spawn_local;
use resonance_types::ipc::EnsureFirewallRule;

#[component]
pub fn SetupWizard(
    finalize: Callback<()>,
    start_download: Callback<web_sys::MouseEvent>,
) -> impl IntoView {
    let signals = use_context::<AppSignals>().expect("Signals missing");

    let (firewall_agreed, set_firewall_agreed) = signal(false);

    view! {
        // Fills what the title bar leaves and scrolls inside it; the inner wrapper keeps the
        // card centred when it fits and lets it start at the top when it does not.
        <div class="flex-1 min-h-0 overflow-y-auto bg-base-100 p-6">
          <div class="min-h-full flex items-center justify-center">
            <div class="card w-full max-w-md bg-base-200 shadow-2xl border border-white/5">
                <div class="card-body gap-6">
                    // --- PROGRESS STEPS ---
                    <ul class="steps steps-horizontal w-full mb-4">
                        <li class="step step-success"></li>
                        <li class=move || format!("step {}", if signals.setup.wizard_step.get() >= 1 { "step-success" } else { "" })></li>
                        <li class=move || format!("step {}", if signals.setup.wizard_step.get() >= 2 { "step-success" } else { "" })></li>
                    </ul>

                    {move || match signals.setup.wizard_step.get() {
                        0 => view! {
                            <div class="space-y-4 animate-in fade-in slide-in-from-bottom-4 text-left">
                                <h1 class="text-3xl font-black tracking-tighter text-success text-center">"RESONANCE STREAM"</h1>
                                <p class="text-sm opacity-70 text-center">"블루 프로토콜의 게임 채팅을 실시간으로 분석하고 번역합니다."</p>

                                // FIREWALL WARNING BOX
                                <div class="bg-warning/10 border border-warning/30 p-4 rounded-xl mt-4 space-y-3">
                                    <div class="flex items-center gap-2 text-warning font-bold text-sm">
                                        <svg class="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24"><path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"></path></svg>
                                        <span>"필수 권한 안내 (방화벽 및 관리자 권한)"</span>
                                    </div>
                                    <p class="text-xs text-base-content/80 leading-relaxed break-keep">
                                        "게임 채팅 데이터를 실시간으로 감지하기 위해 네트워크 패킷을 분석합니다. 이를 위해 실행 시 "<span class="text-warning font-bold">"Windows 방화벽 규칙 추가"</span>" 및 "<span class="text-warning font-bold">"관리자 권한"</span>"이 요구될 수 있습니다."
                                    </p>

                                    // Privacy Guarantee (Builds Trust)
                                    <div class="bg-base-300/50 p-2 rounded flex gap-2 items-start mt-2">
                                        <span class="text-success mt-0.5">"🔒"</span>
                                        <span class="text-[10px] text-base-content/70">
                                            <b>"프라이버시 보장:"</b> " 본 프로그램은 오직 '블루 프로토콜'의 채팅 패킷만을 읽어오며, 사용자의 개인 정보나 웹 브라우징 기록 등은 절대 수집하지 않습니다."
                                        </span>
                                    </div>

                                    // Pre-Prompt Heads Up
                                    <div class="text-[10px] font-bold text-error/80 italic">
                                        "※ '다음' 버튼을 누른 후 나타나는 Windows 보안 경고창에서 반드시 '허용'을 눌러주세요."
                                    </div>
                                </div>

                                // --- NEW: AGREEMENT CHECKBOX ---
                                <div class="form-control mt-2">
                                    <label class="label cursor-pointer justify-start gap-3 border border-base-content/10 p-3 rounded-lg hover:bg-base-content/5 transition-colors">
                                        <input type="checkbox" class="checkbox checkbox-success checkbox-sm"
                                            prop:checked=move || firewall_agreed.get()
                                            on:change=move |ev| set_firewall_agreed.set(event_target_checked(&ev))
                                        />
                                        <span class="label-text font-bold text-sm">"방화벽 규칙 자동 변경에 동의합니다."</span>
                                    </label>
                                </div>

                                <div class="card-actions justify-end mt-4">
                                    <button class="btn btn-success btn-block"
                                        disabled=move || !firewall_agreed.get()
                                        on:click=move |_| {
                                            // 1. Temporarily show a loading state (optional, but good UX)
                                            set_firewall_agreed.set(false);

                                            // 2. Call the backend to trigger the UAC prompt
                                            spawn_local(async move {
                                                match call(&EnsureFirewallRule {}).await {
                                                    Ok(_) => {
                                                        // 3a. User clicked YES! Move to the next step.
                                                        signals.setup.set_wizard_step.set(1);
                                                    },
                                                    Err(_) => {
                                                        // 3b. User clicked NO!
                                                        // Inform them they must accept it, and leave them on Step 0
                                                        if let Some(w) = web_sys::window() {
                                                            let _ = w.alert_with_message("방화벽 설정 권한이 거부되었습니다. 게임 채팅을 가져오기 위해서는 방화벽 설정이 필요합니다.");
                                                        }
                                                        set_firewall_agreed.set(false);
                                                    }
                                                }
                                            });
                                        }>
                                        <svg class="w-4 h-4 mr-1" fill="currentColor" viewBox="0 0 20 20"><path fill-rule="evenodd" d="M10 1.944A11.954 11.954 0 012.166 5C2.056 5.649 2 6.319 2 7c0 5.225 3.34 9.67 8 11.317C14.66 16.67 18 12.225 18 7c0-.682-.057-1.35-.166-2.001A11.954 11.954 0 0110 1.944zM11 14a1 1 0 11-2 0 1 1 0 012 0zm0-7a1 1 0 10-2 0v3a1 1 0 102 0V7z" clip-rule="evenodd"></path></svg>
                                        "동의하고 시작하기"
                                    </button>
                                </div>
                            </div>
                        }.into_any(),

                        1 => view! {
                            <div class="space-y-4 animate-in fade-in">
                                <h2 class="text-lg font-bold">"빠른 설정"</h2>
                                <div class="form-control bg-base-300 p-4 rounded-xl border border-white/5">
                                    <label class="label cursor-pointer">
                                        <span class="label-text font-bold">"실시간 번역 활성화"</span>
                                        <input type="checkbox" class="toggle toggle-success"
                                            prop:checked=move || signals.config.use_translation.get()
                                            on:change=move |ev| signals.config.set_use_translation.set(event_target_checked(&ev)) />
                                    </label>
                                </div>
                                <Show when=move || signals.config.use_translation.get()>
                                    <div class="space-y-2">
                                        <span class="text-xs font-bold opacity-50 uppercase">"연산 장치 (Compute Mode)"</span>
                                        <div class="join w-full">
                                            <button class="join-item btn btn-sm flex-1"
                                                class:btn-success=move || signals.config.compute_mode.get() == ComputeMode::Cpu
                                                on:click=move |_| signals.config.set_compute_mode.set(ComputeMode::Cpu)>"CPU"</button>
                                            <button class="join-item btn btn-sm flex-1"
                                                class:btn-success=move || signals.config.compute_mode.get() == ComputeMode::Gpu
                                                on:click=move |_| signals.config.set_compute_mode.set(ComputeMode::Gpu)>"GPU"</button>
                                        </div>
                                    </div>
                                </Show>
                                <button class="btn btn-success btn-block"
                                    on:click=move |_| if signals.config.use_translation.get_untracked() { signals.setup.set_wizard_step.set(2) } else { finalize.run(()) }>
                                    "다음"
                                </button>
                            </div>
                        }.into_any(),

                        2 => view! {
                            <div class="space-y-4 text-center">
                                <h2 class="text-lg font-bold">"AI 모델 설치"</h2>
                                <p class="text-xs opacity-60">"번역을 위해 약 2.4GB의 AI 모델 파일 다운로드가 필요합니다."</p>
                                <Show when=move || signals.setup.downloading.get() fallback=move || view! {
                                    <button class="btn btn-success btn-block" on:click=move |ev| start_download.run(ev)>"다운로드 시작"</button>
                                }>
                                    <progress class="progress progress-success w-full h-4" value=move || signals.setup.progress.get().to_string() max="100"></progress>
                                    <span class="text-xs font-mono">{move || format!("{}%", signals.setup.progress.get())}</span>
                                </Show>
                                // Why the last try stopped (a refused signature, no network, a failed download).
                                <Show when=move || signals.setup.setup_error.get().is_some()>
                                    <div role="alert" class="alert alert-error text-left text-xs break-words">
                                        <span>{move || signals.setup.setup_error.get().unwrap_or_default()}</span>
                                    </div>
                                </Show>
                            </div>
                        }.into_any(),
                        _ => view! { <div></div> }.into_any(),
                    }}
                </div>
            </div>
          </div>
        </div>
    }
}
