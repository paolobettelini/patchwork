use crate::{
    icons::{ArrowLeftIcon, DownloadIcon, GearIcon, PlayIcon, StopIcon, TrashIcon},
    model::{DependencyPage, LauncherModpack, PatchworkTaskStatus},
    tauri_bridge::{
        create_modpack, delete_modpack, import_modpack, list_modpacks, listen_patchwork_console,
        load_dependency_page, patchwork_task_status, select_icon_file, select_modpack_icon,
        start_patchwork_action, stop_patchwork_action, update_profile_metadata,
    },
};
use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};

mod dependencies;
mod sidebar;

use dependencies::DependencyPanel;
use sidebar::ProfilesSidebar;

#[component]
pub(crate) fn HomePage(
    modpacks: ReadSignal<Vec<LauncherModpack>>,
    set_modpacks: WriteSignal<Vec<LauncherModpack>>,
    selected_modpack: ReadSignal<usize>,
    set_selected_modpack: WriteSignal<usize>,
) -> impl IntoView {
    let (show_create_modal, set_show_create_modal) = signal(false);
    let (new_modpack_id, set_new_modpack_id) = signal(String::new());
    let (new_modpack_name, set_new_modpack_name) = signal(String::new());
    let (new_modpack_description, set_new_modpack_description) = signal(String::new());
    let (new_modpack_icon, set_new_modpack_icon) = signal(None::<String>);
    let (new_modpack_icon_preview, set_new_modpack_icon_preview) = signal(None::<String>);
    let (create_error, set_create_error) = signal(None::<String>);
    let (delete_candidate, set_delete_candidate) = signal(None::<LauncherModpack>);
    let (delete_error, set_delete_error) = signal(None::<String>);
    let (dependency_page, set_dependency_page) = signal(None::<DependencyPage>);
    let (dependency_error, set_dependency_error) = signal(None::<String>);
    let (active_detail_tab, set_active_detail_tab) = signal("dependencies");
    let (compose_action, set_compose_action) = signal("compose-build");
    let (build_mode, set_build_mode) = signal("release");
    let (show_compose_menu, set_show_compose_menu) = signal(false);
    let (task_running, set_task_running) = signal(false);
    let (running_action, set_running_action) = signal(None::<String>);
    let (is_runnable, set_is_runnable) = signal(false);
    let (patchwork_action_error, set_patchwork_action_error) = signal(None::<String>);
    let (console_output, set_console_output) =
        signal("Console output will appear here.".to_string());
    let (navigation_history, set_navigation_history) = signal(Vec::<(String, String)>::new());
    let (editing_profile_title, set_editing_profile_title) = signal(false);
    let (editing_profile_description, set_editing_profile_description) = signal(false);
    let (profile_title_draft, set_profile_title_draft) = signal(String::new());
    let (profile_description_draft, set_profile_description_draft) = signal(String::new());
    let (profile_edit_error, set_profile_edit_error) = signal(None::<String>);
    let selected = move || selected_modpack_data(modpacks, selected_modpack);

    let _ = listen_patchwork_console(move |event| {
        let current_profile_id = dependency_page
            .get()
            .filter(|page| page.editable_profile)
            .map(|page| page.id);
        if current_profile_id.as_deref() != Some(event.profile_id.as_str()) {
            return;
        }

        if event.reset {
            set_console_output.set(if event.line.is_empty() {
                String::new()
            } else {
                format!("{}\n", event.line)
            });
            set_patchwork_action_error.set(None);
        } else if !event.line.is_empty() {
            set_console_output.update(|output| {
                if output == "Console output will appear here." {
                    output.clear();
                }
                output.push_str(&event.line);
                output.push('\n');
            });
        }

        set_task_running.set(event.running);
        set_running_action.set(if event.running { event.action } else { None });
        if let Some(runnable) = event.runnable {
            set_is_runnable.set(runnable);
        }
        if let Some(error) = event.core_error {
            set_patchwork_action_error.set(Some(error));
        }
    });

    install_task_status_poller(
        dependency_page,
        build_mode,
        set_console_output,
        set_task_running,
        set_running_action,
        set_is_runnable,
        set_patchwork_action_error,
    );

    Effect::new(move |_| {
        if let Some(modpack) = selected() {
            set_navigation_history.set(Vec::new());
            load_page(
                "profile".to_string(),
                modpack.id,
                set_dependency_page,
                set_dependency_error,
            );
        } else {
            set_dependency_page.set(None);
        }
    });

    Effect::new(move |_| {
        let page = dependency_page.get();
        set_patchwork_action_error.set(None);
        if let Some(page) = page {
            if page.editable_profile {
                set_profile_title_draft.set(page.name.clone());
                set_profile_description_draft.set(page.description.clone());
                set_editing_profile_title.set(false);
                set_editing_profile_description.set(false);
                set_profile_edit_error.set(None);
                refresh_profile_status(
                    page.id,
                    build_mode.get(),
                    set_console_output,
                    set_task_running,
                    set_running_action,
                    set_is_runnable,
                    set_patchwork_action_error,
                );
            } else {
                set_editing_profile_title.set(false);
                set_editing_profile_description.set(false);
                set_profile_edit_error.set(None);
                set_is_runnable.set(false);
                set_task_running.set(false);
                set_running_action.set(None);
            }
        } else {
            set_editing_profile_title.set(false);
            set_editing_profile_description.set(false);
            set_profile_edit_error.set(None);
            set_is_runnable.set(false);
            set_task_running.set(false);
            set_running_action.set(None);
        }
    });

    view! {
        <div class="home-layout">
            <ProfilesSidebar
                modpacks
                selected_modpack
                set_selected_modpack
                set_delete_candidate
                set_show_create_modal
                set_create_error
                set_new_modpack_id
                set_new_modpack_name
                set_new_modpack_description
                set_new_modpack_icon
                set_new_modpack_icon_preview
            />

            <main
                class=move || if modpacks.get().is_empty() { "home-main welcome-main" } else { "home-main" }
                style=move || format!(
                    "--profile-color: {}",
                    selected().as_ref().map(|modpack| modpack.accent.as_str()).unwrap_or("#02a9a9"),
                )
            >
                <Show
                    when=move || !modpacks.get().is_empty()
                    fallback=move || view! { <WelcomePanel /> }
                >
                    <section class="profile-header">
                        {move || {
                            let page = dependency_page.get();
                            let icon_src = match page.as_ref() {
                                Some(page) if page.editable_profile => page
                                    .icon_data_url
                                    .clone()
                                    .or_else(|| selected().and_then(|modpack| modpack.icon_data_url))
                                    .unwrap_or_else(|| "/logo.png".to_string()),
                                Some(page) => page
                                    .icon_data_url
                                    .clone()
                                    .unwrap_or_else(|| "/logo.png".to_string()),
                                None => selected()
                                    .and_then(|modpack| modpack.icon_data_url)
                                    .unwrap_or_else(|| "/logo.png".to_string()),
                            };
                            let editable_profile = page.as_ref().is_some_and(|page| page.editable_profile);
                            if editable_profile {
                                view! {
                                    <div class="logo-lockup">
                                        <button
                                            type="button"
                                            class="large-logo-button"
                                            title="Change modpack icon"
                                            on:click=move |_| {
                                                if let Some(page) = dependency_page.get() {
                                                    update_profile_icon(
                                                        page.id,
                                                        set_modpacks,
                                                        set_selected_modpack,
                                                        set_dependency_page,
                                                        set_dependency_error,
                                                    );
                                                }
                                            }
                                        >
                                            <img class="large-logo" src=icon_src alt="Selected modpack icon" />
                                        </button>
                                        <div class="thread-ring" aria-hidden="true"></div>
                                    </div>
                                }.into_any()
                            } else {
                                view! {
                                    <div class="logo-lockup readonly-logo-lockup">
                                        <Show when=move || !navigation_history.get().is_empty()>
                                            <button
                                                type="button"
                                                class="dependency-back-button"
                                                title="Go back"
                                                on:click=move |_| {
                                                    go_back(
                                                        navigation_history,
                                                        set_navigation_history,
                                                        set_dependency_page,
                                                        set_dependency_error,
                                                    );
                                                }
                                            >
                                                <ArrowLeftIcon />
                                            </button>
                                        </Show>
                                        <img class="large-logo" src=icon_src alt="Dependency page icon" />
                                    </div>
                                }.into_any()
                            }
                        }}

                        <div class="profile-intro">
                            <p class="eyebrow">{move || dependency_page.get().map(|page| page_label(&page)).unwrap_or("Selected modpack")}</p>
                            {move || {
                                let page = dependency_page.get();
                                if page.as_ref().is_some_and(|page| page.editable_profile) {
                                    if editing_profile_title.get() {
                                        view! {
                                            <input
                                                class="profile-inline-input profile-title-input"
                                                type="text"
                                                autofocus=true
                                                prop:value=move || profile_title_draft.get()
                                                on:input=move |event| {
                                                    set_profile_title_draft.set(event_target_value(&event));
                                                    set_profile_edit_error.set(None);
                                                }
                                                on:keydown=move |event| handle_inline_edit_keydown(event, set_editing_profile_title)
                                                on:blur=move |_| {
                                                    if let Some(page) = dependency_page.get().filter(|page| page.editable_profile) {
                                                        commit_profile_metadata(
                                                            page.id,
                                                            Some(profile_title_draft.get()),
                                                            None,
                                                            set_dependency_page,
                                                            set_modpacks,
                                                            set_selected_modpack,
                                                            set_editing_profile_title,
                                                            set_profile_edit_error,
                                                        );
                                                    }
                                                }
                                            />
                                        }.into_any()
                                    } else {
                                        let title = page
                                            .map(|page| page.name)
                                            .unwrap_or_else(|| "Untitled modpack".to_string());
                                        view! {
                                            <button
                                                type="button"
                                                class="editable-profile-text editable-profile-title"
                                                title="Edit profile title"
                                                on:click=move |_| {
                                                    if let Some(page) = dependency_page.get().filter(|page| page.editable_profile) {
                                                        set_profile_title_draft.set(page.name);
                                                        set_profile_edit_error.set(None);
                                                        set_editing_profile_title.set(true);
                                                    }
                                                }
                                            >
                                                {title}
                                            </button>
                                        }.into_any()
                                    }
                                } else {
                                    view! {
                                        <h1>{page.map(|page| page.name).or_else(|| selected().map(|modpack| modpack.name)).unwrap_or_else(|| "No modpack selected".to_string())}</h1>
                                    }.into_any()
                                }
                            }}
                            {move || {
                                let page = dependency_page.get();
                                if page.as_ref().is_some_and(|page| page.editable_profile) {
                                    if editing_profile_description.get() {
                                        view! {
                                            <textarea
                                                class="profile-inline-input profile-description-input"
                                                autofocus=true
                                                prop:value=move || profile_description_draft.get()
                                                on:input=move |event| {
                                                    set_profile_description_draft.set(event_target_value(&event));
                                                    set_profile_edit_error.set(None);
                                                }
                                                on:keydown=move |event| handle_inline_edit_keydown(event, set_editing_profile_description)
                                                on:blur=move |_| {
                                                    if let Some(page) = dependency_page.get().filter(|page| page.editable_profile) {
                                                        commit_profile_metadata(
                                                            page.id,
                                                            None,
                                                            Some(profile_description_draft.get()),
                                                            set_dependency_page,
                                                            set_modpacks,
                                                            set_selected_modpack,
                                                            set_editing_profile_description,
                                                            set_profile_edit_error,
                                                        );
                                                    }
                                                }
                                            />
                                        }.into_any()
                                    } else {
                                        let description = page
                                            .map(|page| page.description)
                                            .filter(|description| !description.trim().is_empty())
                                            .unwrap_or_else(|| "Click to add a description.".to_string());
                                        view! {
                                            <button
                                                type="button"
                                                class="editable-profile-text editable-profile-description"
                                                title="Edit profile description"
                                                on:click=move |_| {
                                                    if let Some(page) = dependency_page.get().filter(|page| page.editable_profile) {
                                                        set_profile_description_draft.set(page.description);
                                                        set_profile_edit_error.set(None);
                                                        set_editing_profile_description.set(true);
                                                    }
                                                }
                                            >
                                                {description}
                                            </button>
                                        }.into_any()
                                    }
                                } else {
                                    view! {
                                        <p>{page.map(|page| page.description).or_else(|| selected().map(|modpack| modpack.description)).unwrap_or_else(|| "Create a modpack to start composing.".to_string())}</p>
                                    }.into_any()
                                }
                            }}
                            {move || profile_edit_error.get().map(|error| view! { <em class="field-error profile-edit-error">{error}</em> })}

                            <div class="profile-stats-row">
                                <div class="profile-stats">
                                    <div>
                                        <span>"ID"</span>
                                        <strong>{move || dependency_page.get().map(|page| page.id).or_else(|| selected().map(|modpack| modpack.id)).unwrap_or_else(|| "—".to_string())}</strong>
                                    </div>
                                    <div>
                                        <span>"Dependencies"</span>
                                        <strong>{move || dependency_page.get().map(|page| page.distinct_dependency_count.to_string()).unwrap_or_else(|| "0".to_string())}</strong>
                                    </div>
                                    <div>
                                        <span>"Downloads"</span>
                                        <strong>
                                            {move || {
                                                if dependency_page
                                                    .get()
                                                    .is_some_and(|page| page.kind != "profile")
                                                {
                                                    "—".to_string()
                                                } else {
                                                    selected()
                                                        .map(|modpack| modpack.downloads)
                                                        .unwrap_or_else(|| "—".to_string())
                                                }
                                            }}
                                        </strong>
                                    </div>
                                </div>
                                {move || {
                                    match dependency_page.get() {
                                        Some(page) if page.editable_profile => {
                                            view! {
                                                <div class="profile-actions">
                                                    <button
                                                        type="button"
                                                        class=move || {
                                                            let running_run = task_running.get()
                                                                && running_action.get().as_deref() == Some("run");
                                                            run_button_class(is_runnable.get(), task_running.get(), running_run)
                                                        }
                                                        disabled=move || {
                                                            let running_run = task_running.get()
                                                                && running_action.get().as_deref() == Some("run");
                                                            (!running_run && task_running.get()) || (!running_run && !is_runnable.get())
                                                        }
                                                        on:click=move |_| {
                                                            let running_run = task_running.get()
                                                                && running_action.get().as_deref() == Some("run");
                                                            if running_run {
                                                                if let Some(page) = dependency_page.get() {
                                                                    stop_running_patchwork_action(page.id, set_console_output);
                                                                }
                                                            } else if let Some(page) = dependency_page.get() {
                                                                start_selected_patchwork_action(
                                                                    page.id,
                                                                    "run",
                                                                    build_mode.get(),
                                                                    set_console_output,
                                                                    set_task_running,
                                                                    set_running_action,
                                                                    set_patchwork_action_error,
                                                                );
                                                            }
                                                        }
                                                    >
                                                        {move || {
                                                            let running_run = task_running.get()
                                                                && running_action.get().as_deref() == Some("run");
                                                            if running_run {
                                                                view! { <StopIcon /> }.into_any()
                                                            } else {
                                                                view! { <PlayIcon /> }.into_any()
                                                            }
                                                        }}
                                                        <span>{move || {
                                                            let running_run = task_running.get()
                                                                && running_action.get().as_deref() == Some("run");
                                                            if running_run { "Stop" } else { "Run" }
                                                        }}</span>
                                                    </button>
                                                    <div class="compose-action-group">
                                                        <button
                                                            type="button"
                                                            class=move || if task_running.get() {
                                                                "primary-action compose-primary-action disabled-action"
                                                            } else {
                                                                "primary-action compose-primary-action"
                                                            }
                                                            disabled=move || task_running.get()
                                                            on:click=move |_| {
                                                                if let Some(page) = dependency_page.get() {
                                                                    start_selected_patchwork_action(
                                                                        page.id,
                                                                        compose_action.get(),
                                                                        build_mode.get(),
                                                                        set_console_output,
                                                                        set_task_running,
                                                                        set_running_action,
                                                                        set_patchwork_action_error,
                                                                    );
                                                                }
                                                            }
                                                        >
                                                            {move || {
                                                                let compose_running = task_running.get()
                                                                    && running_action.get().as_deref() != Some("run");
                                                                if compose_running {
                                                                    view! { <span class="button-spinner" aria-hidden="true"></span> }.into_any()
                                                                } else {
                                                                    view! { <img class="sew-action-icon" src="/sew.png" alt="" /> }.into_any()
                                                                }
                                                            }}
                                                            <span>{move || compose_action_label(compose_action.get())}</span>
                                                        </button>
                                                        <div class="compose-selector">
                                                            <button
                                                                type="button"
                                                                class="compose-selector-button"
                                                                aria-label="Configure compose action"
                                                                on:click=move |_| set_show_compose_menu.update(|show| *show = !*show)
                                                            >
                                                                <GearIcon />
                                                            </button>
                                                            <Show when=move || show_compose_menu.get()>
                                                                <div class="compose-selector-menu">
                                                                    <div class="compose-selector-section">
                                                                        <span>"Action"</span>
                                                                        <For
                                                                            each=move || compose_action_alternatives(compose_action.get())
                                                                            key=|mode| *mode
                                                                            children=move |mode| {
                                                                                view! {
                                                                                    <button
                                                                                        type="button"
                                                                                        on:click=move |_| {
                                                                                            set_compose_action.set(mode);
                                                                                            set_show_compose_menu.set(false);
                                                                                        }
                                                                                    >
                                                                                        {compose_action_label(mode)}
                                                                                    </button>
                                                                                }
                                                                            }
                                                                        />
                                                                    </div>
                                                                    <div class="compose-selector-section separated">
                                                                        <span>"Mode"</span>
                                                                        <button
                                                                            type="button"
                                                                            class=move || if build_mode.get() == "release" { "selected" } else { "" }
                                                                            on:click=move |_| set_build_mode.set("release")
                                                                        >
                                                                            "Release mode"
                                                                        </button>
                                                                        <button
                                                                            type="button"
                                                                            class=move || if build_mode.get() == "debug" { "selected" } else { "" }
                                                                            on:click=move |_| set_build_mode.set("debug")
                                                                        >
                                                                            "Debug mode"
                                                                        </button>
                                                                    </div>
                                                                </div>
                                                            </Show>
                                                        </div>
                                                    </div>
                                                </div>
                                            }.into_any()
                                        }
                                        Some(page) if page.kind == "modpack" => {
                                            view! {
                                                <div class="profile-actions">
                                                    <button type="button" class="secondary-action dependency-import-action">
                                                        <DownloadIcon />
                                                        <span>"Import"</span>
                                                    </button>
                                                </div>
                                            }.into_any()
                                        }
                                        _ => view! {}.into_any(),
                                    }
                                }}
                            </div>
                        </div>
                    </section>

                    <section class="dependency-workspace">
                        <div class="detail-tabs" aria-label="Modpack detail tabs">
                            <button
                                type="button"
                                class=move || detail_tab_class(active_detail_tab.get() == "dependencies")
                                on:click=move |_| set_active_detail_tab.set("dependencies")
                            >
                                "Dependencies"
                            </button>
                            <button
                                type="button"
                                class=move || detail_tab_class(active_detail_tab.get() == "console")
                                on:click=move |_| set_active_detail_tab.set("console")
                            >
                                "Console"
                            </button>
                        </div>

                        <Show
                            when=move || active_detail_tab.get() == "dependencies"
                            fallback=move || view! { <pre class="console-output">{move || console_output.get()}</pre> }
                        >
                            <DependencyPanel
                                page=dependency_page
                                error=dependency_error
                                action_error=patchwork_action_error
                                set_page=set_dependency_page
                                set_error=set_dependency_error
                                current_page=dependency_page
                                set_history=set_navigation_history
                            />
                        </Show>
                    </section>
                </Show>
            </main>

            <Show when=move || show_create_modal.get()>
                <div class="modal-backdrop">
                    <div class="modal-card">
                        <div class="section-heading modal-heading">
                            <div>
                                <p class="eyebrow">"New modpack"</p>
                                <h2>"Create new modpack"</h2>
                            </div>
                        </div>

                        <label class="path-input">
                            <span class="path-label">
                                <strong>"ID"</strong>
                                <small>"Unique file-safe identifier for this profile."</small>
                            </span>
                            <input
                                type="text"
                                placeholder="my-awesome-modpack"
                                prop:value=move || new_modpack_id.get()
                                on:input=move |event| {
                                    set_new_modpack_id.set(event_target_value(&event));
                                    set_create_error.set(None);
                                }
                            />
                        </label>

                        <label class="path-input">
                            <span class="path-label">
                                <strong>"Name"</strong>
                                <small>"Human-readable modpack title."</small>
                            </span>
                            <input
                                type="text"
                                placeholder="My Awesome Modpack"
                                prop:value=move || new_modpack_name.get()
                                on:input=move |event| {
                                    set_new_modpack_name.set(event_target_value(&event));
                                    set_create_error.set(None);
                                }
                            />
                        </label>

                        <label class="path-input">
                            <span class="path-label">
                                <strong>"Description"</strong>
                                <small>"Short description shown on the launcher page."</small>
                            </span>
                            <input
                                type="text"
                                placeholder="A carefully stitched Patchwork profile."
                                prop:value=move || new_modpack_description.get()
                                on:input=move |event| {
                                    set_new_modpack_description.set(event_target_value(&event));
                                    set_create_error.set(None);
                                }
                            />
                        </label>

                        <div class="path-input">
                            <span class="path-label">
                                <strong>"Favicon"</strong>
                                <small>"Optional image copied next to the new profile."</small>
                            </span>
                            <div class="create-favicon-row">
                                <div class="logo-lockup create-logo-lockup">
                                    <button
                                        type="button"
                                        class="large-logo-button create-logo-button"
                                        title="Select favicon"
                                        on:click=move |_| {
                                            leptos::task::spawn_local(async move {
                                                if let Ok(Some(icon)) = select_icon_file().await {
                                                    set_new_modpack_icon.set(Some(icon.path));
                                                    set_new_modpack_icon_preview.set(Some(icon.data_url));
                                                }
                                            });
                                        }
                                    >
                                        <img
                                            class="large-logo create-logo"
                                            src=move || new_modpack_icon_preview.get().unwrap_or_else(|| "/logo.png".to_string())
                                            alt="New modpack favicon preview"
                                        />
                                    </button>
                                    <div class="thread-ring" aria-hidden="true"></div>
                                </div>
                                <button
                                    type="button"
                                    class="secondary-action"
                                    on:click=move |_| {
                                        leptos::task::spawn_local(async move {
                                            if let Ok(Some(icon)) = select_icon_file().await {
                                                set_new_modpack_icon.set(Some(icon.path));
                                                set_new_modpack_icon_preview.set(Some(icon.data_url));
                                            }
                                        });
                                    }
                                >
                                    "Choose favicon"
                                </button>
                            </div>
                        </div>

                        {move || create_error.get().map(|error| view! { <em class="field-error">{error}</em> })}

                        <div class="modal-actions">
                            <button
                                type="button"
                                class="secondary-action"
                                on:click=move |_| set_show_create_modal.set(false)
                            >
                                "Cancel"
                            </button>
                            <button
                                type="button"
                                class="secondary-action"
                                on:click=move |_| {
                                    leptos::task::spawn_local(async move {
                                        match import_modpack().await {
                                            Ok(Some(imported)) => {
                                                set_create_error.set(None);
                                                set_show_create_modal.set(false);
                                                refresh_modpacks_selecting(
                                                    imported.id,
                                                    set_modpacks,
                                                    set_selected_modpack,
                                                )
                                                .await;
                                            }
                                            Ok(None) => {}
                                            Err(error) => set_create_error.set(Some(js_error_to_string(error))),
                                        }
                                    });
                                }
                            >
                                "Import"
                            </button>
                            <button
                                type="button"
                                class="primary-action"
                                on:click=move |_| {
                                    let id = new_modpack_id.get();
                                    let name = new_modpack_name.get();
                                    let description = new_modpack_description.get();
                                    let icon_path = new_modpack_icon.get();
                                    leptos::task::spawn_local(async move {
                                        match create_modpack(
                                            &id,
                                            &name,
                                            &description,
                                            icon_path.as_deref(),
                                        ).await {
                                            Ok(created) => {
                                                set_create_error.set(None);
                                                set_show_create_modal.set(false);
                                                set_new_modpack_id.set(String::new());
                                                set_new_modpack_name.set(String::new());
                                                set_new_modpack_description.set(String::new());
                                                set_new_modpack_icon.set(None);
                                                set_new_modpack_icon_preview.set(None);
                                                refresh_modpacks_selecting(created.id, set_modpacks, set_selected_modpack).await;
                                            }
                                            Err(error) => set_create_error.set(Some(js_error_to_string(error))),
                                        }
                                    });
                                }
                            >
                                "Create"
                            </button>
                        </div>
                    </div>
                </div>
            </Show>

            <Show when=move || delete_candidate.get().is_some()>
                <div class="modal-backdrop">
                    <div class="modal-card danger-modal">
                        <div class="section-heading modal-heading">
                            <div>
                                <p class="eyebrow">"Delete modpack"</p>
                                <h2>"Are you sure you want to proceed?"</h2>
                            </div>
                        </div>

                        <p class="modal-copy">
                            {move || delete_candidate.get().map(|modpack| format!("The modpack profile '{}' will be deleted.", modpack.name)).unwrap_or_default()}
                        </p>

                        {move || delete_error.get().map(|error| view! { <em class="field-error">{error}</em> })}

                        <div class="modal-actions">
                            <button
                                type="button"
                                class="secondary-action"
                                on:click=move |_| {
                                    set_delete_error.set(None);
                                    set_delete_candidate.set(None);
                                }
                            >
                                "Cancel"
                            </button>
                            <button
                                type="button"
                                class="danger-action"
                                on:click=move |_| {
                                    if let Some(modpack) = delete_candidate.get() {
                                        leptos::task::spawn_local(async move {
                                            match delete_modpack(&modpack.id).await {
                                                Ok(()) => {
                                                    set_delete_error.set(None);
                                                    set_delete_candidate.set(None);
                                                    refresh_modpacks_after_delete(
                                                        set_modpacks,
                                                        set_selected_modpack,
                                                    )
                                                    .await;
                                                }
                                                Err(error) => set_delete_error.set(Some(js_error_to_string(error))),
                                            }
                                        });
                                    }
                                }
                            >
                                <TrashIcon />
                                <span>"Delete"</span>
                            </button>
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    }
}

#[component]
fn WelcomePanel() -> impl IntoView {
    view! {
        <section class="welcome-panel" aria-label="Welcome">
            <div class="welcome-copy">
                <img class="welcome-logo" src="/logo.png" alt="Patchwork logo" />
                <div class="welcome-text-stack">
                    <span class="welcome-kicker">"Welcome to"</span>
                    <img class="welcome-wordmark" src="/patchwork-word.svg" alt="Patchwork" />
                </div>
            </div>
        </section>
    }
}

fn selected_modpack_data(
    modpacks: ReadSignal<Vec<LauncherModpack>>,
    selected_modpack: ReadSignal<usize>,
) -> Option<LauncherModpack> {
    modpacks.with(|modpacks| {
        modpacks
            .get(selected_modpack.get())
            .cloned()
            .or_else(|| modpacks.first().cloned())
    })
}

fn update_profile_icon(
    modpack_id: String,
    set_modpacks: WriteSignal<Vec<LauncherModpack>>,
    set_selected_modpack: WriteSignal<usize>,
    set_dependency_page: WriteSignal<Option<DependencyPage>>,
    set_dependency_error: WriteSignal<Option<String>>,
) {
    leptos::task::spawn_local(async move {
        match select_modpack_icon(&modpack_id).await {
            Ok(Some(updated)) => {
                refresh_modpacks_selecting(updated.id.clone(), set_modpacks, set_selected_modpack)
                    .await;
                load_dependency_page_into(
                    "profile".to_string(),
                    updated.id,
                    set_dependency_page,
                    set_dependency_error,
                )
                .await;
            }
            Ok(None) => {}
            Err(error) => set_dependency_error.set(Some(js_error_to_string(error))),
        }
    });
}

fn commit_profile_metadata(
    profile_id: String,
    name: Option<String>,
    description: Option<String>,
    set_dependency_page: WriteSignal<Option<DependencyPage>>,
    set_modpacks: WriteSignal<Vec<LauncherModpack>>,
    set_selected_modpack: WriteSignal<usize>,
    set_editing: WriteSignal<bool>,
    set_profile_edit_error: WriteSignal<Option<String>>,
) {
    leptos::task::spawn_local(async move {
        match update_profile_metadata(&profile_id, name.as_deref(), description.as_deref()).await {
            Ok(page) => {
                let updated_id = page.id.clone();
                set_profile_edit_error.set(None);
                set_editing.set(false);
                set_dependency_page.set(Some(page));
                refresh_modpacks_selecting(updated_id, set_modpacks, set_selected_modpack).await;
            }
            Err(error) => set_profile_edit_error.set(Some(js_error_to_string(error))),
        }
    });
}

fn handle_inline_edit_keydown(event: web_sys::KeyboardEvent, set_editing: WriteSignal<bool>) {
    match event.key().as_str() {
        "Enter" => {
            if !event.shift_key() {
                event.prevent_default();
                if let Some(target) = event
                    .target()
                    .and_then(|target| target.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    let _ = target.blur();
                }
            }
        }
        "Escape" => {
            event.prevent_default();
            set_editing.set(false);
        }
        _ => {}
    }
}

fn load_page(
    kind: String,
    id: String,
    set_page: WriteSignal<Option<DependencyPage>>,
    set_error: WriteSignal<Option<String>>,
) {
    leptos::task::spawn_local(async move {
        load_dependency_page_into(kind, id, set_page, set_error).await;
    });
}

fn go_back(
    history: ReadSignal<Vec<(String, String)>>,
    set_history: WriteSignal<Vec<(String, String)>>,
    set_page: WriteSignal<Option<DependencyPage>>,
    set_error: WriteSignal<Option<String>>,
) {
    if let Some((kind, id)) = history.get().last().cloned() {
        set_history.update(|history| {
            history.pop();
        });
        load_page(kind, id, set_page, set_error);
    }
}

async fn load_dependency_page_into(
    kind: String,
    id: String,
    set_page: WriteSignal<Option<DependencyPage>>,
    set_error: WriteSignal<Option<String>>,
) {
    match load_dependency_page(&kind, &id).await {
        Ok(page) => {
            set_error.set(None);
            set_page.set(Some(page));
        }
        Err(error) => set_error.set(Some(js_error_to_string(error))),
    }
}

fn install_task_status_poller(
    dependency_page: ReadSignal<Option<DependencyPage>>,
    build_mode: ReadSignal<&'static str>,
    set_console_output: WriteSignal<String>,
    set_task_running: WriteSignal<bool>,
    set_running_action: WriteSignal<Option<String>>,
    set_is_runnable: WriteSignal<bool>,
    set_patchwork_action_error: WriteSignal<Option<String>>,
) {
    let closure = Closure::wrap(Box::new(move || {
        let Some(page) = dependency_page.get().filter(|page| page.editable_profile) else {
            return;
        };
        refresh_profile_status(
            page.id,
            build_mode.get(),
            set_console_output,
            set_task_running,
            set_running_action,
            set_is_runnable,
            set_patchwork_action_error,
        );
    }) as Box<dyn FnMut()>);

    if let Some(window) = web_sys::window() {
        let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            650,
        );
    }
    closure.forget();
}

fn refresh_profile_status(
    profile_id: String,
    build_mode: &'static str,
    set_console_output: WriteSignal<String>,
    set_task_running: WriteSignal<bool>,
    set_running_action: WriteSignal<Option<String>>,
    set_is_runnable: WriteSignal<bool>,
    set_patchwork_action_error: WriteSignal<Option<String>>,
) {
    leptos::task::spawn_local(async move {
        if let Ok(status) = patchwork_task_status(&profile_id, build_mode).await {
            apply_task_status(
                status,
                set_console_output,
                set_task_running,
                set_running_action,
                set_is_runnable,
                set_patchwork_action_error,
            );
        }
    });
}

fn apply_task_status(
    status: PatchworkTaskStatus,
    set_console_output: WriteSignal<String>,
    set_task_running: WriteSignal<bool>,
    set_running_action: WriteSignal<Option<String>>,
    set_is_runnable: WriteSignal<bool>,
    set_patchwork_action_error: WriteSignal<Option<String>>,
) {
    set_console_output.set(status.output);
    set_task_running.set(status.running);
    set_running_action.set(if status.running { status.action } else { None });
    set_is_runnable.set(status.runnable);
    set_patchwork_action_error.set(status.core_error);
}

fn start_selected_patchwork_action(
    profile_id: String,
    action: &'static str,
    build_mode: &'static str,
    set_console_output: WriteSignal<String>,
    set_task_running: WriteSignal<bool>,
    set_running_action: WriteSignal<Option<String>>,
    set_patchwork_action_error: WriteSignal<Option<String>>,
) {
    set_console_output.set(format!(
        "Starting {} ({})...",
        compose_action_label(action),
        build_mode_label(build_mode)
    ));
    set_task_running.set(true);
    set_running_action.set(Some(action.to_string()));
    set_patchwork_action_error.set(None);
    leptos::task::spawn_local(async move {
        match start_patchwork_action(&profile_id, action, build_mode).await {
            Ok(_) => {}
            Err(error) => {
                let error = js_error_to_string(error);
                set_task_running.set(false);
                set_running_action.set(None);
                set_console_output.set(error.clone());
                set_patchwork_action_error.set(Some(error));
            }
        }
    });
}

fn stop_running_patchwork_action(profile_id: String, set_console_output: WriteSignal<String>) {
    leptos::task::spawn_local(async move {
        if let Err(error) = stop_patchwork_action(&profile_id).await {
            let error = js_error_to_string(error);
            set_console_output.update(|output| {
                if !output.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&error);
                output.push('\n');
            });
        }
    });
}

async fn refresh_modpacks_selecting(
    selected_id: String,
    set_modpacks: WriteSignal<Vec<LauncherModpack>>,
    set_selected_modpack: WriteSignal<usize>,
) {
    if let Ok(modpacks) = list_modpacks().await {
        let selected_index = modpacks
            .iter()
            .position(|modpack| modpack.id == selected_id)
            .unwrap_or(0);
        set_selected_modpack.set(usize::MAX);
        set_modpacks.set(modpacks);
        set_selected_modpack.set(selected_index);
    }
}

async fn refresh_modpacks_after_delete(
    set_modpacks: WriteSignal<Vec<LauncherModpack>>,
    set_selected_modpack: WriteSignal<usize>,
) {
    if let Ok(modpacks) = list_modpacks().await {
        set_selected_modpack.set(usize::MAX);
        let selected_index = if modpacks.is_empty() { usize::MAX } else { 0 };
        set_modpacks.set(modpacks);
        set_selected_modpack.set(selected_index);
    }
}

fn page_label(page: &DependencyPage) -> &'static str {
    match page.kind.as_str() {
        "mod" => "Mod",
        "modpack" => "Modpack",
        "profile" => "Profile modpack",
        _ => "Dependency page",
    }
}

fn detail_tab_class(is_active: bool) -> &'static str {
    if is_active {
        "detail-tab active"
    } else {
        "detail-tab"
    }
}

fn run_button_class(runnable: bool, task_running: bool, running_run: bool) -> &'static str {
    if running_run {
        "run-action running"
    } else if task_running || !runnable {
        "run-action disabled"
    } else {
        "run-action ready"
    }
}

fn compose_action_label(mode: &'static str) -> &'static str {
    match mode {
        "compose" => "Compose",
        "build" => "Build",
        _ => "Compose & Build",
    }
}

fn compose_action_alternatives(mode: &'static str) -> Vec<&'static str> {
    ["compose-build", "compose", "build"]
        .into_iter()
        .filter(|candidate| *candidate != mode)
        .collect()
}

fn build_mode_label(mode: &'static str) -> &'static str {
    match mode {
        "debug" => "Debug mode",
        _ => "Release mode",
    }
}

fn js_error_to_string(error: JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "Unexpected launcher error".to_string())
}
