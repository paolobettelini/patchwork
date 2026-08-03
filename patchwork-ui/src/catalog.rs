use std::collections::HashSet;

use leptos::prelude::*;
use patchwork_registry_types::{
    RegistryAddToProfileRequest, RegistryBrowseProject, RegistryBrowseRequest,
    RegistryBrowseSource, RegistryDependencyKind, RegistryProfileOption, RegistryProjectKind,
    RegistryProjectRef, RegistryPublishRequest, RegistryScan, RegistryScanEntry, RegistryScanPhase,
    RegistryScanProgress, RegistryScanRequest, RegistryScanStatus, is_generated_mod_id,
};

use crate::icons::{SearchIcon, UploadIcon};

#[component]
pub fn BrowsePage(
    results: Signal<Vec<RegistryBrowseProject>>,
    profiles: Signal<Vec<RegistryProfileOption>>,
    pending: Signal<bool>,
    action_pending: Signal<Option<String>>,
    error: Signal<Option<String>>,
    warnings: Signal<Vec<String>>,
    notice: Signal<Option<String>>,
    #[prop(default = false)] allow_downloads: bool,
    on_search: Callback<RegistryBrowseRequest>,
    on_open_project: Callback<RegistryProjectRef>,
    on_download_profile: Callback<RegistryBrowseProject>,
    on_add_to_profile: Callback<RegistryAddToProfileRequest>,
) -> impl IntoView {
    view! {
        <CatalogPage
            results
            profiles
            pending
            action_pending
            error
            warnings
            notice
            allow_downloads
            on_search
            on_open_project
            on_download_profile
            on_add_to_profile
        />
    }
}

#[component]
pub fn UploadPage(
    authenticated: Signal<bool>,
    github_connected: Signal<bool>,
    scan: Signal<Option<RegistryScan>>,
    progress: Signal<Option<RegistryScanProgress>>,
    pending: Signal<bool>,
    error: Signal<Option<String>>,
    notice: Signal<Option<String>>,
    on_sign_in: Callback<()>,
    on_connect_github: Callback<()>,
    on_scan: Callback<RegistryScanRequest>,
    on_publish: Callback<RegistryPublishRequest>,
    on_open_project: Callback<RegistryProjectRef>,
) -> impl IntoView {
    let (repository_url, set_repository_url) = signal(String::new());
    let (base_path, set_base_path) = signal(String::new());
    let (selected_scan_id, set_selected_scan_id) = signal(None::<String>);
    let (selected_entries, set_selected_entries) = signal(HashSet::<String>::new());

    Effect::new(move |_| {
        let current = scan.get();
        let current_id = current.as_ref().map(|scan| scan.scan_id.clone());
        if selected_scan_id.get_untracked() != current_id {
            let selected = current
                .as_ref()
                .map(|scan| {
                    scan.entries
                        .iter()
                        .filter(|entry| entry.is_publishable())
                        .map(|entry| entry.entry_id.clone())
                        .collect()
                })
                .unwrap_or_default();
            set_selected_entries.set(selected);
            set_selected_scan_id.set(current_id);
        }
    });

    let submit_scan = move |event: leptos::ev::SubmitEvent| {
        event.prevent_default();
        let repository_url = repository_url.get().trim().to_owned();
        if repository_url.is_empty() || pending.get_untracked() {
            return;
        }
        on_scan.run(RegistryScanRequest {
            repository_url,
            base_path: base_path.get().trim().to_owned(),
        });
    };

    let publish = move |_| {
        if pending.get_untracked() {
            return;
        }
        let mut entry_ids = selected_entries
            .get_untracked()
            .into_iter()
            .collect::<Vec<_>>();
        entry_ids.sort();
        if !entry_ids.is_empty() {
            on_publish.run(RegistryPublishRequest { entry_ids });
        }
    };

    view! {
        <div class="upload-layout">
            <section class="upload-heading">
                <div>
                    <p class="catalog-kicker">"Publisher console"</p>
                    <h1>"Upload"</h1>
                    <p>"Publish mod and modpack versions from a GitHub repository."</p>
                </div>
            </section>

            <Show
                when=move || authenticated.get()
                fallback=move || view! {
                    <UploadGate
                        title="Sign in to publish"
                        message="A Patchwork account is required to own published projects."
                        action="Sign in"
                        on_action=on_sign_in
                    />
                }
            >
                <Show
                    when=move || github_connected.get()
                    fallback=move || view! {
                        <UploadGate
                            title="Connect GitHub"
                            message="Link the GitHub identity that has write access to the repository."
                            action="Connect GitHub"
                            on_action=on_connect_github
                        />
                    }
                >
                    <form class="upload-scan-form" on:submit=submit_scan>
                        <div class="upload-field-grid">
                            <label class="upload-field upload-repository-field">
                                <span>"GitHub repository"</span>
                                <input
                                    type="url"
                                    inputmode="url"
                                    autocomplete="url"
                                    placeholder="https://github.com/owner/repository"
                                    required
                                    prop:value=move || repository_url.get()
                                    on:input=move |event| set_repository_url.set(event_target_value(&event))
                                />
                            </label>
                            <label class="upload-field">
                                <span>"Subdirectory or modpack file" <small>"Optional"</small></span>
                                <input
                                    placeholder="mods or modpacks/example.toml"
                                    prop:value=move || base_path.get()
                                    on:input=move |event| set_base_path.set(event_target_value(&event))
                                />
                            </label>
                        </div>
                        <button
                            type="submit"
                            class="catalog-primary-action"
                            disabled=move || pending.get() || repository_url.get().trim().is_empty()
                        >
                            <SearchIcon />
                            <span>{move || {
                                if pending.get() && scan.get().is_none() {
                                    "Scanning..."
                                } else {
                                    "Scan"
                                }
                            }}</span>
                        </button>
                    </form>

                    <Show when=move || error.get().is_some()>
                        <p class="upload-feedback error" role="alert">
                            {move || error.get().unwrap_or_default()}
                        </p>
                        <Show when=move || error.get().is_some_and(|message| {
                            message.contains("GitHub App is not installed")
                        })>
                            <p class="upload-install-tip">
                                <strong>"Repository access required."</strong>
                                " Install Patchwork's GitHub App on this repository, then run the scan again."
                            </p>
                        </Show>
                    </Show>
                    <Show when=move || notice.get().is_some()>
                        <p class="upload-feedback success">
                            {move || notice.get().unwrap_or_default()}
                        </p>
                    </Show>

                    {move || progress.get().filter(|progress| {
                        !progress.phase.is_finished()
                    }).map(|progress| view! {
                        <ScanProgressPanel
                            progress
                            selected_entries
                            set_selected_entries
                            on_open_project
                        />
                    })}

                    {move || scan.get().map(|scan| view! {
                        <ScanPreview
                            scan
                            selected_entries
                            set_selected_entries
                            pending
                            on_open_project
                            on_publish=Callback::new(move |()| publish(()))
                        />
                    })}
                </Show>
            </Show>
        </div>
    }
}

#[component]
fn UploadGate(
    title: &'static str,
    message: &'static str,
    action: &'static str,
    on_action: Callback<()>,
) -> impl IntoView {
    view! {
        <section class="upload-gate">
            <UploadIcon />
            <div>
                <h2>{title}</h2>
                <p>{message}</p>
            </div>
            <button
                type="button"
                class="catalog-primary-action"
                on:click=move |_| on_action.run(())
            >
                {action}
            </button>
        </section>
    }
}

#[component]
fn ScanPreview(
    scan: RegistryScan,
    selected_entries: ReadSignal<HashSet<String>>,
    set_selected_entries: WriteSignal<HashSet<String>>,
    pending: Signal<bool>,
    on_open_project: Callback<RegistryProjectRef>,
    on_publish: Callback<()>,
) -> impl IntoView {
    let repository_name = format!("{}/{}", scan.repository.owner, scan.repository.name);
    let resolved_commit = scan.resolved_commit.clone();
    let entries = scan.entries.clone();
    let selected_count = move || selected_entries.get().len();
    let is_published = scan.published_at.is_some();

    view! {
        <section class="scan-preview">
            <header class="scan-preview-heading">
                <div>
                    <p class="catalog-kicker">"Verified snapshot"</p>
                    <h2>{repository_name}</h2>
                    <p>
                        <span>{format!("{} at ", display_repository_path(&scan.base_path))}</span>
                        <code title=resolved_commit.clone()>{short_oid(&resolved_commit)}</code>
                    </p>
                </div>
                <span class="scan-count">{entries.len()} " found"</span>
            </header>

            <For
                each=move || scan.warnings.clone()
                key=|warning| warning.clone()
                children=move |warning| view! { <p class="upload-feedback warning">{warning}</p> }
            />
            <For
                each=move || scan.errors.clone()
                key=|scan_error| scan_error.clone()
                children=move |scan_error| view! { <p class="upload-feedback error">{scan_error}</p> }
            />

            <div class="scan-entry-list">
                <For
                    each=move || entries.clone()
                    key=|entry| entry.entry_id.clone()
                    children=move |entry| view! {
                        <ScanEntryCard
                            entry
                            selected_entries
                            set_selected_entries
                            interaction_disabled=false
                            on_open_project
                        />
                    }
                />
            </div>

            <Show when=move || !scan.entries.is_empty()>
                <footer class="scan-publish-bar">
                    <div>
                        <strong>{move || selected_count()}</strong>
                        <span>" versions selected"</span>
                    </div>
                    <button
                        type="button"
                        class="catalog-primary-action scan-publish-action"
                        disabled=move || pending.get() || selected_count() == 0 || is_published
                        on:click=move |_| on_publish.run(())
                    >
                        <UploadIcon />
                        <span>{move || if is_published {
                            "Published"
                        } else if pending.get() {
                            "Publishing"
                        } else {
                            "Publish / Update"
                        }}</span>
                    </button>
                </footer>
            </Show>
        </section>
    }
}

#[component]
fn ScanProgressPanel(
    progress: RegistryScanProgress,
    selected_entries: ReadSignal<HashSet<String>>,
    set_selected_entries: WriteSignal<HashSet<String>>,
    on_open_project: Callback<RegistryProjectRef>,
) -> impl IntoView {
    let phase = progress.phase;
    let completed = progress.completed;
    let total = progress.total;
    let entries = progress.entries;
    let entries_count = entries.len();
    let width = total
        .filter(|total| *total > 0)
        .map(|total| format!("{}%", completed.saturating_mul(100) / total))
        .unwrap_or_else(|| "32%".to_owned());
    let progress_class = if total.is_some() {
        "scan-progress-fill"
    } else {
        "scan-progress-fill indeterminate"
    };
    let entries_view = if entries.is_empty() {
        view! { <></> }.into_any()
    } else {
        view! {
            <div class="scan-entry-list scan-progress-entries">
                <For
                    each=move || entries.clone()
                    key=|entry| entry.entry_id.clone()
                    children=move |entry| view! {
                        <ScanEntryCard
                            entry
                            selected_entries
                            set_selected_entries
                            interaction_disabled=true
                            on_open_project
                        />
                    }
                />
            </div>
        }
        .into_any()
    };

    view! {
        <section class="scan-progress" aria-live="polite">
            <div class="scan-progress-copy">
                <div>
                    <strong>{scan_phase_label(phase)}</strong>
                    <span>{scan_progress_count(phase, completed, total)}</span>
                </div>
                <span>{format!(
                    "{} project{} ready",
                    entries_count,
                    if entries_count == 1 { "" } else { "s" },
                )}</span>
            </div>
            <div class="scan-progress-track" aria-hidden="true">
                <span class=progress_class style=format!("width: {width}")></span>
            </div>
            {entries_view}
        </section>
    }
}

#[component]
fn ScanEntryCard(
    entry: RegistryScanEntry,
    selected_entries: ReadSignal<HashSet<String>>,
    set_selected_entries: WriteSignal<HashSet<String>>,
    interaction_disabled: bool,
    on_open_project: Callback<RegistryProjectRef>,
) -> impl IntoView {
    let publishable = entry.is_publishable();
    let generated_entry =
        entry.project_kind == RegistryProjectKind::Mod && is_generated_mod_id(&entry.project_id);
    let entry_id_for_checked = entry.entry_id.clone();
    let entry_id_for_change = entry.entry_id.clone();
    let card_class = format!("scan-entry {}", status_class(entry.status));
    let tree_oid = entry.source_tree_oid.clone();
    let manifest_oid = entry.manifest_blob_oid.clone();
    let entry_target = RegistryProjectRef {
        project_kind: entry.project_kind,
        project_id: entry.project_id.clone(),
    };
    let dependencies = entry.dependencies.clone();
    let warnings = entry.warnings.clone();
    let errors = entry.errors.clone();
    let dependency_section = if dependencies.is_empty() {
        view! { <></> }.into_any()
    } else {
        view! {
            <div class="scan-dependencies">
                <strong>"Dependencies"</strong>
                <div>
                    <For
                        each=move || dependencies.clone()
                        key=|dependency| (
                            dependency.kind,
                            dependency.target_kind,
                            dependency.target_id.clone(),
                        )
                        children=move |dependency| {
                            let generated = dependency.target_kind == RegistryProjectKind::Mod
                                && is_generated_mod_id(&dependency.target_id);
                            let clickable = dependency.available && !generated;
                            let target = RegistryProjectRef {
                                project_kind: dependency.target_kind,
                                project_id: dependency.target_id.clone(),
                            };
                            view! {
                            <button
                                type="button"
                                class=if generated { "dependency-chip generated" } else if dependency.available { "dependency-chip" } else { "dependency-chip missing" }
                                disabled=!clickable
                                on:click=move |_| on_open_project.run(target.clone())
                            >
                                {dependency_kind(dependency.kind)} " · "
                                {dependency_target(dependency.target_kind, &dependency.target_id)}
                                {generated.then_some(" · generated during compose")}
                            </button>
                            }
                        }
                    />
                </div>
            </div>
        }
        .into_any()
    };

    view! {
        <article class=card_class>
            <div class="scan-entry-select">
                <input
                    type="checkbox"
                    aria-label=format!("Select {} {}", entry.project_id, entry.version)
                    disabled=!publishable || interaction_disabled
                    prop:checked=move || selected_entries.get().contains(&entry_id_for_checked)
                    on:change=move |event| {
                        let checked = event_target_checked(&event);
                        set_selected_entries.update(|selected| {
                            if checked {
                                selected.insert(entry_id_for_change.clone());
                            } else {
                                selected.remove(&entry_id_for_change);
                            }
                        });
                    }
                />
            </div>
            <div class="scan-entry-body">
                <header class="scan-entry-heading">
                    <div>
                        <button
                            type="button"
                            class="scan-project-link"
                            disabled=generated_entry
                            on:click=move |_| on_open_project.run(entry_target.clone())
                        >
                            <h3>{entry.title}</h3>
                        </button>
                        <p>
                            <span class="scan-project-kind">{project_kind(entry.project_kind)}</span>
                            <code>{entry.project_id}</code>
                            <span>{entry.version}</span>
                        </p>
                    </div>
                    <span class=format!("scan-status {}", status_class(entry.status))>
                        {status_label(entry.status, entry.project_kind)}
                    </span>
                </header>

                <dl class="scan-entry-details">
                    <div><dt>"Directory"</dt><dd><code>{entry.repository_path}</code></dd></div>
                    <div><dt>"Manifest"</dt><dd><code>{entry.manifest_path}</code></dd></div>
                    <div><dt>"Source tree"</dt><dd><code title=tree_oid.clone()>{short_oid(&tree_oid)}</code></dd></div>
                    <div><dt>"Manifest blob"</dt><dd><code title=manifest_oid.clone()>{short_oid(&manifest_oid)}</code></dd></div>
                </dl>

                {dependency_section}

                <For
                    each=move || warnings.clone()
                    key=|warning| warning.clone()
                    children=move |warning| view! { <p class="entry-message warning">{warning}</p> }
                />
                <For
                    each=move || errors.clone()
                    key=|entry_error| entry_error.clone()
                    children=move |entry_error| view! { <p class="entry-message error">{entry_error}</p> }
                />
            </div>
        </article>
    }
}

fn scan_phase_label(phase: RegistryScanPhase) -> &'static str {
    match phase {
        RegistryScanPhase::Queued => "Queued",
        RegistryScanPhase::Authorizing => "Checking repository access",
        RegistryScanPhase::IndexingRepository => "Reading repository tree",
        RegistryScanPhase::FetchingManifests => "Downloading manifests",
        RegistryScanPhase::ValidatingProjects => "Validating projects",
        RegistryScanPhase::Persisting => "Saving scan preview",
        RegistryScanPhase::Complete => "Scan complete",
        RegistryScanPhase::Failed => "Scan failed",
    }
}

fn scan_progress_count(phase: RegistryScanPhase, completed: u32, total: Option<u32>) -> String {
    let unit = match phase {
        RegistryScanPhase::IndexingRepository => "directories",
        RegistryScanPhase::FetchingManifests => "manifests",
        RegistryScanPhase::ValidatingProjects => "projects",
        RegistryScanPhase::Persisting => "preview",
        _ => "",
    };
    match total {
        Some(total) if !unit.is_empty() => format!("{completed} / {total} {unit}"),
        Some(total) => format!("{completed} / {total}"),
        None => "Working...".to_owned(),
    }
}

fn status_label(status: RegistryScanStatus, project_kind: RegistryProjectKind) -> &'static str {
    match status {
        RegistryScanStatus::NewMod => match project_kind {
            RegistryProjectKind::Mod => "NEW MOD",
            RegistryProjectKind::Modpack => "NEW MODPACK",
        },
        RegistryScanStatus::NewVersion => "NEW VERSION",
        RegistryScanStatus::Unchanged => "ALREADY PUBLISHED",
        RegistryScanStatus::VersionConflict => "VERSION CONFLICT",
        RegistryScanStatus::Error => "ERROR",
    }
}

fn status_class(status: RegistryScanStatus) -> &'static str {
    match status {
        RegistryScanStatus::NewMod => "new-mod",
        RegistryScanStatus::NewVersion => "new-version",
        RegistryScanStatus::Unchanged => "unchanged",
        RegistryScanStatus::VersionConflict => "conflict",
        RegistryScanStatus::Error => "error",
    }
}

fn dependency_kind(kind: RegistryDependencyKind) -> &'static str {
    match kind {
        RegistryDependencyKind::Init => "init",
        RegistryDependencyKind::Run => "run",
        RegistryDependencyKind::Ownership => "ownership",
        RegistryDependencyKind::Provides => "provides",
        RegistryDependencyKind::Mod => "mod",
        RegistryDependencyKind::Modpack => "modpack",
        RegistryDependencyKind::Ignore => "ignore",
    }
}

fn project_kind(kind: RegistryProjectKind) -> &'static str {
    match kind {
        RegistryProjectKind::Mod => "MOD",
        RegistryProjectKind::Modpack => "MODPACK",
    }
}

fn dependency_target(kind: RegistryProjectKind, id: &str) -> String {
    match kind {
        RegistryProjectKind::Mod => id.to_owned(),
        RegistryProjectKind::Modpack => format!("modpack/{id}"),
    }
}

fn display_repository_path(path: &str) -> &str {
    if path == "." { "repository root" } else { path }
}

fn short_oid(oid: &str) -> String {
    oid.chars().take(12).collect()
}

#[component]
pub fn CatalogPage(
    results: Signal<Vec<RegistryBrowseProject>>,
    profiles: Signal<Vec<RegistryProfileOption>>,
    pending: Signal<bool>,
    action_pending: Signal<Option<String>>,
    error: Signal<Option<String>>,
    warnings: Signal<Vec<String>>,
    notice: Signal<Option<String>>,
    allow_downloads: bool,
    on_search: Callback<RegistryBrowseRequest>,
    on_open_project: Callback<RegistryProjectRef>,
    on_download_profile: Callback<RegistryBrowseProject>,
    on_add_to_profile: Callback<RegistryAddToProfileRequest>,
) -> impl IntoView {
    let (query, set_query) = signal(String::new());
    let (include_mods, set_include_mods) = signal(true);
    let (include_modpacks, set_include_modpacks) = signal(true);
    let submit = move |event: leptos::ev::SubmitEvent| {
        event.prevent_default();
        if pending.get_untracked()
            || (!include_mods.get_untracked() && !include_modpacks.get_untracked())
        {
            return;
        }
        on_search.run(RegistryBrowseRequest {
            query: query.get_untracked().trim().to_owned(),
            include_mods: include_mods.get_untracked(),
            include_modpacks: include_modpacks.get_untracked(),
        });
    };

    view! {
        <div class="catalog-layout">
            <section class="catalog-hero">
                <div>
                    <p class="catalog-kicker">"Patchwork registry"</p>
                    <h1>"Browse"</h1>
                    <p>"Explore Patchwork mods, APIs, support packages and complete modpacks."</p>
                </div>

                <form class="catalog-search" on:submit=submit>
                    <SearchIcon />
                    <input
                        type="search"
                        placeholder="Search by keyword or ID"
                        aria-label="Search mods and modpacks"
                        prop:value=move || query.get()
                        on:input=move |event| set_query.set(event_target_value(&event))
                    />
                    <button
                        type="submit"
                        class="catalog-search-action"
                        disabled=move || pending.get() || (!include_mods.get() && !include_modpacks.get())
                    >
                        {move || if pending.get() { "Searching..." } else { "Search" }}
                    </button>
                </form>
            </section>

            <div class="catalog-browser">
                <aside class="catalog-sidebar" aria-label="Browse filters">
                    <strong>"Project type"</strong>
                    <label class="catalog-check-filter">
                        <input
                            type="checkbox"
                            prop:checked=move || include_mods.get()
                            on:change=move |event| set_include_mods.set(event_target_checked(&event))
                        />
                        <span>"Mods"</span>
                    </label>
                    <label class="catalog-check-filter">
                        <input
                            type="checkbox"
                            prop:checked=move || include_modpacks.get()
                            on:change=move |event| set_include_modpacks.set(event_target_checked(&event))
                        />
                        <span>"Modpacks"</span>
                    </label>
                </aside>

                <section class="catalog-results" aria-live="polite">
                    <Show when=move || error.get().is_some()>
                        <p class="catalog-feedback error">{move || error.get().unwrap_or_default()}</p>
                    </Show>
                    <For
                        each=move || warnings.get()
                        key=|warning| warning.clone()
                        children=move |warning| view! { <p class="catalog-feedback warning">{warning}</p> }
                    />
                    <Show when=move || notice.get().is_some()>
                        <p class="catalog-feedback success">{move || notice.get().unwrap_or_default()}</p>
                    </Show>
                    <Show
                        when=move || pending.get() || !results.get().is_empty()
                        fallback=move || view! {
                            <div class="catalog-empty">
                                <SearchIcon />
                                <strong>"No projects found"</strong>
                            </div>
                        }
                    >
                        <Show when=move || pending.get() && results.get().is_empty()>
                            <div class="catalog-loading">
                                <span></span>
                                <strong>"Searching registries..."</strong>
                            </div>
                        </Show>
                        <div class="catalog-grid">
                            <For
                                each=move || results.get()
                                key=|project| (
                                    project.project_kind,
                                    project.project_id.clone(),
                                    project.source,
                                    project.source_label.clone(),
                                    project.local_manifest_path.clone().or(project.repository_path.clone()),
                                    project.version.clone(),
                                )
                                children=move |project| view! {
                                    <CatalogProject
                                        project
                                        profiles
                                        action_pending
                                        allow_downloads
                                        on_open_project
                                        on_download_profile
                                        on_add_to_profile
                                    />
                                }
                            />
                        </div>
                    </Show>
                </section>
            </div>
        </div>
    }
}

#[component]
fn CatalogProject(
    project: RegistryBrowseProject,
    profiles: Signal<Vec<RegistryProfileOption>>,
    action_pending: Signal<Option<String>>,
    allow_downloads: bool,
    on_open_project: Callback<RegistryProjectRef>,
    on_download_profile: Callback<RegistryBrowseProject>,
    on_add_to_profile: Callback<RegistryAddToProfileRequest>,
) -> impl IntoView {
    let (project_for_download, _) = signal(project.clone());
    let (project_for_profile, _) = signal(project.clone());
    let (project_ref, _) = signal(project.project_ref());
    let (show_profile_menu, set_show_profile_menu) = signal(false);
    let (action_key, _) = signal(browse_action_key(&project));
    let is_modpack = project.project_kind == RegistryProjectKind::Modpack;
    let generated = project.project_kind == RegistryProjectKind::Mod
        && is_generated_mod_id(&project.project_id);
    let accent = if is_modpack { "#fdb22c" } else { "#02a9a9" };
    let kind = if is_modpack { "Modpack" } else { "Mod" };
    let source = match project.source {
        RegistryBrowseSource::Remote => format!("GitHub · {}", project.source_label),
        RegistryBrowseSource::Local => format!("Local · {}", project.source_label),
    };
    let is_remote = project.source == RegistryBrowseSource::Remote;
    let description = if project.description.trim().is_empty() {
        format!("Patchwork {} {}", kind.to_lowercase(), project.project_id)
    } else {
        project.description.clone()
    };
    let image_url = project.image_url.clone();
    let title = project.title.clone();
    let project_id = project.project_id.clone();
    let version = project.version.clone();
    let downloads = project.downloads;

    view! {
        <article class="catalog-item" style=format!("--item-accent: {accent}")>
            <span class="catalog-swatch"></span>
            <div class="catalog-item-body">
                <div class="catalog-item-heading">
                    <div class="catalog-project-title">
                        {image_url.map(|image_url| view! {
                            <img src=image_url alt="" loading="lazy" />
                        })}
                        <div>
                            <button
                                type="button"
                                class="catalog-title-button"
                                disabled=generated || (allow_downloads && !is_remote)
                                on:click=move |_| on_open_project.run(project_ref.get_untracked())
                            >
                                <h2>{title}</h2>
                            </button>
                            <code>{project_id}</code>
                            <p>{description}</p>
                        </div>
                    </div>
                    <span class="catalog-kind">{kind}</span>
                </div>

                <div class="catalog-meta">
                    <span>{format!("v{version}")}</span>
                    <span>{if is_remote {
                        format!("{} downloads", format_downloads(downloads))
                    } else {
                        "Downloads -".to_owned()
                    }}</span>
                    <span>{source}</span>
                </div>

                <Show when=move || allow_downloads && !generated>
                    <div class="catalog-actions catalog-download-actions">
                        <Show when=move || is_modpack>
                            <button
                                type="button"
                                class="catalog-secondary-action"
                                disabled=move || action_pending.get().as_deref() == Some(action_key.get().as_str())
                                on:click=move |_| on_download_profile.run(project_for_download.get_untracked())
                            >
                                "Download as profile"
                            </button>
                        </Show>
                        <Show
                            when=move || !profiles.get().is_empty()
                            fallback=move || view! { <span class="catalog-no-profiles">"No profiles available"</span> }
                        >
                            <div class="catalog-profile-action">
                                <button
                                    type="button"
                                    class="catalog-secondary-action"
                                    aria-expanded=move || show_profile_menu.get().to_string()
                                    disabled=move || action_pending.get().as_deref() == Some(action_key.get().as_str())
                                    on:click=move |_| set_show_profile_menu.update(|show| *show = !*show)
                                >
                                    "Add to existing profile"
                                </button>
                                <Show when=move || show_profile_menu.get()>
                                    <div class="catalog-profile-menu" role="menu">
                                        <For
                                            each=move || profiles.get()
                                            key=|profile| profile.id.clone()
                                            children=move |profile| {
                                                let profile_id = profile.id.clone();
                                                view! {
                                                    <button
                                                        type="button"
                                                        role="menuitem"
                                                        on:click=move |_| {
                                                            set_show_profile_menu.set(false);
                                                            on_add_to_profile.run(RegistryAddToProfileRequest {
                                                                project: project_ref.get_untracked(),
                                                                selected_project: Some(project_for_profile.get_untracked()),
                                                                profile_id: profile_id.clone(),
                                                            });
                                                        }
                                                    >
                                                        <strong>{profile.name}</strong>
                                                        <code>{profile.id}</code>
                                                    </button>
                                                }
                                            }
                                        />
                                    </div>
                                </Show>
                            </div>
                        </Show>
                    </div>
                </Show>
            </div>
        </article>
    }
}

fn browse_action_key(project: &RegistryBrowseProject) -> String {
    format!(
        "{}:{}",
        project.project_kind.route_segment(),
        project.project_id
    )
}

fn format_downloads(downloads: i64) -> String {
    if downloads >= 1_000_000 {
        format!("{:.1}M", downloads as f64 / 1_000_000.0)
    } else if downloads >= 1_000 {
        format!("{:.1}K", downloads as f64 / 1_000.0)
    } else {
        downloads.to_string()
    }
}
