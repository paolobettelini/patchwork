use std::fmt::Write as _;

use crate::auth_types::{
    GithubAccountDto, LoginRequest, ProfileDto, PublishedProjectDto, RegisterRequest,
    RegistrationChallengeDto, UpdateNicknameRequest, VerifyRegistrationRequest,
};
use gloo_net::http::Request;
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use patchwork_registry_types::{
    RegistryPublishRequest, RegistryPublishResponse, RegistryScan, RegistryScanJobStarted,
    RegistryScanPhase, RegistryScanProgress, RegistryScanRequest,
};
use patchwork_ui::{
    ArrowRightToBracketIcon, BrowsePage, GithubIcon, HomeIcon, ProfilePage, PublishedProject,
    SearchIcon, THEMES, UploadIcon, UploadPage, UserIcon,
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
    let (registry_rescan_pending, set_registry_rescan_pending) = signal(None::<String>);

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
                        "Published {} mod version{}.",
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
    let profile_rescan = Callback::new(move |mod_id: String| {
        set_registry_error.set(None);
        set_registry_progress.set(None);
        set_registry_rescan_pending.set(Some(mod_id.clone()));
        leptos::task::spawn_local(async move {
            match start_registry_rescan(&mod_id).await {
                Ok(started) => navigate_to(&format!("/upload?job={}", started.job_id)),
                Err(message) => set_registry_error.set(Some(message)),
            }
            set_registry_rescan_pending.set(None);
        });
    });

    leptos::task::spawn_local(async move {
        if let Ok(current_profile) = fetch_profile().await {
            set_profile.set(Some(current_profile));
        }
    });

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

    view! {
        <div class="app-shell" data-theme=move || active_theme.get()>
            <TopBar active_page=page profile set_profile set_show_auth active_theme set_active_theme />
            <main class="web-workspace">
                {match page {
                    WebPage::Home => view! { <HomePage /> }.into_any(),
                    WebPage::Browse => view! { <BrowsePage allow_downloads=false /> }.into_any(),
                    WebPage::Upload => view! {
                        <UploadPage
                            authenticated=Signal::derive(move || profile.get().is_some())
                            github_connected=Signal::derive(move || profile.get().is_some_and(|profile| profile.github.is_some()))
                            scan=Signal::from(registry_scan)
                            progress=Signal::from(registry_progress)
                            pending=Signal::from(registry_pending)
                            error=Signal::from(registry_error)
                            notice=Signal::from(registry_notice)
                            on_sign_in=upload_sign_in
                            on_connect_github=upload_connect_github
                            on_scan=upload_scan
                            on_publish=upload_publish
                        />
                    }.into_any(),
                    WebPage::Profile => view! {
                        <WebProfilePage
                            profile
                            set_profile
                            set_show_auth
                            on_rescan=profile_rescan
                            rescan_pending=Signal::from(registry_rescan_pending)
                            registry_error=Signal::from(registry_error)
                        />
                    }.into_any(),
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
            <a class="brand" href="/" aria-label="Patchwork home">
                <img class="brand-logo" src="/logo.png" alt="Patchwork" />
                <div class="brand-copy">
                    <strong>"Patchwork"</strong>
                </div>
            </a>

            <span class="topbar-divider" aria-hidden="true"></span>

            <nav class="top-tabs" aria-label="Main navigation">
                <a class=top_tab_class(active_page == WebPage::Home) href="/">
                    <HomeIcon />
                    <span>"Home"</span>
                </a>
                <a class=top_tab_class(active_page == WebPage::Browse) href="/browse">
                    <SearchIcon />
                    <span>"Browse"</span>
                </a>
                <a class=top_tab_class(active_page == WebPage::Upload) href="/upload">
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
                                view! {
                                    <div class="profile-menu">
                                        <button
                                            type="button"
                                            class=move || profile_button_class(active_page == WebPage::Profile)
                                            on:click=move |_| set_show_profile_menu.update(|show| *show = !*show)
                                        >
                                            <UserIcon />
                                            <span>{profile.account.nickname}</span>
                                        </button>
                                        <Show when=move || show_profile_menu.get()>
                                            <div class="profile-dropdown">
                                                <a href="/profile" on:click=move |_| set_show_profile_menu.set(false)>
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
                <img src="/logo.png" alt="Patchwork" />
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
    set_show_auth: WriteSignal<bool>,
    on_rescan: Callback<String>,
    rescan_pending: Signal<Option<String>>,
    registry_error: Signal<Option<String>>,
) -> impl IntoView {
    let (editing_nickname, set_editing_nickname) = signal(false);
    let (nickname_draft, set_nickname_draft) = signal(String::new());
    let (nickname_error, set_nickname_error) = signal(None::<String>);

    let start_edit = move |_| {
        if let Some(profile) = profile.get() {
            set_nickname_draft.set(profile.account.nickname);
            set_nickname_error.set(None);
            set_editing_nickname.set(true);
        }
    };

    let save_nickname = move |_| {
        let nickname = nickname_draft.get();
        set_nickname_error.set(None);
        leptos::task::spawn_local(async move {
            match update_nickname(nickname).await {
                Ok(profile) => {
                    set_profile.set(Some(profile));
                    set_editing_nickname.set(false);
                }
                Err(error) => set_nickname_error.set(Some(error)),
            }
        });
    };

    let logout = move |_| {
        leptos::task::spawn_local(async move {
            let _ = logout_site().await;
            set_profile.set(None);
        });
    };

    view! {
        {move || {
            profile
                .get()
                .map(|current_profile| {
                    view! {
                        <div class="profile-page-shell">
                            <ProfilePage
                                account_email=current_profile.account.email
                                account_name=current_profile.account.nickname
                                mods=published_projects(current_profile.mods)
                                modpacks=published_projects(current_profile.modpacks)
                                on_rescan
                                rescan_pending
                            >
                                <GithubConnection profile set_profile />
                            </ProfilePage>
                            <div class="profile-local-actions">
                                <Show
                                    when=move || editing_nickname.get()
                                    fallback=move || view! {
                                        <button type="button" class="catalog-secondary-action" on:click=start_edit>
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
                                        <button type="button" class="catalog-primary-action" on:click=save_nickname>"Save"</button>
                                        <button type="button" class="catalog-secondary-action" on:click=move |_| set_editing_nickname.set(false)>"Cancel"</button>
                                    </div>
                                </Show>
                                <button type="button" class="catalog-secondary-action" on:click=logout>
                                    "Logout"
                                </button>
                            </div>
                            <Show when=move || nickname_error.get().is_some()>
                                <p class="auth-error">{move || nickname_error.get().unwrap_or_default()}</p>
                            </Show>
                            <Show when=move || registry_error.get().is_some()>
                                <p class="auth-error">{move || registry_error.get().unwrap_or_default()}</p>
                            </Show>
                        </div>
                    }
                    .into_any()
                })
                .unwrap_or_else(|| {
                    view! {
                        <section class="signed-out-profile">
                            <UserIcon />
                            <h1>"Publisher profile"</h1>
                            <p>"Sign in to see your published mods and modpacks."</p>
                            <button type="button" class="sign-in-button" on:click=move |_| set_show_auth.set(true)>
                                <span>"Sign in"</span>
                                <ArrowRightToBracketIcon />
                            </button>
                        </section>
                    }
                    .into_any()
                })
        }}
    }
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
                            <a class="catalog-primary-action" href="/github/connect">
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
                    <a class="catalog-primary-action" href="/browse">
                        <SearchIcon />
                        <span>"Browse registry"</span>
                    </a>
                    <a class="catalog-secondary-action" href="/upload">
                        <UploadIcon />
                        <span>"Upload"</span>
                    </a>
                </div>
            </div>

            <div class="web-logo-panel" aria-hidden="true">
                <img src="/logo.png" alt="" />
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
        "/browse" => WebPage::Browse,
        "/upload" => WebPage::Upload,
        "/profile" => WebPage::Profile,
        _ => WebPage::Home,
    }
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
        web_sys::window()
            .and_then(|window| window.location().pathname().ok())
            .unwrap_or_else(|| "/".to_string())
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
        query
            .trim_start_matches('?')
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .find_map(|(key, value)| (key == name).then(|| value.to_owned()))
            .filter(|value| !value.is_empty())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = name;
        None
    }
}

fn navigate_to(path: &str) {
    #[cfg(target_arch = "wasm32")]
    if let Some(window) = web_sys::window() {
        let _ = window.location().set_href(path);
    }

    #[cfg(not(target_arch = "wasm32"))]
    let _ = path;
}

async fn fetch_profile() -> Result<ProfileDto, String> {
    let response = Request::get("/api/auth/me")
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

async fn start_registry_scan(input: RegistryScanRequest) -> Result<RegistryScanJobStarted, String> {
    let response = Request::post("/registry/scan-jobs")
        .json(&input)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_json_response(response, "could not start repository scan").await
}

async fn fetch_registry_scan_progress(job_id: &str) -> Result<RegistryScanProgress, String> {
    let response = Request::get(&format!("/registry/scan-jobs/{job_id}"))
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
    let response = Request::get(&format!("/registry/scans/{scan_id}"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_json_response(response, "could not load registry scan").await
}

async fn publish_registry_scan(
    scan_id: &str,
    input: RegistryPublishRequest,
) -> Result<RegistryPublishResponse, String> {
    let response = Request::post(&format!("/registry/scans/{scan_id}/publish"))
        .json(&input)
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_json_response(response, "registry publish failed").await
}

async fn start_registry_rescan(mod_id: &str) -> Result<RegistryScanJobStarted, String> {
    let response = Request::post(&format!("/registry/mods/{mod_id}/rescan-job"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_json_response(response, "could not start registry rescan").await
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
    let response = Request::post("/api/auth/login")
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

    let response = Request::post("/api/auth/register")
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
    let response = Request::post("/api/auth/register/verify")
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
    let response = Request::post("/api/account/nickname")
        .json(&UpdateNicknameRequest { nickname })
        .map_err(|error| error.to_string())?
        .send()
        .await
        .map_err(|error| error.to_string())?;
    parse_profile_response(response, "nickname update failed").await
}

async fn logout_site() -> Result<(), String> {
    let response = Request::post("/api/auth/logout")
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
    let response = Request::delete("/github/account")
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
