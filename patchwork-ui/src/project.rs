use leptos::prelude::*;
use patchwork_registry_types::{
    RegistryDependency, RegistryDependencyKind, RegistryProjectDetails, RegistryProjectKind,
    RegistryProjectRef, is_generated_mod_id,
};

#[component]
pub fn RegistryProjectPage(
    details: Signal<Option<RegistryProjectDetails>>,
    pending: Signal<bool>,
    error: Signal<Option<String>>,
    on_open_dependency: Callback<RegistryProjectRef>,
    on_open_dependency_tree: Option<Callback<RegistryProjectRef>>,
    on_open_publisher: Callback<String>,
) -> impl IntoView {
    let (active_tab, set_active_tab) = signal("details");

    view! {
        <div class="registry-project-page">
            <Show when=move || pending.get()>
                <div class="project-page-state">
                    <span class="project-page-spinner"></span>
                    <strong>"Loading project..."</strong>
                </div>
            </Show>
            <Show when=move || error.get().is_some()>
                <div class="project-page-state error">
                    <strong>"Project unavailable"</strong>
                    <p>{move || error.get().unwrap_or_default()}</p>
                </div>
            </Show>

            {move || details.get().map(|project| {
                let kind = project_kind_label(project.project_kind);
                let image_url = project.image_url.clone();
                let title = project.title.clone();
                let project_id = project.project_id.clone();
                let description = if project.description.trim().is_empty() {
                    format!("Patchwork {} {project_id}", kind.to_lowercase())
                } else {
                    project.description.clone()
                };
                let version = project.version.clone();
                let downloads = format_downloads(project.downloads);
                let dependencies = project.dependencies.clone();
                let dependency_count = dependencies.len();
                let details_for_panel = project.clone();
                let dependency_tree_action = on_open_dependency_tree.clone().map(|open_tree| {
                    let project_ref = RegistryProjectRef {
                        project_kind: project.project_kind,
                        project_id: project.project_id.clone(),
                    };
                    view! {
                        <button
                            type="button"
                            class="registry-dependency-tree-action"
                            on:click=move |_| open_tree.run(project_ref.clone())
                        >
                            "Dependency graph"
                        </button>
                    }
                });

                view! {
                    <section class="registry-project-hero">
                        <div class="registry-project-identity">
                            {image_url.map(|url| view! { <img src=url alt="" /> })}
                            <div>
                                <p class="catalog-kicker">{kind}</p>
                                <h1>{title}</h1>
                                <p>{description}</p>
                                {dependency_tree_action}
                            </div>
                        </div>
                        <div class="registry-project-stats">
                            <div><span>"ID"</span><strong>{project_id}</strong></div>
                            <div><span>"Version"</span><strong>{version}</strong></div>
                            <div><span>"Dependencies"</span><strong>{dependency_count}</strong></div>
                            <div><span>"Downloads"</span><strong>{downloads}</strong></div>
                        </div>
                    </section>

                    <section class="registry-project-workspace">
                        <div class="detail-tabs" aria-label="Project details">
                            <button
                                type="button"
                                class=move || detail_tab_class(active_tab.get() == "details")
                                on:click=move |_| set_active_tab.set("details")
                            >
                                "Details"
                            </button>
                            <button
                                type="button"
                                class=move || detail_tab_class(active_tab.get() == "dependencies")
                                on:click=move |_| set_active_tab.set("dependencies")
                            >
                                "Dependencies"
                            </button>
                        </div>
                        <Show
                            when=move || active_tab.get() == "details"
                            fallback=move || view! {
                                <RegistryDependencyList
                                    dependencies=dependencies.clone()
                                    on_open_dependency
                                />
                            }
                        >
                            <RegistryDetails
                                details=details_for_panel.clone()
                                on_open_publisher
                            />
                        </Show>
                    </section>
                }
            })}
        </div>
    }
}

#[component]
fn RegistryDetails(
    details: RegistryProjectDetails,
    on_open_publisher: Callback<String>,
) -> impl IntoView {
    let publisher_name = details.publisher_name;
    let publisher_for_click = publisher_name.clone();
    let publisher_uuid = details.publisher_uuid;
    let published_at = display_date(&details.published_at);
    let repository_url = details.repository_url;
    let repository_link = repository_url.clone();
    let repository_path = display_path(&details.repository_path).to_owned();
    let commit = details.source_commit.clone();
    let tree = details.source_tree_oid.clone();
    let manifest = details.manifest_sha256.clone();
    view! {
        <dl class="registry-details-grid">
            <div>
                <dt>"Publisher"</dt>
                <dd>
                    <button
                        type="button"
                        class="registry-publisher-link"
                        on:click=move |_| on_open_publisher.run(publisher_for_click.clone())
                    >
                        {publisher_name}
                    </button>
                    <code>{publisher_uuid}</code>
                </dd>
            </div>
            <div><dt>"Published"</dt><dd>{published_at}</dd></div>
            <div><dt>"Repository"</dt><dd><a href=repository_link target="_blank" rel="noreferrer">{repository_url}</a></dd></div>
            <div><dt>"Directory"</dt><dd><code>{repository_path}</code></dd></div>
            <div><dt>"Source commit"</dt><dd><code title=commit.clone()>{short_hash(&commit)}</code></dd></div>
            <div><dt>"Source tree"</dt><dd><code title=tree.clone()>{short_hash(&tree)}</code></dd></div>
            <div><dt>"Manifest SHA-256"</dt><dd><code title=manifest.clone()>{short_hash(&manifest)}</code></dd></div>
        </dl>
    }
}

#[component]
fn RegistryDependencyList(
    dependencies: Vec<RegistryDependency>,
    on_open_dependency: Callback<RegistryProjectRef>,
) -> impl IntoView {
    if dependencies.is_empty() {
        return view! {
            <div class="registry-dependency-empty">"This version has no declared dependencies."</div>
        }
        .into_any();
    }

    view! {
        <div class="registry-dependency-list">
            {dependencies.into_iter().map(|dependency| {
                let generated = dependency.target_kind == RegistryProjectKind::Mod
                    && is_generated_mod_id(&dependency.target_id);
                let clickable = dependency.available && !generated;
                let target = RegistryProjectRef {
                    project_kind: dependency.target_kind,
                    project_id: dependency.target_id.clone(),
                };
                let kind = project_kind_label(dependency.target_kind);
                let relation = dependency_kind_label(dependency.kind);
                view! {
                    <button
                        type="button"
                        class=if generated { "registry-dependency-row generated" } else if dependency.available { "registry-dependency-row" } else { "registry-dependency-row missing" }
                        disabled=!clickable
                        on:click=move |_| on_open_dependency.run(target.clone())
                    >
                        <span><strong>{dependency.target_id}</strong><em>{kind}</em></span>
                        <small>{if generated { "Generated during compose" } else if dependency.available { relation } else { "Not published" }}</small>
                    </button>
                }
            }).collect_view()}
        </div>
    }
    .into_any()
}

fn detail_tab_class(active: bool) -> &'static str {
    if active {
        "detail-tab active"
    } else {
        "detail-tab"
    }
}

fn project_kind_label(kind: RegistryProjectKind) -> &'static str {
    match kind {
        RegistryProjectKind::Mod => "Mod",
        RegistryProjectKind::Modpack => "Modpack",
    }
}

fn dependency_kind_label(kind: RegistryDependencyKind) -> &'static str {
    match kind {
        RegistryDependencyKind::Init => "Initialization",
        RegistryDependencyKind::Run => "Runtime",
        RegistryDependencyKind::Ownership => "Ownership",
        RegistryDependencyKind::Provides => "Provided API",
        RegistryDependencyKind::Mod => "Mod",
        RegistryDependencyKind::Modpack => "Modpack",
        RegistryDependencyKind::Ignore => "Ignored",
    }
}

fn format_downloads(downloads: Option<i64>) -> String {
    let Some(downloads) = downloads else {
        return "-".to_owned();
    };
    if downloads >= 1_000_000 {
        format!("{:.1}M", downloads as f64 / 1_000_000.0)
    } else if downloads >= 1_000 {
        format!("{:.1}K", downloads as f64 / 1_000.0)
    } else {
        downloads.to_string()
    }
}

fn display_date(value: &str) -> String {
    value.strip_suffix('Z').unwrap_or(value).replace('T', " ")
}

fn display_path(path: &str) -> &str {
    if path.is_empty() || path == "." {
        "Repository root"
    } else {
        path
    }
}

fn short_hash(value: &str) -> String {
    value.chars().take(12).collect()
}
