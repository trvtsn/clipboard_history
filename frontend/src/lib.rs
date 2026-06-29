pub mod components;
pub mod pages;

use clipboard_history::{AppError, CopiedObject, CopiedObjectPreview, EncryptionStatus, ObjectContent, ObjectFormat, PreviewContent};
use base64::{Engine, engine::general_purpose::STANDARD};
use leptos::{prelude::*, task::spawn_local};
use tauri_sys::core::invoke_result;
use wasm_bindgen::prelude::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Page {
    Home,
    Settings,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SortBy {
    Date,
    Type,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SortOrder {
    Ascending,
    Descending,
}

pub fn format_kind(item: &CopiedObjectPreview) -> &'static str {
    match item.content_format {
        ObjectFormat::Text => "text",
        ObjectFormat::Rtf => "rtf",
        ObjectFormat::Html => "html",
        ObjectFormat::Image => "image",
        ObjectFormat::Files => "files",
        ObjectFormat::Other(_) => "other",
    }
}

pub fn format_preview(item: &CopiedObjectPreview) -> AnyView {
    match &item.preview {
        PreviewContent::Text(s) | PreviewContent::Rtf(s) | PreviewContent::Html(s) => {
            s.clone().into_any()
        }
        PreviewContent::Image(bytes) => {
            if bytes.is_empty() {
                "[image]".into_any()
            } else {
                let src = format!("data:image/png;base64,{}", STANDARD.encode(bytes));
                view! { <img src=src alt="" style="max-width:120px;max-height:80px;"/> }.into_any()
            }
        }
        PreviewContent::Files(fs) => {
            {fs.into_iter()
            .map(|file| {
                let file = file.clone();
                let display = file.clone();
                display
            })
            .collect_view().into_any()}
        },
        PreviewContent::Other(name, n) => format!("[{name} ({n} bytes)]").into_any(),
    }
}

pub fn format_full(obj: &CopiedObject) -> AnyView {
    match &obj.content {
        ObjectContent::Text(s) | ObjectContent::Rtf(s) | ObjectContent::Html(s) => {
            view! {
                <pre class="max-h-80 overflow-auto whitespace-pre-wrap break-words font-sans text-sm text-text/85">{s.clone()}</pre>
            }.into_any()
        }
        ObjectContent::Image(bytes) => {
            if bytes.is_empty() {
                "[image]".into_any()
            } else {
                let src = format!("data:image/png;base64,{}", STANDARD.encode(bytes));
                view! { <img src=src alt="" class="max-h-80 max-w-full rounded-md border border-input-border"/> }.into_any()
            }
        }
        ObjectContent::Files(fs) => {
            view! {
                <ul class="space-y-1 text-sm">
                    {fs.iter()
                        .map(|file| {
                            let file = file.clone();
                            let display = file.clone();
                            view! {
                                <li>
                                    <a
                                        class="text-cobalt-blue-500 underline-offset-4 hover:underline"
                                        href="#"
                                        on:click=move |ev| {
                                            ev.prevent_default();
                                            let path = file.clone();
                                            spawn_local(async move {
                                                let _ = invoke_result::<(), AppError>(
                                                    "reveal_in_directory",
                                                    &serde_json::json!({ "path": path }),
                                                ).await;
                                            });
                                        }
                                    >{display}</a>
                                </li>
                            }
                        })
                        .collect_view()}
                </ul>
            }.into_any()
        }
        ObjectContent::Other(name, bytes) => format!("[{name} ({} bytes)]", bytes.len()).into_any(),
    }
}

pub fn format_date(millis: u64) -> String {
    let date = js_sys::Date::new(&JsValue::from_f64(millis as f64));
    String::from(date.to_locale_string("default", &JsValue::UNDEFINED))
}

pub fn refresh_encryption_status(status: RwSignal<Option<EncryptionStatus>>) {
    spawn_local(async move {
        if let Ok(s) = invoke_result::<EncryptionStatus, String>("encryption_status", &()).await {
            status.set(Some(s));
        }
    });
}
