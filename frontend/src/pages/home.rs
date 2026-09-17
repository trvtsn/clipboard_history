use crate::{components::utils::{DimmingOverlay, FORMAT_CHIPS}, Page, SortBy, SortOrder, format_date, format_full, format_kind, format_preview, refresh_encryption_status};
use clipboard_history::{AppError, CopiedObject, CopiedObjectPreview, EncryptionStatus};
use futures::StreamExt;
use icondata as i;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use leptos_use::signal_debounced;
use std::collections::HashSet;
use tauri_sys::core::invoke_result;

#[component]
pub fn Home() -> impl IntoView {
    let history = RwSignal::new(Vec::<CopiedObjectPreview>::new());
    let sort_by = RwSignal::new(SortBy::Date);
    let sort_order = RwSignal::new(SortOrder::Descending);
    let search_query = RwSignal::new(String::new());
    let search_results = RwSignal::new(Option::<HashSet<u32>>::None);
    let format_filter = RwSignal::new(HashSet::<&'static str>::new());
    let page = expect_context::<RwSignal<Page>>();
    let status = expect_context::<RwSignal<Option<EncryptionStatus>>>();
    let opened_object = RwSignal::new(Option::<u32>::None);
    let full_content = RwSignal::new(Option::<(u32, CopiedObject)>::None);

    let overlay_triggered = RwSignal::new(false);
    let clearing = RwSignal::new(false);

    spawn_local(async move {
        if let Ok(items) = invoke_result::<Vec<CopiedObjectPreview>, AppError>("load_history", &()).await {
            history.set(items);
        }
    });

    spawn_local(async move {
        if let Ok(mut stream) = tauri_sys::event::listen::<CopiedObjectPreview>("new_copied_object").await {
            while let Some(ev) = stream.next().await {
                history.update(|history| history.push(ev.payload));
            }
        }
    });

    let history_sorted = Memo::new(move |_| {
        let sort_order = sort_order.get();
        let mut history_sorted = match sort_by.get() {
            SortBy::Date => {
                let mut history = history.get();
                history.sort_by_key(|entry| entry.date);
                history
            },
            SortBy::Type => {
                let mut history = history.get();
                history.sort_by_key(|entry| entry.content_format.clone());
                history
            }
        };
        if sort_order == SortOrder::Ascending { history_sorted } else {
            history_sorted.reverse();
            history_sorted
        }
    });

    let search_query_debounced: Signal<String> = signal_debounced(search_query, 250.0);
    Effect::new(move |_| {
        let debounced = search_query_debounced.get();
        let _ = history.get(); // tracking history as well so that we update on newly copied/deleted objects

        let query = debounced.trim().to_string();
        if query.is_empty() {
            search_results.set(None);
            return;
        }
        
        spawn_local(async move {
            if let Ok(ids) = invoke_result::<Vec<u32>, AppError>("search_history", &serde_json::json!({ "query": query })).await
                && search_query_debounced.get_untracked().trim() == query
            {
                search_results.set(Some(ids.into_iter().collect()));
            }
        });
    });

    let history_filtered = Memo::new(move |_| {
        let filters = format_filter.get();
        let results = search_results.get();
        history_sorted
            .get()
            .into_iter()
            .filter(|item| {
                (filters.is_empty() || filters.contains(format_kind(item)))
                    && results.as_ref().is_none_or(|ids| ids.contains(&item.id))
            })
            .collect::<Vec<_>>()
    });

    view! {
        <DimmingOverlay overlay_triggered />
        <div class="mx-auto px-6 py-8">
            <Show when=move || clearing.get()>
                <div class="absolute inset-0 z-20 flex content-center items-center justify-center rounded-lg p-4">
                    <div class="card p-4 gap-4 flex flex-col items-center justify-center text-center">
                        <div class="text-center">
                            <p>"This will clear all of your history and cannot be undone."</p>
                            <p>"Confirm?"</p>
                        </div>
                        <div class="text-center">
                            <button 
                                class="btn btn-danger btn-sm border border-red-500/30"
                                on:click=move |_| {
                                    spawn_local(async move {
                                        if invoke_result::<(), AppError>("clear_history", &()).await.is_ok() {
                                            history.set(Vec::new());
                                        }
                                    });
                                    clearing.set(false); 
                                    overlay_triggered.set(false);

                                }
                            >"Yes"</button>
                            <button 
                                class="btn btn-ghost btn-sm"
                                on:click=move |_| {
                                    clearing.set(false); 
                                    overlay_triggered.set(false);
                                }
                            >"Nevermind"</button>
                        </div>
                    </div>
                </div>
            </Show>
            
            <header class="mb-6 flex items-center justify-between gap-4">
                <div class="flex items-center gap-3">
                    <img src="/public/clipboard_history_logo_icon.svg" class="size-8" />
                    <h1 class="text-xl font-semibold tracking-tight">"Clipboard History"</h1>
                </div>
                <div class="flex items-center gap-1.5">
                    <button class="btn btn-ghost" on:click=move |_| page.set(Page::Settings)>
                        <Icon icon=i::LuSettings />
                        "Settings"
                    </button>
                    <button class="btn btn-danger" on:click=move |_| {
                        clearing.set(true);
                        overlay_triggered.set(true);
                    }>
                        <Icon icon=i::LuTrash2 />
                        "Clear"
                    </button>
                    <Show when=move || status.get().unwrap_or_default().enabled>
                        <button class="btn btn-secondary" on:click=move |_| {
                            spawn_local(async move {
                                if invoke_result::<(), AppError>("lock", &()).await.is_ok() {
                                    refresh_encryption_status(status);
                                }
                            });
                        }>
                            <Icon icon=i::LuLock />
                            "Lock"
                        </button>
                    </Show>
                </div>
            </header>

            <div class="mb-3 flex flex-wrap items-center gap-2">
                <div class="relative">
                    <span class="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-text/40">
                        <Icon icon=i::LuSearch />
                    </span>
                    <input
                        class="field w-56 pl-8"
                        type="search"
                        placeholder="Search..."
                        prop:value=move || search_query.get()
                        on:input=move |ev| search_query.set(event_target_value(&ev))
                    />
                </div>

                <div class="flex items-center gap-1">
                    <button
                        class="btn btn-sm"
                        class:btn-secondary=move || format_filter.get().is_empty()
                        class:btn-ghost=move || !format_filter.get().is_empty()
                        on:click=move |_| format_filter.update(HashSet::clear)
                    >"All"</button>
                    {FORMAT_CHIPS
                        .into_iter()
                        .map(|(key, label)| {
                            let active = move || format_filter.get().contains(key);
                            view! {
                                <button
                                    class="btn btn-sm"
                                    class:btn-secondary=active
                                    class:btn-ghost=move || !active()
                                    on:click=move |_| {
                                        format_filter.update(|filters| {
                                            if !filters.remove(key) {
                                                filters.insert(key);
                                            }
                                        });
                                    }
                                >{label}</button>
                            }
                        })
                        .collect_view()}
                </div>

                <span class="label ml-auto mr-1">"Sort"</span>
                <select
                    class="field w-auto"
                    prop:value=move || match sort_by.get() {
                        SortBy::Date => "Date",
                        SortBy::Type => "Type",
                    }
                    on:change=move |ev| {
                        let criteria = match event_target_value(&ev).as_str() {
                            "Date" => SortBy::Date,
                            "Type" => SortBy::Type,
                            _ => SortBy::Date,
                        };
                        sort_by.set(criteria);
                    }
                >
                    <option value="Date">"Date"</option>
                    <option value="Type">"Type"</option>
                </select>

                <select
                    class="field w-auto"
                    prop:value=move || match sort_order.get() {
                        SortOrder::Ascending => "Ascending",
                        SortOrder::Descending => "Descending",
                    }
                    on:change=move |ev| {
                        let order = match event_target_value(&ev).as_str() {
                            "Ascending" => SortOrder::Ascending,
                            "Descending" => SortOrder::Descending,
                            _ => SortOrder::Descending,
                        };
                        sort_order.set(order);
                    }
                >
                    <option value="Ascending">"Ascending"</option>
                    <option value="Descending">"Descending"</option>
                </select>
            </div>

            <Show when=move || history.get().is_empty()>
                <div class="card flex flex-col items-center justify-center gap-1 px-6 py-16 text-center">
                    <p class="text-sm font-medium text-text/70">"Nothing copied yet"</p>
                    <p class="text-xs text-text/40">"Items you copy will show up here."</p>
                </div>
            </Show>

            <Show when=move || !history.get().is_empty() && history_filtered.get().is_empty()>
                <div class="card flex flex-col items-center justify-center gap-1 px-6 py-16 text-center">
                    <p class="text-sm font-medium text-text/70">"No matches"</p>
                    <p class="text-xs text-text/40">"Try a different search or filter."</p>
                </div>
            </Show>

            <Show when=move || !history_filtered.get().is_empty()>
                <div class="card overflow-hidden">
                    <table class="w-full table-fixed border-collapse text-sm">
                        <thead>
                            <tr class="border-b border-input-border/70 text-left">
                                <th class="label w-44 px-4 py-2.5">"Date"</th>
                                <th class="label w-20 px-4 py-2.5">"Type"</th>
                                <th class="label px-4 py-2.5">"Preview"</th>
                                <th class="label w-32 px-4 py-2.5 text-right">"Actions"</th>
                            </tr>
                        </thead>
                        <tbody>
                            <For
                                each=move || history_filtered.get()
                                key=|item: &CopiedObjectPreview| item.id
                                children={move |item| {
                                    let id = item.id;
                                    let kind = format_kind(&item);
                                    let preview = format_preview(&item);
                                    let date = format_date(item.date);
                                    let has_formatting = item.has_formatting;
                                    let is_open = move || opened_object.get() == Some(id);

                                    view! {
                                        <tr
                                            class="group border-b border-input-border/40 transition-colors hover:bg-background-hover"
                                            class:border-0=is_open
                                            class:bg-background-hover=is_open
                                            data-id=id
                                        >
                                            <td class="px-4 py-2.5 font-mono text-xs tabular-nums text-text/55">{date}</td>
                                            <td class="px-4 py-2.5"><span class="badge">{kind}</span></td>
                                            <td
                                                class="cursor-pointer px-4 py-2.5 text-text/85"
                                                title="Click to view full content"
                                                on:click=move |_| {
                                                    if opened_object.get_untracked() == Some(id) {
                                                        opened_object.set(None);
                                                        return;
                                                    }
                                                    opened_object.set(Some(id));
                                                    full_content.set(None);
                                                    spawn_local(async move {
                                                        if let Ok(obj) = invoke_result::<CopiedObject, AppError>("get_full_content", &serde_json::json!({ "id": id })).await {
                                                            if opened_object.get_untracked() == Some(id) {
                                                                full_content.set(Some((id, obj)));
                                                            }
                                                        }
                                                    });
                                                }
                                            >
                                                <div class="truncate [&_img]:rounded-md [&_img]:border [&_img]:border-input-border">{preview}</div>
                                            </td>
                                            <td class="px-4 py-2.5">
                                                <div class="flex items-center justify-end gap-0.5 opacity-0 transition-opacity duration-150 focus-within:opacity-100 group-hover:opacity-100">
                                                    <button class="btn btn-ghost btn-sm" title="Copy" on:click=move |_| {
                                                        spawn_local(async move {
                                                            let _ = invoke_result::<(), AppError>("copy_to_clipboard", &serde_json::json!({ "id": id, "formatted": false })).await;
                                                        });
                                                    }>
                                                        <Icon icon=i::LuCopy />
                                                    </button>
                                                    <Show when=move || has_formatting>
                                                        <button class="btn btn-ghost btn-sm" title="Copy with formatting" on:click=move |_| {
                                                            spawn_local(async move {
                                                                let _ = invoke_result::<(), AppError>("copy_to_clipboard", &serde_json::json!({ "id": id, "formatted": true })).await;
                                                            });
                                                        }>
                                                            <Icon icon=i::LuType />
                                                        </button>
                                                    </Show>
                                                    <button class="btn btn-danger btn-sm" title="Delete" on:click=move |_| {
                                                        spawn_local(async move {
                                                            if invoke_result::<(), AppError>("delete_from_history", &serde_json::json!({ "id": id })).await.is_ok() {
                                                                history.update(|history| history.retain(|entry| entry.id != id));
                                                            }
                                                        });
                                                    }>
                                                        <Icon icon=i::LuTrash2 />
                                                    </button>
                                                </div>
                                            </td>
                                        </tr>
                                        <Show when=is_open>
                                            <tr class="bg-background-hover/50">
                                                <td colspan="4" class="px-4 pb-4 pt-0">
                                                    <div class="rounded-lg border border-input-border/70 bg-background p-3">
                                                        {move || match full_content.get() {
                                                            Some((fid, obj)) if fid == id => format_full(&obj),
                                                            _ => view! { <p class="text-xs text-text/40 animate-pulse">"Loading..."</p> }.into_any(),
                                                        }}
                                                    </div>
                                                </td>
                                            </tr>
                                        </Show>
                                    }
                                        .into_any()
                                }}
                            />
                        </tbody>
                    </table>
                </div>
            </Show>
        </div>
    }
}
