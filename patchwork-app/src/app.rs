use crate::{
    home::HomePage,
    icons::{ArrowRightToBracketIcon, GearIcon, HomeIcon, SearchIcon},
    model::{AppTab, LauncherAuthStatus, LauncherModpack, LauncherSettings, RegistryDownloadEvent},
    settings::SettingsPage,
    tauri_bridge::{
        auth_status, disconnect_github, download_profile_dependencies, list_modpacks,
        listen_patchwork_auth, listen_registry_download, load_launcher_settings, logout_auth,
        refresh_auth_profile, refresh_profiles, registry_add_to_profile, registry_browse,
        registry_download_modpack_as_profile, registry_download_status, registry_get_scan,
        registry_project_details, registry_publish_scan, registry_scan_progress,
        registry_start_scan, start_github_connect, start_oauth_login, update_auth_nickname,
    },
};
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use patchwork_registry_types::{
    RegistryAddToProfileRequest, RegistryBrowseProject, RegistryBrowseRequest,
    RegistryProfileOption, RegistryProjectDetails, RegistryProjectKind, RegistryProjectRef,
    RegistryPublishRequest, RegistryScan, RegistryScanPhase, RegistryScanProgress,
    RegistryScanRequest, is_generated_mod_id,
};
use patchwork_ui::{
    BrowsePage, GithubIcon, ProfilePage, PublishedProject as UiPublishedProject,
    RegistryProjectPage, UploadIcon, UploadPage, UserIcon,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;

#[component]
pub(crate) fn App() -> impl IntoView {
    let (active_tab, set_active_tab) = signal(AppTab::Home);
    let (selected_modpack, set_selected_modpack) = signal(0_usize);
    let (active_theme, set_active_theme) = signal("dark");
    let (settings, set_settings) = signal(None::<LauncherSettings>);
    let (modpacks, set_modpacks) = signal(Vec::<LauncherModpack>::new());
    let (auth, set_auth) = signal(None::<LauncherAuthStatus>);
    let (auth_pending, set_auth_pending) = signal(false);
    let (github_pending, set_github_pending) = signal(false);
    let (auth_error, set_auth_error) = signal(None::<String>);
    let (registry_scan, set_registry_scan) = signal(None::<RegistryScan>);
    let (registry_progress, set_registry_progress) = signal(None::<RegistryScanProgress>);
    let (registry_pending, set_registry_pending) = signal(false);
    let (registry_error, set_registry_error) = signal(None::<String>);
    let (registry_notice, set_registry_notice) = signal(None::<String>);
    let (upload_prefill, set_upload_prefill) = signal(None::<RegistryScanRequest>);
    let (browse_results, set_browse_results) = signal(Vec::<RegistryBrowseProject>::new());
    let (browse_pending, set_browse_pending) = signal(false);
    let (browse_action_pending, set_browse_action_pending) = signal(None::<String>);
    let (browse_error, set_browse_error) = signal(None::<String>);
    let (browse_warnings, set_browse_warnings) = signal(Vec::<String>::new());
    let (browse_notice, set_browse_notice) = signal(None::<String>);
    let (project_details, set_project_details) = signal(None::<RegistryProjectDetails>);
    let (project_pending, set_project_pending) = signal(false);
    let (project_error, set_project_error) = signal(None::<String>);
    let (download_event, set_download_event) = signal(None::<RegistryDownloadEvent>);

    let open_registry_project = Callback::new(move |project: RegistryProjectRef| {
        if project.project_kind == RegistryProjectKind::Mod
            && is_generated_mod_id(&project.project_id)
        {
            return;
        }
        set_project_details.set(None);
        set_project_error.set(None);
        set_project_pending.set(true);
        set_active_tab.set(AppTab::Project);
        leptos::task::spawn_local(async move {
            match registry_project_details(project).await {
                Ok(details) => set_project_details.set(Some(details)),
                Err(error) => set_project_error.set(Some(js_error_message(error))),
            }
            set_project_pending.set(false);
        });
    });

    let browse_search = Callback::new(move |input: RegistryBrowseRequest| {
        set_browse_pending.set(true);
        set_browse_error.set(None);
        set_browse_notice.set(None);
        set_browse_warnings.set(Vec::new());
        leptos::task::spawn_local(async move {
            match registry_browse(input).await {
                Ok(response) => {
                    set_browse_results.set(response.projects);
                    set_browse_warnings.set(response.warnings);
                }
                Err(error) => set_browse_error.set(Some(js_error_message(error))),
            }
            set_browse_pending.set(false);
        });
    });
    let browse_download_profile = Callback::new(move |project: RegistryBrowseProject| {
        let action_key = format!(
            "{}:{}",
            project.project_kind.route_segment(),
            project.project_id
        );
        set_browse_action_pending.set(Some(action_key));
        set_browse_error.set(None);
        set_browse_notice.set(None);
        leptos::task::spawn_local(async move {
            match registry_download_modpack_as_profile(project).await {
                Ok(result) => {
                    let created = result.profile;
                    let created_id = created.id.clone();
                    let created_name = created.name.clone();
                    if let Ok(loaded) = list_modpacks().await {
                        let selected = loaded
                            .iter()
                            .position(|profile| profile.id == created_id)
                            .unwrap_or(0);
                        set_modpacks.set(loaded);
                        set_selected_modpack.set(selected);
                    } else {
                        set_modpacks.update(|profiles| profiles.push(created));
                        set_selected_modpack.set(modpacks.get().len().saturating_sub(1));
                    }
                    set_browse_notice.set(Some(format!(
                        "Profile '{created_name}' created. Downloading dependencies..."
                    )));

                    match download_profile_dependencies(&created_id).await {
                        Ok(dependencies) => {
                            let installed = result.report.installed + dependencies.installed;
                            set_browse_notice.set(Some(format!(
                                "Profile '{created_name}' created. {installed} project(s) downloaded."
                            )));
                            if !dependencies.errors.is_empty() {
                                set_browse_warnings
                                    .update(|warnings| warnings.extend(dependencies.errors));
                            }
                        }
                        Err(error) => {
                            set_browse_warnings
                                .update(|warnings| warnings.push(js_error_message(error)));
                        }
                    }
                }
                Err(error) => set_browse_error.set(Some(js_error_message(error))),
            }
            set_browse_action_pending.set(None);
        });
    });
    let browse_add_to_profile = Callback::new(move |input: RegistryAddToProfileRequest| {
        let action_key = format!(
            "{}:{}",
            input.project.project_kind.route_segment(),
            input.project.project_id
        );
        set_browse_action_pending.set(Some(action_key));
        set_browse_error.set(None);
        set_browse_notice.set(None);
        leptos::task::spawn_local(async move {
            match registry_add_to_profile(input).await {
                Ok(result) => {
                    let updated = result.profile;
                    set_browse_notice.set(Some(format!(
                        "Profile '{}' updated. {} project(s) downloaded.",
                        updated.name, result.report.installed
                    )));
                    if !result.report.errors.is_empty() {
                        set_browse_warnings
                            .update(|warnings| warnings.extend(result.report.errors));
                    }
                    if let Ok(loaded) = list_modpacks().await {
                        set_modpacks.set(loaded);
                    }
                }
                Err(error) => set_browse_error.set(Some(js_error_message(error))),
            }
            set_browse_action_pending.set(None);
        });
    });

    let upload_sign_in = Callback::new(move |()| {
        set_auth_error.set(None);
        set_auth_pending.set(true);
        leptos::task::spawn_local(async move {
            match start_oauth_login().await {
                Ok(status) => {
                    let is_complete = status.profile.is_some();
                    set_auth.set(Some(status));
                    if is_complete {
                        set_auth_pending.set(false);
                    } else {
                        poll_auth_until_complete(set_auth, set_auth_pending, set_auth_error).await;
                    }
                }
                Err(error) => {
                    set_auth_error.set(Some(js_error_message(error)));
                    set_auth_pending.set(false);
                }
            }
        });
    });
    let upload_connect_github = Callback::new(move |()| {
        set_auth_error.set(None);
        set_github_pending.set(true);
        leptos::task::spawn_local(async move {
            match start_github_connect().await {
                Ok(status) => {
                    set_auth.set(Some(status));
                    poll_github_until_complete(
                        set_auth,
                        github_pending,
                        set_github_pending,
                        set_auth_error,
                    )
                    .await;
                }
                Err(error) => {
                    set_auth_error.set(Some(js_error_message(error)));
                    set_github_pending.set(false);
                }
            }
        });
    });
    let upload_scan = Callback::new(move |input: RegistryScanRequest| {
        set_registry_error.set(None);
        set_registry_notice.set(None);
        set_registry_scan.set(None);
        set_registry_progress.set(None);
        set_registry_pending.set(true);
        leptos::task::spawn_local(async move {
            let result = async {
                let started = registry_start_scan(input).await.map_err(js_error_message)?;
                poll_registry_scan(&started.job_id, set_registry_progress).await
            }
            .await;
            match result {
                Ok(scan) => set_registry_scan.set(Some(scan)),
                Err(error) => set_registry_error.set(Some(error)),
            }
            set_registry_pending.set(false);
        });
    });
    let upload_publish = Callback::new(move |input: RegistryPublishRequest| {
        let Some(scan_id) = registry_scan.get_untracked().map(|scan| scan.scan_id) else {
            set_registry_error.set(Some("Run a scan before publishing.".to_owned()));
            return;
        };
        set_registry_error.set(None);
        set_registry_notice.set(None);
        set_registry_pending.set(true);
        leptos::task::spawn_local(async move {
            match registry_publish_scan(&scan_id, input).await {
                Ok(published) => {
                    set_registry_notice.set(Some(format!(
                        "Published {} project version{}.",
                        published.published.len(),
                        if published.published.len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    )));
                    if let Ok(scan) = registry_get_scan(&scan_id).await {
                        set_registry_scan.set(Some(scan));
                    }
                    if let Ok(status) = refresh_auth_profile().await {
                        set_auth.set(Some(status));
                    }
                }
                Err(error) => set_registry_error.set(Some(js_error_message(error))),
            }
            set_registry_pending.set(false);
        });
    });
    let profile_rescan = Callback::new(move |project: UiPublishedProject| {
        set_registry_error.set(None);
        set_registry_progress.set(None);
        set_registry_scan.set(None);
        set_registry_notice.set(None);
        set_registry_pending.set(false);
        set_active_tab.set(AppTab::Upload);
        match project.rescan_request() {
            Some(request) => {
                set_upload_prefill.set(None);
                set_upload_prefill.set(Some(request));
            }
            None => set_registry_error.set(Some(
                "This project does not contain repository coordinates for a rescan.".to_owned(),
            )),
        }
    });

    let _ = listen_patchwork_auth(move |event| {
        set_auth.set(Some(event.status.clone()));
        set_auth_error.set(event.error);
        set_auth_pending.set(false);
        set_github_pending.set(false);
        if event.status.profile.is_some() {
            set_active_tab.set(AppTab::Profile);
        }
    });

    let _ = listen_registry_download(move |event| {
        set_download_event.set(Some(event));
    });

    leptos::task::spawn_local(async move {
        loop {
            if let Ok(status) = registry_download_status().await {
                set_download_event.set(status);
            }
            TimeoutFuture::new(120).await;
        }
    });

    leptos::task::spawn_local(async move {
        if let Ok(loaded_settings) = load_launcher_settings().await {
            set_active_theme.set(theme_id_or_default(&loaded_settings.theme));
            set_settings.set(Some(loaded_settings));
        }

        browse_search.run(RegistryBrowseRequest::default());

        if let Ok(loaded_modpacks) = list_modpacks().await {
            set_selected_modpack.set(0);
            set_modpacks.set(loaded_modpacks);
        }

        if let Ok(refreshed_modpacks) = refresh_profiles().await {
            set_modpacks.set(refreshed_modpacks);
        }

        if let Ok(status) = auth_status().await {
            let should_refresh = status.profile.is_some();
            set_auth.set(Some(status));

            if should_refresh {
                match refresh_auth_profile().await {
                    Ok(status) => {
                        set_auth.set(Some(status));
                        set_auth_error.set(None);
                    }
                    Err(error) => {
                        set_auth_error.set(Some(format!(
                            "Could not refresh your profile: {}",
                            js_error_message(error)
                        )));
                    }
                }
            }
        }
    });

    view! {
        <div class="app-shell" data-theme=move || active_theme.get()>
            <TopBar
                active_tab
                set_active_tab
                auth
                set_auth
                auth_pending
                set_auth_pending
                auth_error
                set_auth_error
                download_event
            />

            <div class="workspace">
                <section class=move || page_class(active_tab.get() == AppTab::Home)>
                    <HomePage
                        modpacks
                        set_modpacks
                        selected_modpack
                        set_selected_modpack
                    />
                </section>

                <section class=move || page_class(active_tab.get() == AppTab::Browse)>
                    <BrowsePage
                        results=Signal::from(browse_results)
                        profiles=Signal::derive(move || modpacks.get().into_iter().map(|profile| {
                            RegistryProfileOption { id: profile.id, name: profile.name }
                        }).collect())
                        pending=Signal::from(browse_pending)
                        action_pending=Signal::from(browse_action_pending)
                        error=Signal::from(browse_error)
                        warnings=Signal::from(browse_warnings)
                        notice=Signal::from(browse_notice)
                        allow_downloads=true
                        on_search=browse_search
                        on_open_project=open_registry_project
                        on_download_profile=browse_download_profile
                        on_add_to_profile=browse_add_to_profile
                    />
                </section>

                <section class=move || page_class(active_tab.get() == AppTab::Upload)>
                    <UploadPage
                        authenticated=Signal::derive(move || auth.get().and_then(|status| status.profile).is_some())
                        github_connected=Signal::derive(move || auth.get().and_then(|status| status.profile).and_then(|profile| profile.github).is_some())
                        scan=Signal::from(registry_scan)
                        progress=Signal::from(registry_progress)
                        pending=Signal::from(registry_pending)
                        error=Signal::from(registry_error)
                        notice=Signal::from(registry_notice)
                        prefill=Signal::from(upload_prefill)
                        on_sign_in=upload_sign_in
                        on_connect_github=upload_connect_github
                        on_scan=upload_scan
                        on_publish=upload_publish
                        on_open_project=open_registry_project
                    />
                </section>

                <section class=move || page_class(active_tab.get() == AppTab::Project)>
                    <RegistryProjectPage
                        details=Signal::from(project_details)
                        pending=Signal::from(project_pending)
                        error=Signal::from(project_error)
                        on_open_dependency=open_registry_project
                    />
                </section>

                <section class=move || page_class(active_tab.get() == AppTab::Profile)>
                    <AppProfilePage
                        auth
                        set_auth
                        set_auth_error
                        github_pending
                        set_github_pending
                        on_open_project=open_registry_project
                        on_rescan=profile_rescan
                        registry_error=Signal::from(registry_error)
                    />
                </section>

                <section class=move || page_class(active_tab.get() == AppTab::Settings)>
                    <SettingsPage
                        active_theme
                        set_active_theme
                        settings
                        set_settings
                        set_modpacks
                        set_selected_modpack
                    />
                </section>
            </div>
        </div>
    }
}

#[component]
fn TopBar(
    active_tab: ReadSignal<AppTab>,
    set_active_tab: WriteSignal<AppTab>,
    auth: ReadSignal<Option<LauncherAuthStatus>>,
    set_auth: WriteSignal<Option<LauncherAuthStatus>>,
    auth_pending: ReadSignal<bool>,
    set_auth_pending: WriteSignal<bool>,
    auth_error: ReadSignal<Option<String>>,
    set_auth_error: WriteSignal<Option<String>>,
    download_event: ReadSignal<Option<RegistryDownloadEvent>>,
) -> impl IntoView {
    let start_login = move |_| {
        set_auth_error.set(None);
        set_auth_pending.set(true);
        leptos::task::spawn_local(async move {
            match start_oauth_login().await {
                Ok(status) => {
                    let is_complete = status.profile.is_some();
                    set_auth.set(Some(status));
                    if is_complete {
                        set_auth_pending.set(false);
                    } else {
                        poll_auth_until_complete(set_auth, set_auth_pending, set_auth_error).await;
                    }
                }
                Err(error) => {
                    set_auth_error.set(Some(js_error_message(error)));
                    set_auth_pending.set(false);
                }
            }
        });
    };

    view! {
        <header class="topbar">
            <div class="brand">
                <img class="brand-logo" src="/logo.png" alt="Patchwork" />
                <div class="brand-copy">
                    <strong>"Patchwork"</strong>
                </div>
            </div>

            <span class="topbar-divider" aria-hidden="true"></span>

            <nav class="top-tabs" aria-label="Main tabs">
                <button
                    type="button"
                    class=move || top_tab_class(active_tab.get() == AppTab::Home)
                    on:click=move |_| set_active_tab.set(AppTab::Home)
                >
                    <HomeIcon />
                    <span>"Home"</span>
                </button>

                <button
                    type="button"
                    class=move || top_tab_class(active_tab.get() == AppTab::Browse)
                    on:click=move |_| set_active_tab.set(AppTab::Browse)
                >
                    <SearchIcon />
                    <span>"Browse"</span>
                </button>

                <button
                    type="button"
                    class=move || top_tab_class(active_tab.get() == AppTab::Upload)
                    on:click=move |_| set_active_tab.set(AppTab::Upload)
                >
                    <UploadIcon />
                    <span>"Upload"</span>
                </button>

                <button
                    type="button"
                    class=move || top_tab_class(active_tab.get() == AppTab::Settings)
                    on:click=move |_| set_active_tab.set(AppTab::Settings)
                >
                    <GearIcon />
                    <span>"Settings"</span>
                </button>
            </nav>

            <Show when=move || download_event.get().is_some()>
                {move || {
                    download_event.get().map(|event| {
                        let failed = !event.errors.is_empty();
                        let progress = if event.total == 0 {
                            4.0
                        } else {
                            ((event.completed as f64 / event.total as f64) * 100.0).clamp(4.0, 100.0)
                        };
                        let progress_count = format!(
                            "[{}/{}]",
                            event.completed.min(event.total),
                            event.total
                        );
                        let count = event
                            .current
                            .as_deref()
                            .map(|id| format!("{progress_count} {id}"))
                            .unwrap_or(progress_count);
                        let title = event.errors.first().cloned().unwrap_or_default();
                        view! {
                            <div
                                class=if failed { "topbar-download error" } else { "topbar-download" }
                                role="status"
                                aria-live="polite"
                                title=title
                            >
                                <div class="topbar-download-track" aria-hidden="true">
                                    <span
                                        class:indeterminate=event.total == 0
                                        style:width=format!("{progress:.2}%")
                                    ></span>
                                </div>
                                <span class="topbar-download-count">{count}</span>
                            </div>
                        }
                    })
                }}
            </Show>

            <div class="topbar-actions">
                <Show
                    when=move || auth.get().and_then(|status| status.profile).is_some()
                    fallback=move || view! {
                        <button type="button" class="sign-in-button" on:click=start_login disabled=move || auth_pending.get()>
                            <span>{move || if auth_pending.get() { "Signing in" } else { "Sign in / Sign up" }}</span>
                            <ArrowRightToBracketIcon />
                        </button>
                    }
                >
                    {move || {
                        auth.get()
                            .and_then(|status| status.profile)
                            .map(|profile| view! {
                                <button
                                    type="button"
                                    class=move || top_tab_class(active_tab.get() == AppTab::Profile)
                                    on:click=move |_| set_active_tab.set(AppTab::Profile)
                                >
                                    <UserIcon />
                                    <span>{profile.account.nickname}</span>
                                </button>
                            })
                    }}
                </Show>
                <Show when=move || auth_error.get().is_some()>
                    <span class="auth-inline-error">{move || auth_error.get().unwrap_or_default()}</span>
                </Show>
            </div>
        </header>
    }
}

#[component]
fn AppProfilePage(
    auth: ReadSignal<Option<LauncherAuthStatus>>,
    set_auth: WriteSignal<Option<LauncherAuthStatus>>,
    set_auth_error: WriteSignal<Option<String>>,
    github_pending: ReadSignal<bool>,
    set_github_pending: WriteSignal<bool>,
    on_open_project: Callback<RegistryProjectRef>,
    on_rescan: Callback<UiPublishedProject>,
    registry_error: Signal<Option<String>>,
) -> impl IntoView {
    let (editing_nickname, set_editing_nickname) = signal(false);
    let (nickname_draft, set_nickname_draft) = signal(String::new());
    let (nickname_error, set_nickname_error) = signal(None::<String>);

    let sign_out = move |_| {
        set_auth_error.set(None);
        leptos::task::spawn_local(async move {
            match logout_auth().await {
                Ok(status) => set_auth.set(Some(status)),
                Err(error) => set_auth_error.set(Some(js_error_message(error))),
            }
        });
    };

    view! {
        {move || {
            auth.get()
                .and_then(|status| status.profile)
                .map(|profile| view! {
                    <div class="profile-page-shell">
                        <ProfilePage
                            account_email=profile.account.email.clone()
                            account_name=profile.account.nickname.clone()
                            mods=ui_projects(&profile.mods)
                            modpacks=ui_projects(&profile.modpacks)
                            on_open_project
                            on_rescan
                        >
                            <AppGithubConnection
                                github=profile.github.clone()
                                set_auth
                                set_auth_error
                                github_pending
                                set_github_pending
                            />
                            <div class="profile-local-actions">
                                <Show
                                    when=move || editing_nickname.get()
                                    fallback=move || view! {
                                        <button
                                            type="button"
                                            class="catalog-secondary-action"
                                            on:click=move |_| {
                                                if let Some(status) = auth.get() {
                                                    if let Some(profile) = status.profile {
                                                        set_nickname_draft.set(profile.account.nickname);
                                                    }
                                                }
                                                set_nickname_error.set(None);
                                                set_editing_nickname.set(true);
                                            }
                                        >
                                            "Change username"
                                        </button>
                                    }
                                >
                                    <div class="nickname-editor">
                                        <input
                                            maxlength="16"
                                            prop:value=move || nickname_draft.get()
                                            on:input=move |event| set_nickname_draft.set(event_target_value(&event))
                                        />
                                        <button
                                            type="button"
                                            class="catalog-primary-action"
                                            on:click=move |_| {
                                                let nickname = nickname_draft.get();
                                                set_nickname_error.set(None);
                                                leptos::task::spawn_local(async move {
                                                    match update_auth_nickname(&nickname).await {
                                                        Ok(status) => {
                                                            set_auth.set(Some(status));
                                                            set_editing_nickname.set(false);
                                                        }
                                                        Err(error) => set_nickname_error.set(Some(js_error_message(error))),
                                                    }
                                                });
                                            }
                                        >
                                            "Save"
                                        </button>
                                        <button
                                            type="button"
                                            class="catalog-secondary-action"
                                            on:click=move |_| set_editing_nickname.set(false)
                                        >
                                            "Cancel"
                                        </button>
                                    </div>
                                </Show>
                                <button type="button" class="catalog-secondary-action" on:click=sign_out>
                                    "Sign out"
                                </button>
                            </div>
                            <Show when=move || nickname_error.get().is_some()>
                                <p class="auth-inline-error">{move || nickname_error.get().unwrap_or_default()}</p>
                            </Show>
                        </ProfilePage>
                        <Show when=move || registry_error.get().is_some()>
                            <p class="auth-inline-error">{move || registry_error.get().unwrap_or_default()}</p>
                        </Show>
                    </div>
                }.into_any())
                .unwrap_or_else(|| view! {
                    <section class="signed-out-profile">
                        <UserIcon />
                        <h1>"Publisher profile"</h1>
                        <p>"Sign in to see your published mods and modpacks."</p>
                    </section>
                }.into_any())
        }}
    }
}

#[component]
fn AppGithubConnection(
    github: Option<crate::model::GithubAccount>,
    set_auth: WriteSignal<Option<LauncherAuthStatus>>,
    set_auth_error: WriteSignal<Option<String>>,
    github_pending: ReadSignal<bool>,
    set_github_pending: WriteSignal<bool>,
) -> impl IntoView {
    let content = if let Some(github) = github {
        view! {
            <div class="github-account-row">
                <img
                    src=github.github_avatar_url
                    alt="GitHub avatar"
                    loading="lazy"
                    referrerpolicy="no-referrer"
                />
                <div>
                    <strong>{format!("@{}", github.github_login)}</strong>
                    <span>{format!("GitHub user ID {}", github.github_user_id)}</span>
                </div>
                <button
                    type="button"
                    class="danger-secondary-action"
                    disabled=move || github_pending.get()
                    on:click=move |_| {
                        set_auth_error.set(None);
                        set_github_pending.set(true);
                        leptos::task::spawn_local(async move {
                            match disconnect_github().await {
                                Ok(status) => set_auth.set(Some(status)),
                                Err(error) => set_auth_error.set(Some(js_error_message(error))),
                            }
                            set_github_pending.set(false);
                        });
                    }
                >
                    {move || if github_pending.get() { "Disconnecting" } else { "Disconnect GitHub" }}
                </button>
            </div>
        }
        .into_any()
    } else {
        view! {
            <div class="github-connect-empty">
                <p>"Connect the GitHub identity you will use to publish projects."</p>
                <button
                    type="button"
                    class="catalog-primary-action"
                    disabled=move || github_pending.get()
                    on:click=move |_| {
                        set_auth_error.set(None);
                        set_github_pending.set(true);
                        leptos::task::spawn_local(async move {
                            match start_github_connect().await {
                                Ok(status) => {
                                    set_auth.set(Some(status));
                                    poll_github_until_complete(
                                        set_auth,
                                        github_pending,
                                        set_github_pending,
                                        set_auth_error,
                                    )
                                    .await;
                                }
                                Err(error) => {
                                    set_auth_error.set(Some(js_error_message(error)));
                                    set_github_pending.set(false);
                                }
                            }
                        });
                    }
                >
                    <GithubIcon />
                    <span>{move || if github_pending.get() { "Waiting for GitHub" } else { "Connect GitHub" }}</span>
                </button>
            </div>
        }
        .into_any()
    };

    view! {
        <section class="github-connection">
            <div class="github-connection-heading">
                <div>
                    <p class="catalog-kicker">"Connected account"</p>
                    <h2>"GitHub"</h2>
                </div>
                <GithubIcon />
            </div>
            {content}
        </section>
    }
}

fn page_class(is_active: bool) -> &'static str {
    if is_active { "page active" } else { "page" }
}

fn top_tab_class(is_active: bool) -> &'static str {
    if is_active {
        "top-tab active"
    } else {
        "top-tab"
    }
}

fn theme_id_or_default(theme: &str) -> &'static str {
    match theme {
        "dim-white" => "dim-white",
        "aurora" => "aurora",
        "volcanic" => "volcanic",
        "nebula" => "nebula",
        "moss" => "moss",
        "bubblegum" => "bubblegum",
        "terminal" => "terminal",
        _ => "dark",
    }
}

fn ui_projects(projects: &[crate::model::PublishedProject]) -> Vec<UiPublishedProject> {
    projects
        .iter()
        .map(|project| UiPublishedProject {
            project_kind: if project.kind.eq_ignore_ascii_case("modpack") {
                RegistryProjectKind::Modpack
            } else {
                RegistryProjectKind::Mod
            },
            id: project.id.clone(),
            title: project.title.clone(),
            kind: project.kind.clone(),
            downloads: project.downloads,
            latest_version: project.latest_version.clone(),
            repository_url: project.repository_url.clone(),
            repository_path: project.repository_path.clone(),
            can_rescan: project.can_rescan,
        })
        .collect()
}

fn js_error_message(error: wasm_bindgen::JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "Patchwork command failed".to_string())
}

async fn poll_auth_until_complete(
    set_auth: WriteSignal<Option<LauncherAuthStatus>>,
    set_auth_pending: WriteSignal<bool>,
    set_auth_error: WriteSignal<Option<String>>,
) {
    for _ in 0..300 {
        sleep_ms(1_000).await;
        if let Ok(status) = auth_status().await {
            let is_complete = status.profile.is_some();
            set_auth.set(Some(status));
            if is_complete {
                set_auth_pending.set(false);
                set_auth_error.set(None);
                return;
            }
        }
    }
    set_auth_pending.set(false);
    set_auth_error.set(Some("Login did not finish in time.".to_string()));
}

async fn poll_github_until_complete(
    set_auth: WriteSignal<Option<LauncherAuthStatus>>,
    github_pending: ReadSignal<bool>,
    set_github_pending: WriteSignal<bool>,
    set_auth_error: WriteSignal<Option<String>>,
) {
    for _ in 0..600 {
        sleep_ms(500).await;
        if !github_pending.get_untracked() {
            return;
        }

        if let Ok(status) = auth_status().await {
            let connected = status
                .profile
                .as_ref()
                .and_then(|profile| profile.github.as_ref())
                .is_some();
            set_auth.set(Some(status));
            if connected {
                set_github_pending.set(false);
                set_auth_error.set(None);
                return;
            }
        }
    }

    set_github_pending.set(false);
    set_auth_error.set(Some(
        "GitHub connection did not finish in time.".to_string(),
    ));
}

async fn poll_registry_scan(
    job_id: &str,
    set_progress: WriteSignal<Option<RegistryScanProgress>>,
) -> Result<RegistryScan, String> {
    for _ in 0..2_400 {
        let progress = registry_scan_progress(job_id)
            .await
            .map_err(js_error_message)?;
        let phase = progress.phase;
        let result = match phase {
            RegistryScanPhase::Complete => progress
                .scan
                .clone()
                .ok_or_else(|| "repository scan completed without a persisted preview".to_owned()),
            RegistryScanPhase::Failed => Err(progress
                .error
                .clone()
                .unwrap_or_else(|| "repository scan failed".to_owned())),
            _ => {
                set_progress.set(Some(progress));
                sleep_ms(250).await;
                continue;
            }
        };
        set_progress.set(Some(progress));
        return result;
    }
    Err("repository scan did not finish in time".to_owned())
}

async fn sleep_ms(milliseconds: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _reject| {
        let callback = Closure::once_into_js(move || {
            let _ = resolve.call0(&JsValue::NULL);
        });
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                milliseconds,
            );
        }
    });
    let _ = JsFuture::from(promise).await;
}
