use leptos::prelude::*;

pub enum ComponentSize {
    Small,
    Medium,
    Big,
}

pub const FORMAT_CHIPS: [(&str, &str); 6] = [
    ("text", "Text"),
    ("rtf", "RTF"),
    ("html", "HTML"),
    ("image", "Image"),
    ("files", "Files"),
    ("other", "Other"),
];

// Thanks to devAaus (https://github.com/devAaus)
// https://uiverse.io/devAaus/funny-catfish-94
#[component]
pub fn Spinner(component_size: ComponentSize) -> impl IntoView {
    let blue_indicator_classes_base = "border-4 border-transparent text-blue-400 text-4xl animate-spin flex items-center justify-center border-t-blue-400 rounded-full".to_string();
    let blue_indicator_classes = match component_size {
        ComponentSize::Small => Memo::new(move |_| format!("w-5 h-5 {blue_indicator_classes_base}")),
        ComponentSize::Medium => Memo::new(move |_| format!("w-10 h-10 {blue_indicator_classes_base}")),
        ComponentSize::Big => Memo::new(move |_| format!("w-20 h-20 {blue_indicator_classes_base}")),
    };
    let red_indicator_classes_base = "border-4 border-transparent text-red-400 text-2xl animate-spin flex items-center justify-center border-t-red-400 rounded-full".to_string();
    let red_indicator_classes = match component_size {
        ComponentSize::Small => Memo::new(move |_| format!("w-4 h-4 {red_indicator_classes_base}")),
        ComponentSize::Medium => Memo::new(move |_| format!("w-8 h-8 {red_indicator_classes_base}")),
        ComponentSize::Big => Memo::new(move |_| format!("w-16 h-16 {red_indicator_classes_base}")),
    };
    view! {
        <div class="flex-col gap-4 w-full flex items-center justify-center">
            <div 
                class=move || blue_indicator_classes.get()
            >
                <div
                    class=move || red_indicator_classes.get()
                ></div>
            </div>
        </div>
    }
}

#[component]
pub fn DimmingOverlay(overlay_triggered: RwSignal<bool>) -> impl IntoView {
    view! {
        <Show when=move || overlay_triggered.get()>
            <div
                class="fixed inset-0 bg-black/45 backdrop-blur-[1px] z-10"
                on:click=move |_| overlay_triggered.set(false)
            ></div>
        </Show>
    }
}
