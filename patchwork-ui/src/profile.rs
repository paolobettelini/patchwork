use leptos::prelude::*;
use patchwork_registry_types::{RegistryProjectKind, RegistryProjectRef, is_generated_mod_id};

use crate::icons::{RefreshCwIcon, UploadIcon, UserIcon};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedProject {
    pub id: String,
    pub project_kind: RegistryProjectKind,
    pub title: String,
    pub kind: String,
    pub downloads: i64,
    pub latest_version: Option<String>,
    pub repository_url: Option<String>,
    pub repository_path: Option<String>,
    pub can_rescan: bool,
}

#[component]
pub fn ProfilePage(
    account_email: String,
    account_name: String,
    mods: Vec<PublishedProject>,
    modpacks: Vec<PublishedProject>,
    on_open_project: Callback<RegistryProjectRef>,
    on_rescan: Callback<RegistryProjectRef>,
    rescan_pending: Signal<Option<String>>,
    children: Children,
) -> impl IntoView {
    let total_downloads = mods
        .iter()
        .chain(modpacks.iter())
        .map(|project| project.downloads)
        .sum::<i64>();
    let total_projects = mods.len() + modpacks.len();
    let github_connection = children();

    view! {
        <div class="profile-page">
            <section class="profile-hero">
                <div class="profile-avatar" aria-hidden="true">
                    <UserIcon />
                </div>

                <div class="profile-identity">
                    <p class="catalog-kicker">"Publisher profile"</p>
                    <h1>{account_name}</h1>
                    <p>{account_email}</p>
                </div>

                <div class="profile-summary">
                    <div>
                        <strong>{total_projects}</strong>
                        <span>"published"</span>
                    </div>
                    <div>
                        <strong>{format_downloads(total_downloads)}</strong>
                        <span>"downloads"</span>
                    </div>
                </div>
            </section>

            {github_connection}

            <section class="profile-published-grid">
                <PublishedSection title="Mods" projects=mods on_open_project on_rescan rescan_pending />
                <PublishedSection title="Modpacks" projects=modpacks on_open_project on_rescan rescan_pending />
            </section>
        </div>
    }
}

#[component]
fn PublishedSection(
    title: &'static str,
    projects: Vec<PublishedProject>,
    on_open_project: Callback<RegistryProjectRef>,
    on_rescan: Callback<RegistryProjectRef>,
    rescan_pending: Signal<Option<String>>,
) -> impl IntoView {
    let projects_count = projects.len();
    let content = if projects.is_empty() {
        view! {
            <div class="published-empty">
                <UploadIcon />
                <span>"Nothing published yet."</span>
            </div>
        }
        .into_any()
    } else {
        let projects = projects.clone();
        view! {
            <div class="published-list">
                <For
                    each=move || projects.clone()
                    key=|project| project.id.clone()
                    children=move |project| {
                        let project_id = project.id.clone();
                        let project_kind = project.project_kind;
                        let generated = project_kind == RegistryProjectKind::Mod
                            && is_generated_mod_id(&project.id);
                        let pending_id = project.id.clone();
                        let open_id = project.id.clone();
                        let version = project.latest_version.clone();
                        let version_view = if let Some(version) = version {
                            view! { <span>{format!("Latest {version}")}</span> }.into_any()
                        } else {
                            view! { <></> }.into_any()
                        };
                        let project_action = if project.can_rescan && !generated {
                            view! {
                                <button
                                    type="button"
                                    class="published-rescan-action"
                                    title="Scan the current default branch"
                                    disabled=move || rescan_pending.get().is_some()
                                    on:click=move |_| on_rescan.run(RegistryProjectRef {
                                        project_kind,
                                        project_id: project_id.clone(),
                                    })
                                >
                                    <RefreshCwIcon />
                                    <span>{move || if rescan_pending.get().as_deref() == Some(pending_id.as_str()) {
                                        "Scanning"
                                    } else {
                                        "Rescan"
                                    }}</span>
                                </button>
                            }
                            .into_any()
                        } else {
                            view! { <></> }.into_any()
                        };
                        view! {
                            <article class="published-item">
                                <div class="published-project-copy">
                                    <button
                                        type="button"
                                        class="published-project-link"
                                        disabled=generated
                                        on:click=move |_| on_open_project.run(RegistryProjectRef {
                                            project_kind,
                                            project_id: open_id.clone(),
                                        })
                                    >
                                        <h3>{project.title}</h3>
                                    </button>
                                    <p>{project.id}</p>
                                    {version_view}
                                </div>
                                <div class="published-item-meta">
                                    <span>{project.kind}</span>
                                    <strong>{format_downloads(project.downloads)}</strong>
                                    {project_action}
                                </div>
                            </article>
                        }
                    }
                />
            </div>
        }
        .into_any()
    };

    view! {
        <section class="published-section">
            <div class="published-section-heading">
                <h2>{title}</h2>
                <span>{projects_count}</span>
            </div>
            {content}
        </section>
    }
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
