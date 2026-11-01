use crate::{
    icons::SearchIcon,
    model::{DependencyEntry, DependencyPage, ProfileDependency},
    tauri_bridge::{
        load_dependency_page, load_dependency_page_for_profile, load_profile_dependencies,
        toggle_profile_ignore,
    },
};
use leptos::prelude::*;
use patchwork_registry_types::is_generated_mod_id;
use patchwork_ui::RefreshCwIcon;
use wasm_bindgen::JsValue;

#[derive(Clone, Copy)]
struct Navigation {
    current: ReadSignal<Option<DependencyPage>>,
    set_page: WriteSignal<Option<DependencyPage>>,
    set_error: WriteSignal<Option<String>>,
    set_history: WriteSignal<Vec<(String, String)>>,
    generation: RwSignal<u64>,
}

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
    let detail_kind = RwSignal::new("modpack");
    let navigation = Navigation {
        current: current_page,
        set_page,
        set_error,
        set_history,
        generation: RwSignal::new(0),
    };
    Effect::new(move |_| {
        page.get();
        navigation.generation.update(|value| *value += 1);
    });
    on_cleanup(move || {
        navigation.generation.try_update(|value| *value += 1);
    });
    view! {
        <div class="dependency-panel">
            {move || error.get().map(|error| view! { <div class="dependency-error" role="alert">{error}</div> })}
            {move || action_error.get().map(|error| view! { <div class="dependency-error" role="alert">{error}</div> })}
            <Show when=move || page.get().is_some_and(|page| page.editable_profile) fallback=move || {
                page.get().map(|page| view! {
                    <div class="dependency-content">
                        <div class="profile-dependency-filters" role="group" aria-label="Dependency type">
                            <button type="button" class:active=move || detail_kind.get() == "modpack" on:click=move |_| detail_kind.set("modpack")>"Modpacks"</button>
                            <button type="button" class:active=move || detail_kind.get() == "mod" on:click=move |_| detail_kind.set("mod")>"Mods"</button>
                        </div>
                        {move || if detail_kind.get() == "modpack" {
                            view! { <DependencySection title="Modpacks" empty="No explicit modpack dependencies." kind="modpack" entries=page.modpacks.clone() navigation /> }.into_any()
                        } else {
                            view! { <DependencySection title="Mods" empty="No explicit mod dependencies." kind="mod" entries=page.mods.clone() navigation /> }.into_any()
                        }}
                    </div>
                })
            }>
                <ProfileDependencyManager page navigation />
            </Show>
        </div>
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DependencyFilter {
    All,
    Enabled,
    Disabled,
}

fn matches_dependency(entry: &ProfileDependency, query: &str, filter: DependencyFilter) -> bool {
    let state_matches = match filter {
        DependencyFilter::All => true,
        DependencyFilter::Enabled => !entry.ignored,
        DependencyFilter::Disabled => entry.ignored,
    };
    state_matches
        && (query.is_empty()
            || entry.id.to_lowercase().contains(query)
            || entry.name.to_lowercase().contains(query)
            || entry.kind.to_lowercase().contains(query)
            || entry
                .required_by
                .iter()
                .any(|id| id.to_lowercase().contains(query)))
}

#[component]
fn ProfileDependencyManager(
    page: ReadSignal<Option<DependencyPage>>,
    navigation: Navigation,
) -> impl IntoView {
    let live_sources = expect_context::<crate::app::LiveSources>();
    let entries = RwSignal::new(Vec::<ProfileDependency>::new());
    let query = RwSignal::new(String::new());
    let filter = RwSignal::new(DependencyFilter::All);
    let kind_filter = RwSignal::new("modpack");
    let loading = RwSignal::new(true);
    let pending = RwSignal::new(None::<String>);
    let load_error = RwSignal::new(None::<String>);
    let generation = RwSignal::new(0_u64);
    let reload = RwSignal::new(0_u64);
    let last_profile = RwSignal::new(None::<String>);
    on_cleanup(move || {
        generation.try_update(|value| *value += 1);
    });
    Effect::new(move |_| {
        let current = page.get().filter(|page| page.editable_profile);
        reload.get();
        generation.update(|value| *value += 1);
        let request = generation.get_untracked();
        let Some(current) = current else {
            return;
        };
        if last_profile.get_untracked().as_deref() != Some(&current.id) {
            query.set(String::new());
            filter.set(DependencyFilter::All);
            kind_filter.set("modpack");
            last_profile.set(Some(current.id.clone()));
            entries.set(Vec::new());
        }
        loading.set(true);
        pending.set(None);
        load_error.set(None);
        leptos::task::spawn_local(async move {
            let result = load_profile_dependencies(&current.id).await;
            if !request_current(generation, request, page, &current.id) {
                return;
            }
            match result {
                Ok(mut dependencies) => {
                    dependencies.sort_by_cached_key(|entry| {
                        (
                            entry.name.to_lowercase(),
                            entry.kind.clone(),
                            entry.id.clone(),
                        )
                    });
                    entries.set(dependencies);
                }
                Err(error) => load_error.set(Some(js_error_to_string(error))),
            }
            loading.set(false);
        });
    });
    let filtered = Memo::new(move |_| {
        let query = query.get().trim().to_lowercase();
        let mut result = entries
            .get()
            .into_iter()
            .filter(|entry| {
                entry.kind == kind_filter.get() && matches_dependency(entry, &query, filter.get())
            })
            .collect::<Vec<_>>();
        if !query.is_empty() {
            result.reverse();
        }
        result
    });

    view! {
        <section class="profile-dependency-manager" aria-label="Profile dependencies" aria-busy=move || loading.get() || pending.get().is_some()>
            <div class="profile-dependency-toolbar">
                <div class="profile-dependency-filters" role="group" aria-label="Dependency type">
                    {[("modpack", "Modpacks"), ("mod", "Mods")].into_iter().map(|(kind, label)| view! {
                        <button type="button" class:active=move || kind_filter.get() == kind aria-pressed=move || kind_filter.get() == kind on:click=move |_| kind_filter.set(kind)>{label}</button>
                    }).collect_view()}
                </div>
                <label class="profile-dependency-search">
                    <SearchIcon />
                    <input type="search" aria-label="Search dependencies" placeholder="Search dependencies" prop:value=move || query.get()
                        on:input=move |event| query.set(event_target_value(&event)) />
                </label>
                <div class="profile-dependency-filters" role="group" aria-label="Dependency status">
                    {[(DependencyFilter::All, "All"), (DependencyFilter::Enabled, "Enabled"), (DependencyFilter::Disabled, "Disabled")].into_iter().map(|(value, label)| view! {
                        <button type="button" class:active=move || filter.get() == value aria-pressed=move || filter.get() == value on:click=move |_| filter.set(value)>
                            {label}<span>{move || entries.get().iter().filter(|entry| entry.kind == kind_filter.get() && matches_dependency(entry, "", value)).count()}</span>
                        </button>
                    }).collect_view()}
                </div>
                <button type="button" class="icon-button" title="Reload dependencies" aria-label="Reload dependencies"
                    disabled=move || loading.get() || pending.get().is_some() on:click=move |_| reload.update(|value| *value += 1)><RefreshCwIcon /></button>
            </div>
            {move || load_error.get().map(|message| view! { <div class="dependency-error" role="alert">{message}</div> })}
            <div class="profile-dependency-summary" role="status">
                {move || if loading.get() { "Loading dependencies...".into() } else {
                    let all = entries.get();
                    let missing = all.iter().filter(|entry| !entry.found && !(entry.kind == "mod" && is_generated_mod_id(&entry.id))).count();
                    format!("{} of {} dependencies / {} local / {} missing", filtered.get().len(), all.len(), all.iter().filter(|entry| entry.local).count(), missing)
                }}
            </div>
            <div class="profile-dependency-scroll">
                <Show when=move || loading.get()><div class="options-loading"><span class="button-spinner"></span>"Loading dependencies..."</div></Show>
                <Show when=move || !loading.get() && load_error.get().is_none() && filtered.get().is_empty()>
                    <div class="dependency-empty">{move || if entries.get().is_empty() { "No dependencies in this profile." } else { "No matching dependencies." }}</div>
                </Show>
                <For each=move || filtered.get() key=|entry| (entry.kind.clone(), entry.id.clone()) children=move |initial| {
                    let row_id = initial.id.clone();
                    let row_kind = initial.kind.clone();
                    let entry = Memo::new(move |_| entries.get().into_iter().find(|e| e.id == row_id && e.kind == row_kind).unwrap_or_else(|| initial.clone()));
                    let expanded = RwSignal::new(false);
                    let generated = entry.get_untracked().kind == "mod" && is_generated_mod_id(&entry.get_untracked().id);
                    let toggle_id = if entry.get_untracked().kind == "modpack" { format!("modpack/{}", entry.get_untracked().id) } else { entry.get_untracked().id };
                    let pending_id = toggle_id.clone();
                    view! {
                        <div class="profile-dependency-item" class:is-ignored=move || entry.get().ignored>
                            <div class="profile-dependency-row" class:is-missing=move || !entry.get().found>
                                <button type="button" class="profile-dependency-link" disabled=move || !entry.get().found || generated
                                    on:click=move |_| { let e = entry.get_untracked(); navigate_page(e.kind, e.id, navigation); }>
                                    <img class="dependency-favicon" src=move || entry.get().icon_data_url.unwrap_or_else(|| "/logo.png".into()) alt="" on:error=move |event| {
                                        use wasm_bindgen::JsCast;
                                        if let Some(image) = event.target().and_then(|t| t.dyn_into::<web_sys::HtmlImageElement>().ok()) {
                                            if image.get_attribute("src").as_deref() != Some("/logo.png") { image.set_src("/logo.png"); }
                                        }
                                    } />
                                    <span class="dependency-inline-name"><strong>{move || entry.get().name}</strong>
                                        <code>{move || format!("({})", entry.get().id)}</code>
                                        <span class="dependency-version">{move || { let v = entry.get().version; if v.is_empty() { String::new() } else { format!("v{v}") } }}</span>
                                    </span>
                                </button>
                                <div class="profile-dependency-badges">
                                    <button type="button" class="dependency-expand icon-button" class:expanded=move || expanded.get() aria-expanded=move || expanded.get()
                                        title="Show dependency information" aria-label="Show dependency information" on:click=move |_| expanded.update(|v| *v = !*v)>
                                        <span class="disclosure-triangle" aria-hidden="true"></span>
                                    </button>
                                    <span>{move || if entry.get().kind == "modpack" { "Modpack" } else { "Mod" }}</span>
                                    <Show when=move || entry.get().local><span class="local">"Local"</span></Show>
                                    {generated.then(|| view! { <span>"Generated"</span> })}
                                    <Show when=move || !entry.get().found && !generated><span class="missing">"Missing"</span></Show>
                                </div>
                                <label class="profile-dependency-state" title=move || entry.get().toggle_blocked_reason.unwrap_or_else(|| "Enable or exclude in this profile".into())>
                                    <input type="checkbox" class="profile-switch" role="switch" aria-label="Enabled in profile"
                                        prop:checked=move || !entry.get().explicitly_ignored
                                        disabled=move || generated || loading.get() || pending.get().is_some()
                                        on:change=move |event| {
                                            use wasm_bindgen::JsCast;
                                            if let Some(input) = event.target().and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok()) {
                                                input.set_checked(!entry.get_untracked().explicitly_ignored);
                                            }
                                            if pending.get_untracked().is_some() { return; }
                                            let Some(current) = page.get_untracked().filter(|p| p.editable_profile) else { return; };
                                            let id = toggle_id.clone();
                                            pending.set(Some(id.clone()));
                                            generation.update(|v| *v += 1);
                                            let request = generation.get_untracked();
                                            load_error.set(None);
                                            leptos::task::spawn_local(async move {
                                                let result = match toggle_profile_ignore(&current.id, &id).await {
                                                    Ok(_) => load_profile_dependencies(&current.id).await,
                                                    Err(error) => Err(error),
                                                };
                                                if !request_current(generation, request, page, &current.id) { return; }
                                                match result {
                                                    Ok(mut updated) => {
                                                        updated.sort_by_cached_key(|e| (e.name.to_lowercase(), e.kind.clone(), e.id.clone()));
                                                        entries.set(updated);
                                                        live_sources.profile_updates_revision.update(|v| *v += 1);
                                                    }
                                                    Err(error) => {
                                                        load_error.set(Some(js_error_to_string(error)));
                                                        // Notify bindings to restore the persisted checkbox value.
                                                        entries.update(|_| {});
                                                    }
                                                }
                                                pending.set(None);
                                            });
                                        } />
                                    <span>{move || if pending.get().as_deref() == Some(&pending_id) { "Saving..." } else if entry.get().explicitly_ignored { "Disabled" } else { "Enabled" }}</span>
                                </label>
                            </div>
                            <div class="dependency-disclosure" class:expanded=move || expanded.get() inert=move || !expanded.get()>
                                <div><div class="dependency-disclosure-content">
                                    <dl>
                                        <dt>"Dependencies"</dt><dd>{move || { let e = entry.get(); if e.dependencies.is_empty() { "None".into() } else { e.dependencies.join(", ") } }}</dd>
                                        <dt>"Required by"</dt><dd>{move || entry.get().required_by.join(", ")}</dd>
                                        <dt>"Source"</dt><dd>{move || if entry.get().local { "Local override" } else { "Installed sources" }}</dd>
                                        <dt>"Status"</dt><dd>{move || entry.get().toggle_blocked_reason.unwrap_or_else(|| if entry.get().ignored { "Excluded by profile".into() } else { "Active".into() })}</dd>
                                    </dl>
                                </div></div>
                            </div>
                        </div>
                    }
                } />
            </div>
        </section>
    }
}

fn request_current(
    generation: RwSignal<u64>,
    request: u64,
    page: ReadSignal<Option<DependencyPage>>,
    id: &str,
) -> bool {
    generation.try_get_untracked() == Some(request)
        && page
            .try_get_untracked()
            .flatten()
            .is_some_and(|page| page.editable_profile && page.id == id)
}

#[component]
fn DependencySection(
    title: &'static str,
    empty: &'static str,
    kind: &'static str,
    entries: Vec<DependencyEntry>,
    navigation: Navigation,
) -> impl IntoView {
    let count = entries.len();
    view! {
        <section class="dependency-section">
            <div class="section-heading"><h2>{format!("{title} ({count})")}</h2></div>
            {if entries.is_empty() {
                view! { <div class="dependency-empty">{empty}</div> }.into_any()
            } else {
                view! { <div class="dependency-list">{entries.into_iter().map(|entry| view! { <DependencyRow entry kind navigation /> }).collect_view()}</div> }.into_any()
            }}
        </section>
    }
}

#[component]
fn DependencyRow(
    entry: DependencyEntry,
    kind: &'static str,
    navigation: Navigation,
) -> impl IntoView {
    let generated = kind == "mod" && is_generated_mod_id(&entry.id);
    let clickable = entry.found && !generated;
    let note = if generated {
        Some("Generated during compose".into())
    } else if !entry.found {
        Some(entry.reason.unwrap_or_else(|| "Not Found".into()))
    } else {
        None
    };
    let id = entry.id.clone();
    view! {
        <div class=dependency_row_class(entry.found, entry.ignored, generated)>
            <button type="button" class="dependency-row-main" disabled=!clickable on:click=move |_| navigate_page(kind.into(), id.clone(), navigation)>
                <span><strong>{entry.name}</strong><em>{entry.id}</em></span>
                {note.map(|note| view! { <small class:generated-note=generated>{note}</small> })}
            </button>
        </div>
    }
}

fn navigate_page(kind: String, id: String, navigation: Navigation) {
    let previous = navigation.current.get_untracked();
    let root = previous.as_ref().and_then(|page| {
        page.profile_id
            .clone()
            .or_else(|| page.editable_profile.then(|| page.id.clone()))
    });
    navigation.generation.update(|value| *value += 1);
    let request = navigation.generation.get_untracked();
    leptos::task::spawn_local(async move {
        let result = match root {
            Some(root) => load_dependency_page_for_profile(&kind, &id, &root).await,
            None => load_dependency_page(&kind, &id).await,
        };
        if navigation.generation.try_get_untracked() != Some(request)
            || navigation.current.try_get_untracked() != Some(previous.clone())
        {
            return;
        }
        match result {
            Ok(page) => {
                if let Some(previous) = previous {
                    navigation
                        .set_history
                        .update(|history| history.push((previous.kind, previous.id)));
                }
                navigation.set_error.set(None);
                navigation.set_page.set(Some(page));
            }
            Err(error) => navigation.set_error.set(Some(js_error_to_string(error))),
        }
    });
}

fn dependency_row_class(found: bool, ignored: bool, generated: bool) -> &'static str {
    match (found, ignored, generated) {
        (_, _, true) => "dependency-row generated",
        (false, _, false) => "dependency-row missing",
        (true, true, false) => "dependency-row ignored",
        (true, false, false) => "dependency-row",
    }
}

fn js_error_to_string(error: JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "Unexpected launcher error".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_includes_nested_parents_and_status() {
        let entry = ProfileDependency {
            id: "renderer".into(),
            name: "World Renderer".into(),
            kind: "mod".into(),
            found: false,
            ignored: true,
            local: true,
            required_by: vec!["Base-Pack".into()],
            toggle_blocked_reason: None,
            version: "1.0.0".into(),
            dependencies: Vec::new(),
            explicitly_ignored: true,
            icon_data_url: None,
        };
        assert!(matches_dependency(
            &entry,
            "base-pack",
            DependencyFilter::Disabled
        ));
        assert!(matches_dependency(
            &entry,
            "renderer",
            DependencyFilter::All
        ));
        assert!(!matches_dependency(&entry, "", DependencyFilter::Enabled));
        assert!(!matches_dependency(
            &entry,
            "unrelated",
            DependencyFilter::All
        ));
    }
}
