use crate::{Page, refresh_encryption_status};
use clipboard_history::{AppError, AppSettings, EncryptionStatus, RetentionUnit};
use icondata as i;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use leptos_use::ColorMode;
use tauri_sys::core::invoke_result;

#[component]
pub fn Settings() -> impl IntoView {
    let password = RwSignal::new(String::new());
    let password_confirm = RwSignal::new(String::new());
    let error = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let screen = expect_context::<RwSignal<Page>>();
    let encryption_status = expect_context::<RwSignal<Option<EncryptionStatus>>>();

    let color_mode = use_context::<Signal<ColorMode>>().unwrap_or_default();
    let set_color_mode = expect_context::<WriteSignal<ColorMode>>();

    let retention_amount = RwSignal::new(0u64);
    let retention_unit = RwSignal::new(RetentionUnit::Days);
    let retention_status = RwSignal::new(String::new());

    let auto_lock_minutes = expect_context::<RwSignal<u64>>();
    let auto_lock_status = RwSignal::new(String::new());

    spawn_local(async move {
        if let Ok(s) = invoke_result::<AppSettings, AppError>("load_settings", &()).await {
            retention_amount.set(s.retention_amount);
            retention_unit.set(s.retention_unit);
            auto_lock_minutes.set(s.auto_lock_minutes);
        }
    });

    view! {
        <div class="mx-auto max-w-2xl px-6 py-8">
            <header class="mb-8 flex items-center justify-between gap-4">
                <div class="flex items-center gap-3">
                    <Icon icon=i::LuSettings />
                    <h1 class="text-xl font-semibold tracking-tight">"Settings"</h1>
                </div>
                <div class="flex items-center gap-1.5">
                    <button class="btn btn-secondary" on:click=move |_| {
                        if color_mode.get() == ColorMode::Light { set_color_mode.set(ColorMode::Dark) } else { set_color_mode.set(ColorMode::Light) }
                    }>
                        {move || {
                            if color_mode.get() == ColorMode::Light {
                                view! {
                                    <Icon icon=i::LuMoon />
                                    "Dark"
                                }.into_any()
                            } else {
                                view! {
                                    <Icon icon=i::LuSun />
                                    "Light"
                                }.into_any()
                            }
                        }}
                    </button>
                    <button class="btn btn-ghost" on:click=move |_| screen.set(Page::Home)>"Back"</button>
                </div>
            </header>

            <div class="space-y-8">
                <section class="space-y-3">
                    <h2 class="text-base font-semibold tracking-tight">"Retention"</h2>
                    <p class="text-sm text-text/55">"Clipboard items older than this will be deleted. Set amount to 0 to keep forever."</p>
                    <div class="flex items-center gap-2">
                        <input
                            class="field w-20"
                            type="number"
                            min="0"
                            prop:value=move || retention_amount.get().to_string()
                            on:input=move |ev| {
                                let value = event_target_value(&ev);
                                retention_amount.set(value.parse::<u64>().unwrap_or(0));
                            }
                        />
                        <select
                            class="field w-auto"
                            prop:value=move || match retention_unit.get() {
                                RetentionUnit::Minutes => "Minutes",
                                RetentionUnit::Hours => "Hours",
                                RetentionUnit::Days => "Days",
                            }
                            on:change=move |ev| {
                                let unit = match event_target_value(&ev).as_str() {
                                    "Minutes" => RetentionUnit::Minutes,
                                    "Hours" => RetentionUnit::Hours,
                                    _ => RetentionUnit::Days,
                                };
                                retention_unit.set(unit);
                            }
                        >
                            <option value="Minutes">"Minutes"</option>
                            <option value="Hours">"Hours"</option>
                            <option value="Days">"Days"</option>
                        </select>
                        <button class="btn btn-primary" on:click=move |_| {
                            let amount = retention_amount.get();
                            let unit = retention_unit.get();
                            spawn_local(async move {
                                match invoke_result::<(), AppError>("set_retention", &serde_json::json!({
                                    "amount": amount,
                                    "unit": unit
                                })).await {
                                    Ok(_) => retention_status.set("Saved.".into()),
                                    Err(e) => retention_status.set(e.to_string()),
                                }
                            });
                        }>"Save"</button>
                        <Show when=move || !retention_status.get().is_empty()>
                            <span class="text-xs text-text/50">{move || retention_status.get()}</span>
                        </Show>
                    </div>
                </section>

                <div class="h-px bg-input-border/60"></div>

                <section class="space-y-3">
                    <h2 class="text-base font-semibold tracking-tight">"Auto-lock"</h2>
                    <p class="text-sm text-text/55">"Lock the vault after this many minutes of inactivity. Set to 0 to disable. Only applies when encryption is enabled."</p>
                    <div class="flex items-center gap-2">
                        <input
                            class="field w-20"
                            type="number"
                            min="0"
                            prop:value=move || auto_lock_minutes.get().to_string()
                            on:input=move |ev| {
                                let value = event_target_value(&ev);
                                auto_lock_minutes.set(value.parse::<u64>().unwrap_or(0));
                            }
                        />
                        <span class="text-sm text-text/60">"minutes"</span>
                        <button class="btn btn-primary" on:click=move |_| {
                            let minutes = auto_lock_minutes.get();
                            spawn_local(async move {
                                match invoke_result::<(), AppError>("set_auto_lock", &serde_json::json!({ "minutes": minutes })).await {
                                    Ok(_) => auto_lock_status.set("Saved.".into()),
                                    Err(e) => auto_lock_status.set(e.to_string()),
                                }
                            });
                        }>"Save"</button>
                        <Show when=move || !auto_lock_status.get().is_empty()>
                            <span class="text-xs text-text/50">{move || auto_lock_status.get()}</span>
                        </Show>
                    </div>
                </section>

                <div class="h-px bg-input-border/60"></div>

                <section class="space-y-3">
                    <h2 class="text-base font-semibold tracking-tight">"Encryption"</h2>
                    <Show when=move || encryption_status.get().is_some_and(|encryption_status| encryption_status.enabled)>
                        <div class="space-y-3">
                            <p class="text-sm text-text/70">
                                "Encryption is "
                                <span class="font-semibold text-rich-cerulean-500">"enabled"</span>"."
                            </p>
                            <p class="text-sm text-text/55">"Enter your password to disable encryption (history will be decrypted to plaintext)."</p>
                            <div class="flex flex-wrap items-center gap-2">
                                <input
                                    class="field w-56"
                                    type="password"
                                    placeholder="Current password"
                                    prop:value=move || password.get()
                                    on:input=move |ev| password.set(event_target_value(&ev))
                                />
                                <button
                                    class="btn btn-danger border border-red-500/30"
                                    on:click=move |_| {
                                        let pw = password.get();
                                        if pw.is_empty() {
                                            error.set("Enter your password to disable encryption".into());
                                            return;
                                        }
                                        busy.set(true);
                                        spawn_local(async move {
                                            match invoke_result::<(), AppError>("disable_encryption", &serde_json::json!({ "password": pw })).await {
                                                Ok(_) => {
                                                    error.set(String::new());
                                                    password.set(String::new());
                                                    refresh_encryption_status(encryption_status);
                                                    screen.set(Page::Home);
                                                }
                                                Err(e) => error.set(e.to_string()),
                                            }
                                            busy.set(false);
                                        });
                                    }
                                    prop:disabled=move || busy.get()
                                >"Disable encryption"</button>
                            </div>
                        </div>
                    </Show>

                    <Show when=move || encryption_status.get().is_some_and(|encryption_status| !encryption_status.enabled)>
                        <div class="space-y-3">
                            <p class="text-sm text-text/70">
                                "Encryption is "
                                <span class="font-semibold text-text/50">"disabled"</span>"."
                            </p>
                            <p class="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-sm text-amber-600 dark:text-amber-400">
                                "Warning: if you forget your password, your clipboard history cannot be recovered."
                            </p>
                            <div class="flex flex-wrap items-center gap-2">
                                <input
                                    class="field w-56"
                                    type="password"
                                    placeholder="New password"
                                    prop:value=move || password.get()
                                    on:input=move |ev| password.set(event_target_value(&ev))
                                />
                                <input
                                    class="field w-56"
                                    type="password"
                                    placeholder="Confirm password"
                                    prop:value=move || password_confirm.get()
                                    on:input=move |ev| password_confirm.set(event_target_value(&ev))
                                />
                                <button
                                    class="btn btn-primary"
                                    on:click=move |_| {
                                        let pw = password.get();
                                        let pw2 = password_confirm.get();
                                        if pw.is_empty() {
                                            error.set("Password must not be empty".into());
                                            return;
                                        }
                                        if pw != pw2 {
                                            error.set("Passwords do not match".into());
                                            return;
                                        }
                                        busy.set(true);
                                        spawn_local(async move {
                                            match invoke_result::<(), AppError>("setup_encryption", &serde_json::json!({ "password": pw })).await {
                                                Ok(_) => {
                                                    error.set(String::new());
                                                    password.set(String::new());
                                                    password_confirm.set(String::new());
                                                    refresh_encryption_status(encryption_status);
                                                    screen.set(Page::Home);
                                                }
                                                Err(e) => error.set(e.to_string()),
                                            }
                                            busy.set(false);
                                        });
                                    }
                                    prop:disabled=move || busy.get()
                                >"Enable encryption"</button>
                            </div>
                        </div>
                    </Show>

                    <Show when=move || !error.get().is_empty()>
                        <p class="text-sm text-red-500">{move || error.get()}</p>
                    </Show>
                </section>
            </div>
        </div>
    }
}
