use crate::{
    model::{DependencyEntry, DependencyPage},
    tauri_bridge::{load_dependency_page, toggle_profile_ignore},
};
use leptos::prelude::*;
use wasm_bindgen::JsValue;

#[component]
pub(super) fn DependencyPanel(
    page: ReadSignal<Option<DependencyPage>>,
    error: ReadSignal<Option<String>>,
    action_error: ReadSignal<Option<String>>,
    set_page: WriteSignal<Option<DependencyPage>>,
    set_error: WriteSignal<Option<String>>,
    current_page: ReadSignal<Option<DependencyPage>>,
    set_history: WriteSignal<Vec<(String, String)>>,
) -> impl IntoView {
    view! {
        <div class="dependency-panel">
            {move || error.get().map(|error| view! { <div class="dependency-error">{error}</div> })}
            {move || action_error.get().map(|error| view! { <div class="dependency-error">{error}</div> })}
            {move || {
                page.get().map(|page| {
                    let page_id = page.id.clone();
                    let modpacks = page.modpacks.clone();
                    let mods = page.mods.clone();
                    let editable_profile = page.editable_profile;
                    view! {
                        <div class="dependency-content">
                            <DependencySection
                                title="Modpacks"
                                empty="No explicit modpack dependencies."
                                kind="modpack"
                                entries=modpacks
                                editable_profile=false
                                profile_id=page_id.clone()
                                set_page
                                set_error
                                current_page
                                set_history
                            />
                            <DependencySection
                                title="Mods"
                                empty="No explicit mod dependencies."
                                kind="mod"
                                entries=mods
                                editable_profile=editable_profile
                                profile_id=page_id
                                set_page
                                set_error
                                current_page
                                set_history
                            />
                        </div>
                    }
                })
            }}
        </div>
    }
}

#[component]
fn DependencySection(
    title: &'static str,
    empty: &'static str,
    kind: &'static str,
    entries: Vec<DependencyEntry>,
    editable_profile: bool,
    profile_id: String,
    set_page: WriteSignal<Option<DependencyPage>>,
    set_error: WriteSignal<Option<String>>,
    current_page: ReadSignal<Option<DependencyPage>>,
    set_history: WriteSignal<Vec<(String, String)>>,
) -> impl IntoView {
    let entry_count = entries.len();
    let entries_view = if entries.is_empty() {
        view! { <div class="dependency-empty">{empty}</div> }.into_any()
    } else {
        view! {
            <div class="dependency-list">
                {entries
                    .into_iter()
                    .map(|entry| {
                        view! {
                            <DependencyRow
                                entry
                                kind
                                editable_profile
                                profile_id=profile_id.clone()
                                set_page
                                set_error
                                current_page
                                set_history
                            />
                        }
                    })
                    .collect_view()}
            </div>
        }
        .into_any()
    };

    view! {
        <section class="dependency-section">
            <div class="section-heading">
                <h2>{format!("{title} ({entry_count})")}</h2>
            </div>
            {entries_view}
        </section>
    }
}

#[component]
fn DependencyRow(
    entry: DependencyEntry,
    kind: &'static str,
    editable_profile: bool,
    profile_id: String,
    set_page: WriteSignal<Option<DependencyPage>>,
    set_error: WriteSignal<Option<String>>,
    current_page: ReadSignal<Option<DependencyPage>>,
    set_history: WriteSignal<Vec<(String, String)>>,
) -> impl IntoView {
    let id = entry.id.clone();
    let name = entry.name.clone();
    let found = entry.found;
    let ignored = entry.ignored;
    let reason = entry
        .reason
        .clone()
        .unwrap_or_else(|| "Not Found".to_string());
    let row_id = id.clone();
    let missing_reason = if found {
        view! {}.into_any()
    } else {
        view! { <small>{reason}</small> }.into_any()
    };
    let ignore_button = if editable_profile && kind == "mod" {
        let profile_id_for_click = profile_id.clone();
        let mod_id_for_click = id.clone();
        let ignore_class = if ignored {
            "ignore-toggle active"
        } else {
            "ignore-toggle"
        };
        view! {
            <button
                type="button"
                class=ignore_class
                on:click=move |_| {
                    let profile_id = profile_id_for_click.clone();
                    let mod_id = mod_id_for_click.clone();
                    leptos::task::spawn_local(async move {
                        match toggle_profile_ignore(&profile_id, &mod_id).await {
                            Ok(page) => {
                                set_error.set(None);
                                set_page.set(Some(page));
                            }
                            Err(error) => set_error.set(Some(js_error_to_string(error))),
                        }
                    });
                }
            >
                "Ignore"
            </button>
        }
        .into_any()
    } else {
        view! {}.into_any()
    };

    view! {
        <div class=dependency_row_class(found, ignored)>
            <button
                type="button"
                class="dependency-row-main"
                disabled=!found
                on:click=move |_| {
                    if found {
                        navigate_page(
                            kind.to_string(),
                            row_id.clone(),
                            current_page,
                            set_history,
                            set_page,
                            set_error,
                        );
                    }
                }
            >
                <span>
                    <strong>{name}</strong>
                    <em>{id}</em>
                </span>
                {missing_reason}
            </button>
            {ignore_button}
        </div>
    }
}

fn navigate_page(
    kind: String,
    id: String,
    current_page: ReadSignal<Option<DependencyPage>>,
    set_history: WriteSignal<Vec<(String, String)>>,
    set_page: WriteSignal<Option<DependencyPage>>,
    set_error: WriteSignal<Option<String>>,
) {
    if let Some(page) = current_page.get() {
        set_history.update(|history| history.push((page.kind, page.id)));
    }
    load_page(kind, id, set_page, set_error);
}

fn load_page(
    kind: String,
    id: String,
    set_page: WriteSignal<Option<DependencyPage>>,
    set_error: WriteSignal<Option<String>>,
) {
    leptos::task::spawn_local(async move {
        match load_dependency_page(&kind, &id).await {
            Ok(page) => {
                set_error.set(None);
                set_page.set(Some(page));
            }
            Err(error) => set_error.set(Some(js_error_to_string(error))),
        }
    });
}

fn dependency_row_class(found: bool, ignored: bool) -> &'static str {
    match (found, ignored) {
        (false, _) => "dependency-row missing",
        (true, true) => "dependency-row ignored",
        (true, false) => "dependency-row",
    }
}

fn js_error_to_string(error: JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "Unexpected launcher error".to_string())
}
