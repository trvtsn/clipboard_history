use clipboard_history::{AppError, AppSettings, EncryptionStatus};
use clipboard_history_frontend::{Page, pages::{home::Home, login::Login, settings::Settings}, refresh_encryption_status};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_use::{UseColorModeOptions, UseColorModeReturn, UseIdleReturn, use_active_element, use_color_mode_with_options, use_event_listener, use_idle, use_interval_fn};
use tauri_sys::core::invoke_result;

#[component]
pub fn App() -> impl IntoView {
    let UseColorModeReturn { mode, set_mode, .. } = use_color_mode_with_options(
        UseColorModeOptions::default().cookie_enabled(true)
    );
    provide_context(mode);
    provide_context(set_mode);
    
    let encryption_status = RwSignal::new(Option::<EncryptionStatus>::None);
    let page = RwSignal::new(Page::Home);
    let auto_lock_minutes = RwSignal::new(5u64);
    provide_context(encryption_status);
    provide_context(page);
    provide_context(auto_lock_minutes);

    refresh_encryption_status(encryption_status);

    spawn_local(async move {
        if let Ok(settings) = invoke_result::<AppSettings, AppError>("load_settings", &()).await {
            auto_lock_minutes.set(settings.auto_lock_minutes);
        }
    });

    // Lock on "L" keypress
    let _ = use_event_listener(window(), leptos::ev::keydown, move |ev| {
        if ev.ctrl_key() || ev.meta_key() || ev.alt_key() || !ev.key().eq_ignore_ascii_case("l") {
            return;
        }

        let active = use_active_element();
        if let Some(el) = active.get_untracked() && matches!(
            el.tag_name().as_str(), "INPUT" | "TEXTAREA" | "SELECT"
        ) { return; }

        let Some(s) = encryption_status.get_untracked() else { return; };
        if !(s.enabled && s.unlocked) {
            return;
        }
        spawn_local(async move {
            if invoke_result::<(), AppError>("lock", &()).await.is_ok() {
                refresh_encryption_status(encryption_status);
            }
        });
    });

    // To-Do: Replace with backend idle checker as this approach can be unreliable.
    let UseIdleReturn { last_active, .. } = use_idle(1_000);
    use_interval_fn(
        move || {
            let minutes = auto_lock_minutes.get_untracked();
            if minutes == 0 { return; }
            let Some(s) = encryption_status.get_untracked() else { return; };
            if !(s.enabled && s.unlocked) { return; }

            let elapsed = js_sys::Date::now() - last_active.get_untracked();
            if elapsed >= (minutes as f64) * 60_000.0 {
                spawn_local(async move {
                    if invoke_result::<(), AppError>("lock", &()).await.is_ok() {
                        refresh_encryption_status(encryption_status);
                    }
                });
            }
        },
        30_000,
    );

    let is_encrypted_and_locked = Memo::new(move |_| {
        encryption_status.get().is_some_and(|encryption_status| {
            encryption_status.enabled && !encryption_status.unlocked
        })
    });

    view! {
        <leptos_meta::Html attr:data-theme=move || mode.get().to_string() {..} class="h-full"/>

        <main class="min-h-screen bg-background text-text">
            <Show when=move || encryption_status.get().is_none()>
                <div class="flex min-h-screen items-center justify-center">
                    <p class="text-sm text-text/50 animate-pulse">"Loading..."</p>
                </div>
            </Show>

            <Show when=move || is_encrypted_and_locked.get()>
                <Login />
            </Show>

            <Show when=move || !is_encrypted_and_locked.get() && page.get() == Page::Home>
                <Home />
            </Show>

            <Show when=move || !is_encrypted_and_locked.get() && page.get() == Page::Settings>
                <Settings />
            </Show>
        </main>
    }
}
