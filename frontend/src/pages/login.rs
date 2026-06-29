use crate::components::utils::{ComponentSize, Spinner};
use crate::refresh_encryption_status;
use clipboard_history::constants::WIPE_CONFIRMATION_PHRASE;
use clipboard_history::{AppError, EncryptionStatus};
use icondata as i;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use tauri_sys::core::invoke_result;

#[component]
pub fn Login() -> impl IntoView {
    let password = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());
    let status = expect_context::<RwSignal<Option<EncryptionStatus>>>();

    let confirming_wipe = RwSignal::new(false);

    let unlock = Action::new_local(move |password: &String| {
        let password = password.clone();
        async move {
            match invoke_result::<(), AppError>("unlock", &serde_json::json!({ "password": password })).await {
                Ok(_) => {
                    error.set(String::new());
                    refresh_encryption_status(status);
                }
                Err(e) => error.set(e.to_string()),
            }
        }
    });

    view! {
        <div class="flex min-h-screen items-center justify-center">
            <div class="card w-full max-w-sm p-8">
                <div class="flex flex-col items-center mb-6 gap-2 text-center">
                    <Icon icon=i::LuLock />
                    <h1 class="text-lg font-semibold tracking-tight">"Locked"</h1>
                    <p class="text-sm text-text/55">"Enter your password to view clipboard history."</p>
                </div>

                <form class="space-y-3" on:submit=move |ev| {
                    ev.prevent_default();

                    let pw = password.get();
                    if pw.is_empty() {
                        return;
                    }

                    unlock.dispatch(pw);
                }>
                    <input
                        class="field"
                        type="password"
                        placeholder="Password"
                        bind:value=password
                    />
                    <button class="btn btn-primary w-full" type="submit">
                        {move || {
                                if unlock.pending().get() {
                                    view! { <Spinner component_size=ComponentSize::Small /> }.into_any()
                                } else {
                                    "Unlock".into_any()
                                }
                            }
                        }
                    </button>
                </form>

                <Show when=move || !error.get().is_empty()>
                    <p class="text-sm text-red-500">{move || error.get()}</p>
                </Show>

                <div class="my-6 h-px bg-input-border/70"></div>

                <Show when=move || confirming_wipe.get()>
                    <div class="rounded-lg border border-red-500/30 bg-red-500/5 p-4">
                        <p class="text-sm text-red-500">
                            "This will permanently delete all clipboard history. Continue?"
                        </p>
                        <div class="mt-3 flex gap-2">
                            <button class="btn btn-danger btn-sm border border-red-500/30" on:click=move |_| {
                                spawn_local(async move {
                                    if invoke_result::<(), AppError>("wipe_and_reset", &serde_json::json!({
                                        "confirmation": WIPE_CONFIRMATION_PHRASE,
                                        // "new_password": new_password
                                    })).await.is_ok() {
                                        refresh_encryption_status(status);
                                    }
                                });
                            }>"Confirm"</button>
                            <button class="btn btn-ghost btn-sm" on:click=move |_| confirming_wipe.set(false)>"Cancel"</button>
                        </div>
                    </div>
                </Show>

                <Show when=move || !confirming_wipe.get()>
                    <div class="text-center">
                        <a
                            class="text-xs text-text/50 hover:text-text hover:underline"
                            href="#"
                            on:click=move |ev| {
                                ev.prevent_default();
                                confirming_wipe.set(true);
                            }
                        >"Forgot password?"</a>
                    </div>
                </Show>
            </div>
        </div>
    }
}
