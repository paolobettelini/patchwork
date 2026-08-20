use std::fmt::Write as _;

use crate::deptree::DependencyTreePage;

use crate::auth_types::{
    GithubAccountDto, LoginRequest, ProfileDto, PublicProfileDto, PublishedProjectDto,
    RegisterRequest, RegistrationChallengeDto, UpdateNicknameRequest, VerifyRegistrationRequest,
};
use gloo_net::http::Request;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use patchwork_registry_types::{
    RegistryAddToProfileRequest, RegistryBrowseProject, RegistryBrowseRequest,
    RegistryBrowseResponse, RegistryDependencyGraph, RegistryProfileOption, RegistryProjectDetails,
    RegistryProjectKind,
    RegistryProjectRef, RegistryPublishRequest, RegistryPublishResponse, RegistryScan,
    RegistryScanJobStarted, RegistryScanPhase, RegistryScanProgress, RegistryScanRequest,
    is_generated_mod_id,
};
use patchwork_ui::{
    ArrowRightToBracketIcon, BrowsePage, GithubIcon, HomeIcon, ProfilePage, PublishedProject,
    RegistryProjectPage, SearchIcon, THEMES, UploadIcon, UploadPage, UserIcon,
};
use sha2::{Digest, Sha256};

const GITHUB_URL: &str = "https://github.com/paolobettelini/patchwork";
const THEME_STORAGE_KEY: &str = "patchwork-theme";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WebPage {
    Home,
    Browse,
    Upload,
    Profile,
    Project,
    DependencyTree,
    NotFound,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuthMode {
    Login,
    Register,
}

enum AuthSubmission {
    Authenticated(ProfileDto),
    VerificationRequired(RegistrationChallengeDto),
}

#[derive(Clone)]
struct PasswordRequirement {
    label: &'static str,
    met: bool,
}

#[component]
pub(crate) fn WebApp() -> impl IntoView {
    let page = current_page();
    let (active_theme, set_active_theme) = signal(load_stored_theme());
    let (profile, set_profile) = signal(None::<ProfileDto>);
    let (show_auth, set_show_auth) = signal(false);
    let (registry_scan, set_registry_scan) = signal(None::<RegistryScan>);
    let (registry_progress, set_registry_progress) = signal(None::<RegistryScanProgress>);
    let (registry_pending, set_registry_pending) = signal(false);
    let (registry_error, set_registry_error) = signal(None::<String>);
    let (registry_notice, set_registry_notice) = signal(None::<String>);
    let (upload_prefill, _) = signal(upload_prefill_from_query());
    let (browse_results, set_browse_results) = signal(Vec::<RegistryBrowseProject>::new());
    let (browse_pending, set_browse_pending) = signal(false);
    let (browse_error, set_browse_error) = signal(None::<String>);
    let (browse_warnings, set_browse_warnings) = signal(Vec::<String>::new());
    let (project_details, set_project_details) = signal(None::<RegistryProjectDetails>);
    let (project_pending, set_project_pending) = signal(false);
    let (project_error, set_project_error) = signal(None::<String>);
    let (dependency_graph, set_dependency_graph) = signal(None::<RegistryDependencyGraph>);
    let (dependency_graph_pending, set_dependency_graph_pending) = signal(false);
    let (dependency_graph_error, set_dependency_graph_error) = signal(None::<String>);
    let (viewed_profile, set_viewed_profile) = signal(None::<PublicProfileDto>);
    let (viewed_profile_pending, set_viewed_profile_pending) = signal(false);
    let (viewed_profile_error, set_viewed_profile_error) = signal(None::<String>);

    let open_project = Callback::new(move |project: RegistryProjectRef| {
        if project.project_kind == RegistryProjectKind::Mod
            && is_generated_mod_id(&project.project_id)
        {
            return;
        }
        let preview = registry_scan.get_untracked().and_then(|scan| {
            scan.entries
                .iter()
                .find(|entry| {
                    entry.project_kind == project.project_kind
                        && entry.project_id == project.project_id
                })
                .map(|entry| (scan.scan_id.clone(), entry.entry_id.clone()))
        });
        let mut path = project_path(&project);
        if let Some((scan_id, entry_id)) = preview {
            path.push_str(&format!("?scan={scan_id}&entry={entry_id}"));
        }
        navigate_to(&path);
    });

    let open_dependency_tree = Callback::new(move |project: RegistryProjectRef| {
        navigate_to(&dependency_tree_path(&project));
    });

    let browse_search = Callback::new(move |input: RegistryBrowseRequest| {
        set_browse_pending.set(true);
        set_browse_error.set(None);
        set_browse_warnings.set(Vec::new());
        leptos::task::spawn_local(async move {
            match fetch_registry_browse(&input).await {
                Ok(response) => {
                    set_browse_results.set(response.projects);
                    set_browse_warnings.set(response.warnings);
                }
                Err(error) => set_browse_error.set(Some(error)),
            }
            set_browse_pending.set(false);
        });
    });

    let open_publisher = Callback::new(move |nickname: String| {
        navigate_to(&profile_path(&nickname));
    });

    let upload_sign_in = Callback::new(move |()| set_show_auth.set(true));
    let upload_connect_github = Callback::new(move |()| navigate_to("/github/connect"));
    let upload_scan = Callback::new(move |input: RegistryScanRequest| {
        set_registry_error.set(None);
        set_registry_notice.set(None);
        set_registry_scan.set(None);
        set_registry_progress.set(None);
        set_registry_pending.set(true);
        leptos::task::spawn_local(async move {
            let result = async {
                let started = start_registry_scan(input).await?;
                poll_registry_scan(&started.job_id, set_registry_progress).await
            }
            .await;
            match result {
                Ok(scan) => set_registry_scan.set(Some(scan)),
                Err(message) => set_registry_error.set(Some(message)),
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
            match publish_registry_scan(&scan_id, input).await {
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
                    if let Ok(scan) = fetch_registry_scan(&scan_id).await {
                        set_registry_scan.set(Some(scan));
                    }
                    if let Ok(current_profile) = fetch_profile().await {
                        set_profile.set(Some(current_profile));
                    }
                }
                Err(message) => set_registry_error.set(Some(message)),
            }
            set_registry_pending.set(false);
        });
    });
    let profile_rescan = Callback::new(move |project: PublishedProject| {
        set_registry_error.set(None);
        set_registry_progress.set(None);
        set_registry_scan.set(None);
        match project.rescan_request() {
            Some(request) => navigate_to(&upload_path(&request)),
            None => set_registry_error.set(Some(
                "This project does not contain repository coordinates for a rescan.".to_owned(),
            )),
        }
    });

    let profile_without_username = page == WebPage::Profile && current_profile_nickname().is_none();
    leptos::task::spawn_local(async move {
        if let Ok(current_profile) = fetch_profile().await {
            if profile_without_username {
                navigate_to(&profile_path(&current_profile.account.nickname));
            }
            set_profile.set(Some(current_profile));
        }
    });

    if page == WebPage::Profile {
        if let Some(nickname) = current_profile_nickname() {
            set_viewed_profile_pending.set(true);
            leptos::task::spawn_local(async move {
                match fetch_public_profile(&nickname).await {
                    Ok(public_profile) => set_viewed_profile.set(Some(public_profile)),
                    Err(message) => set_viewed_profile_error.set(Some(message)),
                }
                set_viewed_profile_pending.set(false);
            });
        }
    }

    if page == WebPage::Browse {
        browse_search.run(RegistryBrowseRequest::default());
    }

    if page == WebPage::Upload {
        if let Some(scan_id) = query_parameter("scan") {
            set_registry_pending.set(true);
            leptos::task::spawn_local(async move {
                match fetch_registry_scan(&scan_id).await {
                    Ok(scan) => set_registry_scan.set(Some(scan)),
                    Err(message) => set_registry_error.set(Some(message)),
                }
                set_registry_pending.set(false);
            });
        } else if let Some(job_id) = query_parameter("job") {
            set_registry_pending.set(true);
            leptos::task::spawn_local(async move {
                match poll_registry_scan(&job_id, set_registry_progress).await {
                    Ok(scan) => set_registry_scan.set(Some(scan)),
                    Err(message) => set_registry_error.set(Some(message)),
                }
                set_registry_pending.set(false);
            });
        }
    }

    if let Some(project) = current_project_ref() {
        set_project_pending.set(true);
        leptos::task::spawn_local(async move {
            let result = match (query_parameter("scan"), query_parameter("entry")) {
                (Some(scan_id), Some(entry_id)) => {
                    fetch_scan_project_details(&scan_id, &entry_id).await
                }
                _ => fetch_registry_project(&project).await,
            };
            match result {
                Ok(details) => set_project_details.set(Some(details)),
                Err(message) => set_project_error.set(Some(message)),
            }
            set_project_pending.set(false);
        });
    }

    if let Some(project) = current_dependency_tree_project_ref() {
        set_dependency_graph_pending.set(true);
        leptos::task::spawn_local(async move {
            match fetch_dependency_graph(&project).await {
                Ok(graph) => set_dependency_graph.set(Some(graph)),
                Err(message) => set_dependency_graph_error.set(Some(message)),
            }
            set_dependency_graph_pending.set(false);
        });
    }

    view! {
        <div class="app-shell" data-theme=move || active_theme.get()>
            <TopBar active_page=page profile set_profile set_show_auth active_theme set_active_theme />
            <main class=if page == WebPage::DependencyTree { "web-workspace deptree-workspace" } else { "web-workspace" }>
                {match page {
                    WebPage::Home => view! { <HomePage /> }.into_any(),
                    WebPage::Browse => view! {
                        <BrowsePage
                            results=Signal::from(browse_results)
                            profiles=Signal::derive(Vec::<RegistryProfileOption>::new)
                            pending=Signal::from(browse_pending)
                            action_pending=Signal::derive(|| None::<String>)
                            error=Signal::from(browse_error)
                            warnings=Signal::from(browse_warnings)
                            notice=Signal::derive(|| None::<String>)
                            allow_downloads=false
                            on_search=browse_search
                            on_open_project=open_project
                            on_download_profile=Callback::new(|_: RegistryBrowseProject| {})
                            on_add_to_profile=Callback::new(|_: RegistryAddToProfileRequest| {})
                        />
                    }.into_any(),
                    WebPage::Upload => view! {
                        <UploadPage
                            authenticated=Signal::derive(move || profile.get().is_some())
                            github_connected=Signal::derive(move || profile.get().is_some_and(|profile| profile.github.is_some()))
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
                            on_open_project=open_project
                        />
                    }.into_any(),
                    WebPage::Profile => view! {
                        <WebProfilePage
                            profile
                            set_profile
                            viewed_profile
                            viewed_profile_pending
                            viewed_profile_error
                            set_show_auth
                            on_open_project=open_project
                            on_rescan=profile_rescan
                            registry_error=Signal::from(registry_error)
                        />
                    }.into_any(),
                    WebPage::Project => view! {
                        <RegistryProjectPage
                            details=Signal::from(project_details)
                            pending=Signal::from(project_pending)
                            error=Signal::from(project_error)
                            on_open_dependency=open_project
                            on_open_dependency_tree=Some(open_dependency_tree)
                            on_open_publisher=open_publisher
                        />
                    }.into_any(),
                    WebPage::DependencyTree => view! {
                        <DependencyTreePage
                            graph=Signal::from(dependency_graph)
                            pending=Signal::from(dependency_graph_pending)
                            error=Signal::from(dependency_graph_error)
                            on_open_project=open_project
                        />
                    }.into_any(),
                    WebPage::NotFound => view! { <NotFoundPage /> }.into_any(),
                }}
            </main>

            <Show when=move || show_auth.get()>
                <AuthDialog set_profile set_show_auth />
            </Show>
        </div>
    }
}

#[component]
fn TopBar(
    active_page: WebPage,
    profile: ReadSignal<Option<ProfileDto>>,
    set_profile: WriteSignal<Option<ProfileDto>>,
    set_show_auth: WriteSignal<bool>,
    active_theme: ReadSignal<String>,
    set_active_theme: WriteSignal<String>,
) -> impl IntoView {
    let (show_profile_menu, set_show_profile_menu) = signal(false);
    let change_theme = move |event| {
        let theme = theme_id_or_default(&event_target_value(&event));
        store_theme(theme);
        set_active_theme.set(theme.to_string());
    };

    view! {
        <header class="topbar">
            <a class="brand" href=app_url("/") aria-label="Patchwork home">
                <img class="brand-logo" src=app_url("/logo.png") alt="Patchwork" />
                <div class="brand-copy">
                    <strong>"Patchwork"</strong>
                </div>
            </a>

            <span class="topbar-divider" aria-hidden="true"></span>

            <nav class="top-tabs" aria-label="Main navigation">
                <a class=top_tab_class(active_page == WebPage::Home) href=app_url("/")>
                    <HomeIcon />
                    <span>"Home"</span>
                </a>
                <a class=top_tab_class(active_page == WebPage::Browse) href=app_url("/browse")>
                    <SearchIcon />
                    <span>"Browse"</span>
                </a>
                <a class=top_tab_class(active_page == WebPage::Upload) href=app_url("/upload")>
                    <UploadIcon />
                    <span>"Upload"</span>
                </a>
            </nav>

            <div class="topbar-actions">
                <select class="theme-select" prop:value=move || active_theme.get() on:change=change_theme aria-label="Theme">
                    <For
                        each=move || THEMES
                        key=|theme| theme.0
                        children=move |(theme_id, theme_name): (&'static str, &'static str)| view! {
                            <option value=theme_id>{theme_name}</option>
                        }
                    />
                </select>
                <a class="github-link" href=GITHUB_URL target="_blank" rel="noreferrer" aria-label="GitHub">
                    <GithubIcon />
                </a>
                <Show
                    when=move || profile.get().is_some()
                    fallback=move || view! {
                        <button type="button" class="sign-in-button" on:click=move |_| set_show_auth.set(true)>
                            <span>"Sign in"</span>
                            <ArrowRightToBracketIcon />
                        </button>
                    }
                >
                    {move || {
                        profile
                            .get()
                            .map(|profile| {
                                let internal_profile_path = profile_path(&profile.account.nickname);
                                let own_profile_is_active = active_page == WebPage::Profile
                                    && current_path() == internal_profile_path;
                                let profile_href = app_url(&internal_profile_path);
                                view! {
                                    <div class="profile-menu">
                                        <button
                                            type="button"
                                            class=profile_button_class(own_profile_is_active)
                                            on:click=move |_| set_show_profile_menu.update(|show| *show = !*show)
                                        >
                                            <UserIcon />
                                            <span>{profile.account.nickname}</span>
                                        </button>
                                        <Show when=move || show_profile_menu.get()>
                                            <div class="profile-dropdown">
                                                <a href=profile_href.clone() on:click=move |_| set_show_profile_menu.set(false)>
                                                    <UserIcon />
                                                    <span>"Profile"</span>
                                                </a>
                                                <button
                                                    type="button"
                                                    class="danger-menu-action"
                                                    on:click=move |_| {
                                                        leptos::task::spawn_local(async move {
                                                            let _ = logout_site().await;
                                                            set_profile.set(None);
                                                            set_show_profile_menu.set(false);
                                                        });
                                                    }
                                                >
                                                    "Logout"
                                                </button>
                                            </div>
                                        </Show>
                                    </div>
                                }
                            })
                    }}
                </Show>
            </div>
        </header>
    }
}

#[component]
fn AuthDialog(
    set_profile: WriteSignal<Option<ProfileDto>>,
    set_show_auth: WriteSignal<bool>,
) -> impl IntoView {
    let (mode, set_mode) = signal(AuthMode::Register);
    let (identifier, set_identifier) = signal(String::new());
    let (email, set_email) = signal(String::new());
    let (nickname, set_nickname) = signal(String::new());
    let (password, set_password) = signal(String::new());
    let (password_confirmation, set_password_confirmation) = signal(String::new());
    let (registration_challenge, set_registration_challenge) =
        signal(None::<RegistrationChallengeDto>);
    let (verification_code, set_verification_code) = signal(String::new());
    let (error, set_error) = signal(None::<String>);
    let (pending, set_pending) = signal(false);

    let submit = move |event: leptos::ev::SubmitEvent| {
        event.prevent_default();
        let mode = mode.get();
        let identifier = identifier.get();
        let email = email.get();
        let nickname = nickname.get();
        let password = password.get();
        let password_confirmation = password_confirmation.get();
        let challenge = registration_challenge.get();
        let verification_code = verification_code.get();
        set_error.set(None);
        set_pending.set(true);

        leptos::task::spawn_local(async move {
            let result = if let Some(challenge) = challenge {
                verify_registration_code(challenge.verification_id, verification_code)
                    .await
                    .map(AuthSubmission::Authenticated)
            } else {
                match mode {
                    AuthMode::Login => login_with_password(identifier, password)
                        .await
                        .map(AuthSubmission::Authenticated),
                    AuthMode::Register => {
                        register_with_password(email, nickname, password, password_confirmation)
                            .await
                            .map(AuthSubmission::VerificationRequired)
                    }
                }
            };
            match result {
                Ok(AuthSubmission::Authenticated(profile)) => {
                    set_profile.set(Some(profile));
                    set_show_auth.set(false);
                }
                Ok(AuthSubmission::VerificationRequired(challenge)) => {
                    set_password.set(String::new());
                    set_password_confirmation.set(String::new());
                    set_registration_challenge.set(Some(challenge));
                }
                Err(message) => set_error.set(Some(message)),
            }
            set_pending.set(false);
        });
    };

    view! {
        <div class="auth-backdrop">
            <form class="auth-card" on:submit=submit>
                <button type="button" class="auth-close" aria-label="Close" on:click=move |_| set_show_auth.set(false)>
                    "×"
                </button>
                <img src=app_url("/logo.png") alt="Patchwork" />
                <h1>{move || if registration_challenge.get().is_some() {
                    "Check your inbox"
                } else if mode.get() == AuthMode::Register {
                    "Create account"
                } else {
                    "Sign in"
                }}</h1>
                <p>{move || if let Some(challenge) = registration_challenge.get() {
                    format!("Enter the six-digit code sent to {}.", challenge.email)
                } else if mode.get() == AuthMode::Register {
                    "Create a password-protected publisher account backed by a stable UUID.".to_owned()
                } else {
                    "Use your existing Patchwork account.".to_owned()
                }}</p>
                <Show
                    when=move || registration_challenge.get().is_some()
                    fallback=move || view! {
                        <Show
                            when=move || mode.get() == AuthMode::Login
                            fallback=move || view! {
                                <>
                                    <label>
                                        <span>"Email"</span>
                                        <input
                                            type="email"
                                            autocomplete="email"
                                            required
                                            prop:value=move || email.get()
                                            on:input=move |event| set_email.set(event_target_value(&event))
                                        />
                                    </label>
                                    <label>
                                        <span>"Username"</span>
                                        <input
                                            autocomplete="username"
                                            required
                                            maxlength="16"
                                            prop:value=move || nickname.get()
                                            on:input=move |event| set_nickname.set(event_target_value(&event))
                                        />
                                    </label>
                                    <label>
                                        <span>"Password"</span>
                                        <input
                                            type="password"
                                            autocomplete="new-password"
                                            class=move || password_input_class(&password.get(), &password_confirmation.get())
                                            required
                                            minlength="12"
                                            prop:value=move || password.get()
                                            on:input=move |event| set_password.set(event_target_value(&event))
                                        />
                                    </label>
                                    <label>
                                        <span>"Confirm password"</span>
                                        <input
                                            type="password"
                                            autocomplete="new-password"
                                            class=move || password_input_class(&password.get(), &password_confirmation.get())
                                            required
                                            minlength="12"
                                            prop:value=move || password_confirmation.get()
                                            on:input=move |event| set_password_confirmation.set(event_target_value(&event))
                                        />
                                    </label>
                                    <PasswordRequirements password password_confirmation />
                                </>
                            }
                        >
                            <label>
                                <span>"Email or username"</span>
                                <input
                                    autocomplete="username"
                                    required
                                    prop:value=move || identifier.get()
                                    on:input=move |event| set_identifier.set(event_target_value(&event))
                                />
                            </label>
                            <label>
                                <span>"Password"</span>
                                <input
                                    type="password"
                                    autocomplete="current-password"
                                    required
                                    prop:value=move || password.get()
                                    on:input=move |event| set_password.set(event_target_value(&event))
                                />
                            </label>
                        </Show>
                    }
                >
                    <label>
                        <span>"Verification code"</span>
                        <input
                            class="verification-code-input"
                            inputmode="numeric"
                            autocomplete="one-time-code"
                            pattern="[0-9]{6}"
                            minlength="6"
                            maxlength="6"
                            required
                            autofocus
                            prop:value=move || verification_code.get()
                            on:input=move |event| {
                                let mut code = event_target_value(&event)
                                    .chars()
                                    .filter(char::is_ascii_digit)
                                    .take(6)
                                    .collect::<String>();
                                if code.len() > 6 {
                                    code.truncate(6);
                                }
                                set_verification_code.set(code);
                            }
                        />
                    </label>
                </Show>
                <Show when=move || error.get().is_some()>
                    <p class="auth-error">{move || error.get().unwrap_or_default()}</p>
                </Show>
                <button type="submit" class="sign-in-button" disabled=move || pending.get()
                    || (registration_challenge.get().is_some() && verification_code.get().len() != 6)
                    || (registration_challenge.get().is_none() && mode.get() == AuthMode::Register && !password_is_valid(&password.get(), &password_confirmation.get()))>
                    <span>{move || if pending.get() {
                        "Working"
                    } else if registration_challenge.get().is_some() {
                        "Verify email"
                    } else if mode.get() == AuthMode::Login {
                        "Sign in"
                    } else {
                        "Create account"
                    }}</span>
                    <ArrowRightToBracketIcon />
                </button>
                <button
                    type="button"
                    class="auth-switch-button"
                    on:click=move |_| {
                        set_error.set(None);
                        if registration_challenge.get().is_some() {
                            set_registration_challenge.set(None);
                            set_verification_code.set(String::new());
                        } else {
                            set_mode.update(|mode| {
                                *mode = if *mode == AuthMode::Register { AuthMode::Login } else { AuthMode::Register };
                            });
                        }
                    }
                >
                    {move || if registration_challenge.get().is_some() {
                        "Use another email"
                    } else if mode.get() == AuthMode::Register {
                        "Already have an account? Sign in"
                    } else {
                        "Need an account? Create one"
                    }}
                </button>
            </form>
        </div>
    }
}

#[component]
fn PasswordRequirements(
    password: ReadSignal<String>,
    password_confirmation: ReadSignal<String>,
) -> impl IntoView {
    view! {
        <Show when=move || password_feedback_is_visible(&password.get(), &password_confirmation.get())>
            <div class="password-requirements">
                <For
                    each=move || unmet_password_requirements(&password.get(), &password_confirmation.get())
                    key=|requirement| requirement.label
                    children=move |requirement| view! {
                        <p class="password-requirement">
                            {requirement.label}
                        </p>
                    }
                />
            </div>
        </Show>
    }
}

#[component]
fn WebProfilePage(
    profile: ReadSignal<Option<ProfileDto>>,
    set_profile: WriteSignal<Option<ProfileDto>>,
    viewed_profile: ReadSignal<Option<PublicProfileDto>>,
    viewed_profile_pending: ReadSignal<bool>,
    viewed_profile_error: ReadSignal<Option<String>>,
    set_show_auth: WriteSignal<bool>,
    on_open_project: Callback<RegistryProjectRef>,
    on_rescan: Callback<PublishedProject>,
    registry_error: Signal<Option<String>>,
) -> impl IntoView {
    let (editing_nickname, set_editing_nickname) = signal(false);
    let (nickname_draft, set_nickname_draft) = signal(String::new());
    let (nickname_error, set_nickname_error) = signal(None::<String>);

    let start_edit = Callback::new(move |()| {
        if let Some(profile) = profile.get() {
            set_nickname_draft.set(profile.account.nickname);
            set_nickname_error.set(None);
            set_editing_nickname.set(true);
        }
    });

    let save_nickname = Callback::new(move |()| {
        let nickname = nickname_draft.get();
        set_nickname_error.set(None);
        leptos::task::spawn_local(async move {
            match update_nickname(nickname).await {
                Ok(updated_profile) => {
                    let path = profile_path(&updated_profile.account.nickname);
                    set_profile.set(Some(updated_profile));
                    set_editing_nickname.set(false);
                    navigate_to(&path);
                }
                Err(error) => set_nickname_error.set(Some(error)),
            }
        });
    });

    let logout = Callback::new(move |()| {
        leptos::task::spawn_local(async move {
            let _ = logout_site().await;
            set_profile.set(None);
        });
    });

    view! {
        {move || {
            if viewed_profile_pending.get() {
                return view! {
                    <div class="profile-load-state">
                        <span class="project-page-spinner"></span>
                        <strong>"Loading publisher profile..."</strong>
                    </div>
                }.into_any();
            }

            if let Some(error) = viewed_profile_error.get() {
                return view! {
                    <section class="profile-missing-state">
                        <UserIcon />
                        <p class="catalog-kicker">"Profile not found"</p>
                        <h1>"No publisher at this address"</h1>
                        <p>{error}</p>
                        <a class="catalog-primary-action" href=app_url("/browse")>
                            <SearchIcon />
                            <span>"Browse projects"</span>
                        </a>
                    </section>
                }.into_any();
            }

            let Some(public_profile) = viewed_profile.get() else {
                return view! {
                    <section class="signed-out-profile">
                        <UserIcon />
                        <h1>"Publisher profile"</h1>
                        <p>"Sign in to open your own publisher profile."</p>
                        <button type="button" class="sign-in-button" on:click=move |_| set_show_auth.set(true)>
                            <span>"Sign in"</span>
                            <ArrowRightToBracketIcon />
                        </button>
                    </section>
                }.into_any();
            };

            let owner_profile = profile.get().filter(|current| {
                current.account.uuid == public_profile.account.uuid
            });
            let is_owner = owner_profile.is_some();
            let account_email = owner_profile
                .as_ref()
                .map(|current| current.account.email.clone());
            let mods = owner_profile
                .as_ref()
                .map(|current| current.mods.clone())
                .unwrap_or_else(|| public_profile.mods.clone());
            let modpacks = owner_profile
                .as_ref()
                .map(|current| current.modpacks.clone())
                .unwrap_or_else(|| public_profile.modpacks.clone());
            let public_github = public_profile.github.clone();

            view! {
                <div class="profile-page-shell">
                    <ProfilePage
                        account_email
                        account_name=public_profile.account.nickname
                        mods=published_projects(mods)
                        modpacks=published_projects(modpacks)
                        on_open_project
                        on_rescan
                    >
                        {if is_owner {
                            view! {
                                <GithubConnection profile set_profile />
                                <div class="profile-local-actions">
                                    <Show
                                        when=move || editing_nickname.get()
                                        fallback=move || view! {
                                            <button
                                                type="button"
                                                class="catalog-secondary-action"
                                                on:click=move |_| start_edit.run(())
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
                                                on:click=move |_| save_nickname.run(())
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
                                    <button
                                        type="button"
                                        class="danger-secondary-action"
                                        on:click=move |_| logout.run(())
                                    >
                                        "Logout"
                                    </button>
                                </div>
                                <Show when=move || nickname_error.get().is_some()>
                                    <p class="auth-error">{move || nickname_error.get().unwrap_or_default()}</p>
                                </Show>
                            }.into_any()
                        } else {
                            view! { <PublicGithubConnection github=public_github /> }.into_any()
                        }}
                    </ProfilePage>
                    <Show when=move || is_owner && registry_error.get().is_some()>
                        <p class="auth-error">{move || registry_error.get().unwrap_or_default()}</p>
                    </Show>
                </div>
            }.into_any()
        }}
    }
}

#[component]
fn PublicGithubConnection(github: Option<GithubAccountDto>) -> impl IntoView {
    github.map(|github| {
        let github_url = format!("https://github.com/{}", github.github_login);
        view! {
            <section class="github-connection public-github-connection">
                <div class="github-connection-heading">
                    <div>
                        <p class="catalog-kicker">"Connected account"</p>
                        <h2>"GitHub"</h2>
                    </div>
                    <GithubIcon />
                </div>
                <a class="github-account-row" href=github_url target="_blank" rel="noreferrer">
                    <img src=github.github_avatar_url alt="GitHub avatar" loading="lazy" referrerpolicy="no-referrer" />
                    <div>
                        <strong>{format!("@{}", github.github_login)}</strong>
                        <span>"View GitHub profile"</span>
                    </div>
                </a>
            </section>
        }
    })
}

#[component]
fn GithubConnection(
    profile: ReadSignal<Option<ProfileDto>>,
    set_profile: WriteSignal<Option<ProfileDto>>,
) -> impl IntoView {
    let (error, set_error) = signal(None::<String>);
    let callback_notice = github_callback_notice();
    let disconnect = move |_| {
        set_error.set(None);
        leptos::task::spawn_local(async move {
            match disconnect_github().await {
                Ok(()) => set_profile.update(|profile| {
                    if let Some(profile) = profile {
                        profile.github = None;
                    }
                }),
                Err(message) => set_error.set(Some(message)),
            }
        });
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

            {callback_notice.map(|(class, message)| view! {
                <p class=class>{message}</p>
            })}

            {move || {
                profile
                    .get()
                    .and_then(|profile| profile.github)
                    .map(|github| view! {
                        <GithubAccount github on_disconnect=disconnect />
                    }.into_any())
                    .unwrap_or_else(|| view! {
                        <div class="github-connect-empty">
                            <p>"Connect the GitHub identity you will use to publish projects."</p>
                            <a class="catalog-primary-action" href=app_url("/github/connect")>
                                <GithubIcon />
                                <span>"Connect GitHub"</span>
                            </a>
                        </div>
                    }.into_any())
            }}

            <Show when=move || error.get().is_some()>
                <p class="auth-error">{move || error.get().unwrap_or_default()}</p>
            </Show>
        </section>
    }
}

#[component]
fn GithubAccount(
    github: GithubAccountDto,
    on_disconnect: impl Fn(leptos::ev::MouseEvent) + 'static,
) -> impl IntoView {
    view! {
        <div class="github-account-row">
            <img src=github.github_avatar_url alt="GitHub avatar" loading="lazy" referrerpolicy="no-referrer" />
            <div>
                <strong>{format!("@{}", github.github_login)}</strong>
                <span>{format!("GitHub user ID {}", github.github_user_id)}</span>
            </div>
            <button type="button" class="danger-secondary-action" on:click=on_disconnect>
                "Disconnect GitHub"
            </button>
        </div>
    }
}

#[component]
fn HomePage() -> impl IntoView {
    view! {
        <section class="web-home">
            <div class="web-hero-copy">
                <p class="catalog-kicker">"Composable modding"</p>
                <h1>"Patchwork"</h1>
                <p>
                    "A launcher, registry and composition tool for stitching many mods into one buildable project."
                </p>
                <div class="web-hero-actions">
                    <a class="catalog-primary-action" href=app_url("/browse")>
                        <SearchIcon />
                        <span>"Browse registry"</span>
                    </a>
                    <a class="catalog-secondary-action" href=app_url("/upload")>
                        <UploadIcon />
                        <span>"Upload"</span>
                    </a>
                </div>
            </div>

            <div class="web-logo-panel" aria-hidden="true">
                <img src=app_url("/logo.png") alt="" />
                <div class="web-stitches">
                    <span></span>
                    <span></span>
                    <span></span>
                    <span></span>
                </div>
            </div>
        </section>

        <section class="web-feature-grid" aria-label="Patchwork features">
            <FeatureCard title="Compose" text="Collect modpacks, mods and support packages into a single build graph." />
            <FeatureCard title="Build" text="Compile the composed Rust project using the same Cargo workflow as local development." />
            <FeatureCard title="Publish" text="Upload mods and modpacks to the registry for everyone to download." />
        </section>
    }
}

#[component]
fn FeatureCard(title: &'static str, text: &'static str) -> impl IntoView {
    view! {
        <article class="web-feature-card">
            <h2>{title}</h2>
            <p>{text}</p>
        </article>
    }
}

#[component]
fn NotFoundPage() -> impl IntoView {
    view! {
        <section class="not-found-page">
            <div class="not-found-mark" aria-hidden="true">
                <span>"4"</span>
                <img src=app_url("/logo.png") alt="" />
                <span>"4"</span>
            </div>
            <p class="catalog-kicker">"Loose thread"</p>
            <h1>"Page not found"</h1>
            <p>"This address is not part of the current Patchwork pattern."</p>
            <div class="not-found-actions">
                <a class="catalog-primary-action" href=app_url("/")>
                    <HomeIcon />
                    <span>"Back home"</span>
                </a>
                <a class="catalog-secondary-action" href=app_url("/browse")>
                    <SearchIcon />
                    <span>"Browse registry"</span>
                </a>
            </div>
        </section>
    }
}

fn top_tab_class(is_active: bool) -> &'static str {
    if is_active {
        "top-tab active"
    } else {
        "top-tab"
    }
}

fn profile_button_class(is_active: bool) -> &'static str {
    if is_active {
        "profile-button active"
    } else {
        "profile-button"
    }
}

fn current_page() -> WebPage {
    let path = current_path();
    match path.as_str() {
        "/" => WebPage::Home,
        "/browse" => WebPage::Browse,
        "/upload" => WebPage::Upload,
        "/profile" => WebPage::Profile,
        _ if current_profile_nickname().is_some() => WebPage::Profile,
        _ if current_dependency_tree_project_ref().is_some() => WebPage::DependencyTree,
        _ if current_project_ref().is_some() => WebPage::Project,
        _ => WebPage::NotFound,
    }
}

fn current_profile_nickname() -> Option<String> {
    let path = current_path();
    let mut segments = path.trim_matches('/').split('/');
    if segments.next()? != "profile" {
        return None;
    }
    let nickname = segments.next()?.trim();
    if nickname.is_empty() || segments.next().is_some() {
        return None;
    }
    Some(nickname.to_owned())
}

fn current_dependency_tree_project_ref() -> Option<RegistryProjectRef> {
    let path = current_path();
    let mut segments = path.trim_matches('/').split('/');
    if segments.next()? != "deptree" {
        return None;
    }
    let project_kind = match segments.next()? {
        "mod" => RegistryProjectKind::Mod,
        "modpack" => RegistryProjectKind::Modpack,
        _ => return None,
    };
    let project_id = segments.next()?.trim();
    if project_id.is_empty() || segments.next().is_some() {
        return None;
    }
    if project_kind == RegistryProjectKind::Mod && is_generated_mod_id(project_id) {
        return None;
    }
    Some(RegistryProjectRef {
        project_kind,
        project_id: project_id.to_owned(),
    })
}

fn current_project_ref() -> Option<RegistryProjectRef> {
    let path = current_path();
    let mut segments = path.trim_matches('/').split('/');
    let project_kind = match segments.next()? {
        "mods" => RegistryProjectKind::Mod,
        "modpacks" => RegistryProjectKind::Modpack,
        _ => return None,
    };
    let project_id = segments.next()?.trim();
    if project_id.is_empty() || segments.next().is_some() {
        return None;
    }
    if project_kind == RegistryProjectKind::Mod && is_generated_mod_id(project_id) {
        return None;
    }
    Some(RegistryProjectRef {
        project_kind,
        project_id: project_id.to_owned(),
    })
}

fn project_path(project: &RegistryProjectRef) -> String {
    format!(
        "/{}/{}",
        project.project_kind.route_segment(),
        project.project_id
    )
}

fn dependency_tree_path(project: &RegistryProjectRef) -> String {
    let kind = match project.project_kind {
        RegistryProjectKind::Mod => "mod",
        RegistryProjectKind::Modpack => "modpack",
    };
    format!("/deptree/{kind}/{}", project.project_id)
}

fn profile_path(nickname: &str) -> String {
    let encoded: String = url::form_urlencoded::byte_serialize(nickname.as_bytes()).collect();
    format!("/profile/{encoded}")
}

fn load_stored_theme() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .and_then(|storage| storage.get_item(THEME_STORAGE_KEY).ok().flatten())
            .map(|theme| theme_id_or_default(&theme).to_string())
            .unwrap_or_else(|| "dark".to_string())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        "dark".to_string()
    }
}

fn store_theme(theme: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(storage) =
        web_sys::window().and_then(|window| window.local_storage().ok().flatten())
    {
        let _ = storage.set_item(THEME_STORAGE_KEY, theme);
    }
}

fn theme_id_or_default(theme: &str) -> &'static str {
    THEMES
        .iter()
        .find_map(|(theme_id, _)| (*theme_id == theme).then_some(*theme_id))
        .unwrap_or("dark")
}

fn current_path() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let path = web_sys::window()
            .and_then(|window| window.location().pathname().ok())
            .unwrap_or_else(|| "/".to_string());
        let base_path = configured_base_path();
        if base_path == "/" {
            path
        } else if path == base_path {
            "/".to_owned()
        } else {
            path.strip_prefix(&format!("{base_path}/"))
                .map(|relative| format!("/{relative}"))
                .unwrap_or(path)
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        "/".to_string()
    }
}

fn query_parameter(name: &str) -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        let query = web_sys::window()?.location().search().ok()?;
        url::form_urlencoded::parse(query.trim_start_matches('?').as_bytes())
            .find_map(|(key, value)| (key == name).then(|| value.into_owned()))
            .filter(|value| !value.is_empty())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = name;
        None
    }
}

fn upload_prefill_from_query() -> Option<RegistryScanRequest> {
    Some(RegistryScanRequest {
        repository_url: query_parameter("repository")?,
        base_path: query_parameter("path").unwrap_or_default(),
    })
}

fn upload_path(request: &RegistryScanRequest) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("repository", &request.repository_url)
        .append_pair("path", &request.base_path)
        .finish();
    format!("/upload?{query}")
}

fn navigate_to(path: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(window) = web_sys::window() {
        let _ = window.location().set_href(&app_url(path));
    }

    #[cfg(not(target_arch = "wasm32"))]
    let _ = path;
}

fn app_url(path: &str) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        let Some(base_uri) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.base_uri().ok().flatten())
        else {
            return path.to_owned();
        };
        return url::Url::parse(&base_uri)
            .and_then(|base| base.join(path.trim_start_matches('/')))
            .map(|url| url.to_string())
            .unwrap_or_else(|_| path.to_owned());
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        path.to_owned()
    }
}

#[cfg(target_arch = "wasm32")]
fn configured_base_path() -> String {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.base_uri().ok().flatten())
        .and_then(|base_uri| url::Url::parse(&base_uri).ok())
        .map(|url| {
            let path = url.path().trim_end_matches('/');
            if path.is_empty() {
                "/".to_owned()
            } else {
                path.to_owned()
            }
        })
        .unwrap_or_else(|| "/".to_owned())
}

async fn fetch_profile() -> Result<ProfileDto, String> {
    let response = Request::get(&app_url("/api/auth/me"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if !response.ok() {
        return Err(response
            .text()
            .await
            .unwrap_or_else(|_| "not authenticated".to_string()));
    }
    response.json().await.map_err(|error| error.to_string())
}

async fn fetch_public_profile(nickname: &str) -> Result<PublicProfileDto, String> {
    let response = Request::get(&app_url(&format!("/api/profiles/{nickname}")))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.status() == 404 {
        return Err("The requested Patchwork publisher does not exist.".to_owned());
    }
    parse_json_response(response, "could not load publisher profile").await
}

async fn fetch_registry_browse(
    input: &RegistryBrowseRequest,
) -> Result<RegistryBrowseResponse, String> {
    let query: String =
        url::form_urlencoded::byte_serialize(input.query.trim().as_bytes()).collect();
    let response = Request::get(&app_url(&format!(
        "/registry/search?q={query}&mods={}&modpacks={}",
        input.include_mods, input.include_modpacks
    )))
    .send()
    .await
    .map_err(|error| error.to_string())?;
    parse_json_response(response, "registry search failed").await
}

async fn fetch_registry_project(
    project: &RegistryProjectRef,
) -> Result<RegistryProjectDetails, String> {
    let response = Request::get(&app_url(&format!(
        "/registry/projects/{}/{}",
        project.project_kind.route_segment(),
        project.project_id
    )))
    .send()
    .await
    .map_err(|error| error.to_string())?;
    parse_json_response(response, "could not load registry project").await
}

async fn fetch_dependency_graph(
    project: &RegistryProjectRef,
) -> Result<RegistryDependencyGraph, String> {
    let kind = match project.project_kind {
        RegistryProjectKind::Mod => "mod",
        RegistryProjectKind::Modpack => "modpack",
    };
    let response = Request::get(&app_url(&format!(
        "/registry/deptree/{kind}/{}",
        project.project_id
    )))
    .send()
    .await
    .map_err(|error| error.to_string())?;
    parse_json_response(response, "could not load dependency graph").await
}

async fn fetch_scan_project_details(
    scan_id: &str,
    entry_id: &str,
) -> Result<RegistryProjectDetails, String> {
    let scan = fetch_registry_scan(scan_id).await?;
    let entry = scan
        .entries
        .into_iter()
        .find(|entry| entry.entry_id == entry_id)
        .ok_or_else(|| "scan entry was not found".to_owned())?;
    let publisher = fetch_profile().await.ok().map(|profile| profile.account);
    Ok(RegistryProjectDetails {
        project_kind: entry.project_kind,
        project_id: entry.project_id,
        title: entry.title,
        description: entry.description,
        version: entry.version,
        downloads: None,
        publisher_uuid: publisher
            .as_ref()
            .map(|account| account.uuid.clone())
            .unwrap_or_else(|| "-".to_owned()),
        publisher_name: publisher
            .map(|account| account.nickname)
            .unwrap_or_else(|| "Current publisher".to_owned()),
        published_at: scan
            .published_at
            .unwrap_or_else(|| "Pending publication".to_owned()),
        repository_url: scan.repository.canonical_url,
        repository_path: entry.repository_path,
        source_commit: scan.resolved_commit,
        source_tree_oid: entry.source_tree_oid,
        manifest_sha256: entry.manifest_sha256,
        manifest_url: String::new(),
        source_url: None,
        readme_url: None,
        image_url: None,
        dependencies: entry.dependencies,
    })
}

async fn start_registry_scan(input: RegistryScanRequest) -> Result<RegistryScanJobStarted, String> {
    let response = Request::post(&app_url("/registry/scan-jobs"))
        .json(&input)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_json_response(response, "could not start repository scan").await
}

async fn fetch_registry_scan_progress(job_id: &str) -> Result<RegistryScanProgress, String> {
    let response = Request::get(&app_url(&format!("/registry/scan-jobs/{job_id}")))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_json_response(response, "could not load repository scan progress").await
}

async fn poll_registry_scan(
    job_id: &str,
    set_progress: WriteSignal<Option<RegistryScanProgress>>,
) -> Result<RegistryScan, String> {
    for _ in 0..2_400 {
        let progress = fetch_registry_scan_progress(job_id).await?;
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
                TimeoutFuture::new(250).await;
                continue;
            }
        };
        set_progress.set(Some(progress));
        return result;
    }
    Err("repository scan did not finish in time".to_owned())
}

async fn fetch_registry_scan(scan_id: &str) -> Result<RegistryScan, String> {
    let response = Request::get(&app_url(&format!("/registry/scans/{scan_id}")))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_json_response(response, "could not load registry scan").await
}

async fn publish_registry_scan(
    scan_id: &str,
    input: RegistryPublishRequest,
) -> Result<RegistryPublishResponse, String> {
    let response = Request::post(&app_url(&format!("/registry/scans/{scan_id}/publish")))
        .json(&input)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_json_response(response, "registry publish failed").await
}

async fn parse_json_response<T: serde::de::DeserializeOwned>(
    response: gloo_net::http::Response,
    fallback: &str,
) -> Result<T, String> {
    if !response.ok() {
        return Err(response
            .text()
            .await
            .unwrap_or_else(|_| fallback.to_owned()));
    }
    response.json().await.map_err(|error| error.to_string())
}

async fn login_with_password(identifier: String, password: String) -> Result<ProfileDto, String> {
    let identifier = identifier.trim().to_owned();
    if identifier.is_empty() {
        return Err("Email or username is required.".to_string());
    }
    let response = Request::post(&app_url("/api/auth/login"))
        .json(&LoginRequest {
            identifier,
            password_sha256: password_sha256(&password)?,
        })
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_profile_response(response, "sign in failed").await
}

async fn register_with_password(
    email: String,
    nickname: String,
    password: String,
    password_confirmation: String,
) -> Result<RegistrationChallengeDto, String> {
    if password != password_confirmation {
        return Err("Passwords do not match.".to_string());
    }
    if !password_is_valid(&password, &password_confirmation) {
        return Err("Password does not meet the security requirements.".to_string());
    }

    let response = Request::post(&app_url("/api/auth/register"))
        .json(&RegisterRequest {
            email,
            nickname,
            password_sha256: password_sha256(&password)?,
        })
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_registration_response(response, "sign up failed").await
}

async fn verify_registration_code(
    verification_id: String,
    code: String,
) -> Result<ProfileDto, String> {
    let response = Request::post(&app_url("/api/auth/register/verify"))
        .json(&VerifyRegistrationRequest {
            verification_id,
            code,
        })
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_profile_response(response, "email verification failed").await
}

async fn update_nickname(nickname: String) -> Result<ProfileDto, String> {
    let response = Request::post(&app_url("/api/account/nickname"))
        .json(&UpdateNicknameRequest { nickname })
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_profile_response(response, "nickname update failed").await
}

async fn logout_site() -> Result<(), String> {
    let response = Request::post(&app_url("/api/auth/logout"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.ok() {
        Ok(())
    } else {
        Err(response
            .text()
            .await
            .unwrap_or_else(|_| "logout failed".to_string()))
    }
}

async fn disconnect_github() -> Result<(), String> {
    let response = Request::delete(&app_url("/github/account"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    if response.ok() {
        Ok(())
    } else {
        Err(response
            .text()
            .await
            .unwrap_or_else(|_| "GitHub disconnect failed".to_string()))
    }
}

fn github_callback_notice() -> Option<(&'static str, &'static str)> {
    #[cfg(target_arch = "wasm32")]
    {
        let query = web_sys::window()?.location().search().ok()?;
        let result = query
            .trim_start_matches('?')
            .split('&')
            .find_map(|pair| pair.strip_prefix("github="))?;
        match result {
            "connected" => Some(("github-notice success", "GitHub account connected.")),
            "already-linked" => Some((
                "github-notice error",
                "This GitHub account is already connected to another Patchwork account.",
            )),
            "cancelled" => Some(("github-notice error", "GitHub authorization was cancelled.")),
            _ => None,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

async fn parse_profile_response(
    response: gloo_net::http::Response,
    fallback: &str,
) -> Result<ProfileDto, String> {
    if !response.ok() {
        return Err(response
            .text()
            .await
            .unwrap_or_else(|_| fallback.to_string()));
    }
    response.json().await.map_err(|error| error.to_string())
}

async fn parse_registration_response(
    response: gloo_net::http::Response,
    fallback: &str,
) -> Result<RegistrationChallengeDto, String> {
    if !response.ok() {
        return Err(response
            .text()
            .await
            .unwrap_or_else(|_| fallback.to_string()));
    }
    response.json().await.map_err(|error| error.to_string())
}

fn password_sha256(password: &str) -> Result<String, String> {
    let digest = Sha256::digest(password.as_bytes());
    Ok(digest
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            let _ = write!(output, "{byte:02x}");
            output
        }))
}

fn password_requirements(password: &str, confirmation: &str) -> Vec<PasswordRequirement> {
    vec![
        PasswordRequirement {
            label: "At least 12 characters",
            met: password.chars().count() >= 12,
        },
        PasswordRequirement {
            label: "One lowercase letter",
            met: password
                .chars()
                .any(|character| character.is_ascii_lowercase()),
        },
        PasswordRequirement {
            label: "One uppercase letter",
            met: password
                .chars()
                .any(|character| character.is_ascii_uppercase()),
        },
        PasswordRequirement {
            label: "One number",
            met: password.chars().any(|character| character.is_ascii_digit()),
        },
        PasswordRequirement {
            label: "One symbol",
            met: password
                .chars()
                .any(|character| character.is_ascii_punctuation()),
        },
        PasswordRequirement {
            label: "Passwords match",
            met: !password.is_empty() && password == confirmation,
        },
    ]
}

fn unmet_password_requirements(password: &str, confirmation: &str) -> Vec<PasswordRequirement> {
    password_requirements(password, confirmation)
        .into_iter()
        .filter(|requirement| !requirement.met)
        .collect()
}

fn password_feedback_is_visible(password: &str, confirmation: &str) -> bool {
    (!password.is_empty() || !confirmation.is_empty()) && !password_is_valid(password, confirmation)
}

fn password_input_class(password: &str, confirmation: &str) -> &'static str {
    if password_feedback_is_visible(password, confirmation) {
        "invalid"
    } else {
        ""
    }
}

fn password_is_valid(password: &str, confirmation: &str) -> bool {
    password_requirements(password, confirmation)
        .into_iter()
        .all(|requirement| requirement.met)
}

fn published_projects(projects: Vec<PublishedProjectDto>) -> Vec<PublishedProject> {
    projects
        .into_iter()
        .map(|project| PublishedProject {
            project_kind: if project.kind.eq_ignore_ascii_case("modpack") {
                RegistryProjectKind::Modpack
            } else {
                RegistryProjectKind::Mod
            },
            id: project.id,
            title: project.title,
            kind: project.kind,
            downloads: project.downloads,
            latest_version: project.latest_version,
            repository_url: project.repository_url,
            repository_path: project.repository_path,
            can_rescan: project.can_rescan,
        })
        .collect()
}
