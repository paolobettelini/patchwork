use std::collections::HashSet;

use leptos::prelude::*;
use patchwork_registry_types::{
    RegistryDependencyKind, RegistryPublishRequest, RegistryScan, RegistryScanEntry,
    RegistryScanPhase, RegistryScanProgress, RegistryScanRequest, RegistryScanStatus,
};

use crate::icons::{SearchIcon, UploadIcon};

#[derive(Clone, Copy)]
struct CatalogItem {
    name: &'static str,
    kind: &'static str,
    summary: &'static str,
    version: &'static str,
    downloads: &'static str,
    accent: &'static str,
}

const BROWSE_ITEMS: [CatalogItem; 4] = [
    CatalogItem {
        name: "Inventory Loom",
        kind: "Mod",
        summary: "Shared inventory primitives for client and server mods.",
        version: "1.21.4",
        downloads: "18.6K",
        accent: "#02a9a9",
    },
    CatalogItem {
        name: "Copper Trails",
        kind: "Modpack",
        summary: "A compact exploration pack stitched around lightweight worldgen.",
        version: "1.21.x",
        downloads: "9.2K",
        accent: "#fdb22c",
    },
    CatalogItem {
        name: "UI Stitch",
        kind: "API",
        summary: "Composable menu surfaces and interaction contracts.",
        version: "0.7",
        downloads: "31.4K",
        accent: "#6268c8",
    },
    CatalogItem {
        name: "Redstone Cloth",
        kind: "Asset pack",
        summary: "Texture and sound assets for technical modpacks.",
        version: "2.0",
        downloads: "6.8K",
        accent: "#fd614e",
    },
];

#[component]
pub fn BrowsePage(#[prop(default = false)] allow_downloads: bool) -> impl IntoView {
    view! { <CatalogPage allow_downloads=allow_downloads /> }
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
                    <p>"Publish mod versions from a GitHub repository."</p>
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
                                <span>"Subdirectory" <small>"Optional"</small></span>
                                <input
                                    placeholder="mods/my-mod"
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
                        />
                    })}

                    {move || scan.get().map(|scan| view! {
                        <ScanPreview
                            scan
                            selected_entries
                            set_selected_entries
                            pending
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
                    "{} mod{} ready",
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
) -> impl IntoView {
    let publishable = entry.is_publishable();
    let entry_id_for_checked = entry.entry_id.clone();
    let entry_id_for_change = entry.entry_id.clone();
    let card_class = format!("scan-entry {}", status_class(entry.status));
    let tree_oid = entry.source_tree_oid.clone();
    let manifest_oid = entry.manifest_blob_oid.clone();
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
                        key=|dependency| (dependency.kind, dependency.target_id.clone())
                        children=move |dependency| view! {
                            <span class=if dependency.available { "dependency-chip" } else { "dependency-chip missing" }>
                                {dependency_kind(dependency.kind)} " · " {dependency.target_id}
                            </span>
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
                    aria-label=format!("Select {} {}", entry.mod_id, entry.version)
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
                        <h3>{entry.title}</h3>
                        <p><code>{entry.mod_id}</code> <span>{entry.version}</span></p>
                    </div>
                    <span class=format!("scan-status {}", status_class(entry.status))>
                        {status_label(entry.status)}
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
        RegistryScanPhase::FetchingManifests => "Downloading Cargo manifests",
        RegistryScanPhase::ValidatingMods => "Validating mods",
        RegistryScanPhase::Persisting => "Saving scan preview",
        RegistryScanPhase::Complete => "Scan complete",
        RegistryScanPhase::Failed => "Scan failed",
    }
}

fn scan_progress_count(phase: RegistryScanPhase, completed: u32, total: Option<u32>) -> String {
    let unit = match phase {
        RegistryScanPhase::IndexingRepository => "directories",
        RegistryScanPhase::FetchingManifests => "manifests",
        RegistryScanPhase::ValidatingMods => "mods",
        RegistryScanPhase::Persisting => "preview",
        _ => "",
    };
    match total {
        Some(total) if !unit.is_empty() => format!("{completed} / {total} {unit}"),
        Some(total) => format!("{completed} / {total}"),
        None => "Working...".to_owned(),
    }
}

fn status_label(status: RegistryScanStatus) -> &'static str {
    match status {
        RegistryScanStatus::NewMod => "NEW MOD",
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
    }
}

fn display_repository_path(path: &str) -> &str {
    if path == "." { "repository root" } else { path }
}

fn short_oid(oid: &str) -> String {
    oid.chars().take(12).collect()
}

#[component]
pub fn CatalogPage(#[prop(default = false)] allow_downloads: bool) -> impl IntoView {
    view! {
        <div class="catalog-layout">
            <section class="catalog-hero">
                <div>
                    <p class="catalog-kicker">"Patchwork registry"</p>
                    <h1>"Browse"</h1>
                    <p>"Explore Patchwork mods, APIs, support packages and complete modpacks."</p>
                </div>

                <div class="catalog-search">
                    <SearchIcon />
                    <input type="search" placeholder="Search mods and modpacks" aria-label="Search mods and modpacks" />
                </div>
            </section>

            <section class="catalog-toolbar" aria-label="Catalogue filters">
                <button type="button" class="catalog-filter active">"Featured"</button>
                <button type="button" class="catalog-filter">"Mods"</button>
                <button type="button" class="catalog-filter">"Modpacks"</button>
                <button type="button" class="catalog-filter">"APIs"</button>
            </section>

            <section class="catalog-grid">
                <For
                    each=move || BROWSE_ITEMS.to_vec()
                    key=|item| item.name
                    children=move |item| view! {
                        <article class="catalog-item">
                            <span class="catalog-swatch" style=format!("--item-accent: {}", item.accent)></span>
                            <div class="catalog-item-body">
                                <div class="catalog-item-heading">
                                    <div>
                                        <h2>{item.name}</h2>
                                        <p>{item.summary}</p>
                                    </div>
                                    <span class="catalog-kind">{item.kind}</span>
                                </div>

                                <div class="catalog-meta">
                                    <span>{item.version}</span>
                                    <span>{item.downloads} " downloads"</span>
                                </div>

                                <div class="catalog-actions">
                                    <button type="button" class="catalog-secondary-action">
                                        {if allow_downloads { "Download" } else { "View" }}
                                    </button>
                                </div>
                            </div>
                        </article>
                    }
                />
            </section>
        </div>
    }
}
