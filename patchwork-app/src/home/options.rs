use std::collections::BTreeMap;

use crate::{
    icons::{PlusIcon, TrashIcon},
    model::{DependencyPage, ProcessOptions, ProfileOptions},
    tauri_bridge::{load_profile_options, update_profile_options},
};
use leptos::prelude::*;
use wasm_bindgen::JsValue;

#[derive(Clone, Debug, Eq, PartialEq)]
struct EnvironmentRow {
    id: u32,
    name: String,
    value: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ArgumentRow {
    id: u32,
    value: String,
}

#[component]
pub(super) fn OptionsPanel(
    page: ReadSignal<Option<DependencyPage>>,
    build_mode: ReadSignal<&'static str>,
) -> impl IntoView {
    let (build_env, set_build_env) = signal(Vec::<EnvironmentRow>::new());
    let (build_args, set_build_args) = signal(Vec::<ArgumentRow>::new());
    let (run_env, set_run_env) = signal(Vec::<EnvironmentRow>::new());
    let (run_args, set_run_args) = signal(Vec::<ArgumentRow>::new());
    let (defaults, set_defaults) = signal(ProfileOptions::default());
    let (next_id, set_next_id) = signal(0_u32);
    let (loading, set_loading) = signal(false);
    let (saving, set_saving) = signal(false);
    let (error, set_error) = signal(None::<String>);
    let (notice, set_notice) = signal(None::<String>);

    Effect::new(move |_| {
        let Some(profile_id) = page
            .get()
            .filter(|page| page.editable_profile)
            .map(|page| page.id)
        else {
            return;
        };
        let mode = build_mode.get();
        set_loading.set(true);
        set_error.set(None);
        set_notice.set(None);
        leptos::task::spawn_local(async move {
            match load_profile_options(&profile_id, mode).await {
                Ok(view) => {
                    let mut id = 0_u32;
                    set_build_env.set(environment_rows(view.options.build.env, &mut id));
                    set_build_args.set(argument_rows(view.options.build.args, &mut id));
                    set_run_env.set(environment_rows(view.options.run.env, &mut id));
                    set_run_args.set(argument_rows(view.options.run.args, &mut id));
                    set_defaults.set(view.defaults);
                    set_next_id.set(id);
                }
                Err(value) => set_error.set(Some(js_error_to_string(value))),
            }
            set_loading.set(false);
        });
    });

    let save = move |_| {
        let Some(profile_id) = page
            .get()
            .filter(|page| page.editable_profile)
            .map(|page| page.id)
        else {
            return;
        };
        let options = match collect_options(build_env, build_args, run_env, run_args) {
            Ok(options) => options,
            Err(message) => {
                set_error.set(Some(message));
                return;
            }
        };
        set_saving.set(true);
        set_error.set(None);
        set_notice.set(None);
        leptos::task::spawn_local(async move {
            match update_profile_options(&profile_id, options).await {
                Ok(()) => set_notice.set(Some("Profile options saved.".to_owned())),
                Err(value) => set_error.set(Some(js_error_to_string(value))),
            }
            set_saving.set(false);
        });
    };

    view! {
        <div class="profile-options-panel">
            <div class="profile-options-heading">
                <div>
                    <h2>"Profile options"</h2>
                    <p>"Arguments are passed directly to the process as separate values."</p>
                </div>
                <button
                    type="button"
                    class="secondary-action options-save-action"
                    disabled=move || loading.get() || saving.get()
                    on:click=save
                >
                    {move || if saving.get() { "Saving..." } else { "Save options" }}
                </button>
            </div>

            {move || error.get().map(|message| view! { <p class="profile-options-feedback error">{message}</p> })}
            {move || notice.get().map(|message| view! { <p class="profile-options-feedback success">{message}</p> })}

            <Show
                when=move || !loading.get()
                fallback=move || view! { <div class="options-loading"><span class="button-spinner"></span><span>"Loading options..."</span></div> }
            >
                <div class="profile-options-grid">
                    <ProcessOptionsEditor
                        title="Compilation"
                        description="Applied to cargo build for this profile."
                        default_options=Signal::derive(move || defaults.get().build)
                        env_rows=build_env
                        set_env_rows=set_build_env
                        arg_rows=build_args
                        set_arg_rows=set_build_args
                        next_id
                        set_next_id
                        auth_defaults=false
                    />
                    <ProcessOptionsEditor
                        title="Executable"
                        description="Applied only when the cached executable is started."
                        default_options=Signal::derive(move || defaults.get().run)
                        env_rows=run_env
                        set_env_rows=set_run_env
                        arg_rows=run_args
                        set_arg_rows=set_run_args
                        next_id
                        set_next_id
                        auth_defaults=true
                    />
                </div>
            </Show>
        </div>
    }
}

#[component]
fn ProcessOptionsEditor(
    title: &'static str,
    description: &'static str,
    default_options: Signal<ProcessOptions>,
    env_rows: ReadSignal<Vec<EnvironmentRow>>,
    set_env_rows: WriteSignal<Vec<EnvironmentRow>>,
    arg_rows: ReadSignal<Vec<ArgumentRow>>,
    set_arg_rows: WriteSignal<Vec<ArgumentRow>>,
    next_id: ReadSignal<u32>,
    set_next_id: WriteSignal<u32>,
    auth_defaults: bool,
) -> impl IntoView {
    view! {
        <section class="profile-options-section">
            <div class="section-heading">
                <div>
                    <h3>{title}</h3>
                    <p>{description}</p>
                </div>
            </div>

            <div class="options-field-group">
                <div class="options-field-heading">
                    <div>
                        <strong>"Environment variables"</strong>
                        <small>"Patchwork defaults are read-only; custom values are stored in the profile."</small>
                    </div>
                    <button
                        type="button"
                        class="icon-button"
                        title="Add environment variable"
                        aria-label="Add environment variable"
                        on:click=move |_| add_environment_row(set_env_rows, next_id, set_next_id)
                    >
                        <PlusIcon />
                    </button>
                </div>

                <div class="options-rows">
                    <For
                        each=move || {
                            default_options
                                .get()
                                .env
                                .into_iter()
                                .collect::<Vec<(String, String)>>()
                        }
                        key=|(name, _)| name.clone()
                        children=move |(name, value)| {
                            let auth_only = auth_defaults && name.starts_with("PATCHWORK_AUTH_");
                            view! {
                                <div class="options-env-row readonly">
                                    <input type="text" prop:value=name readonly disabled />
                                    <input type="text" prop:value=value readonly disabled />
                                    <span class="options-row-note">{if auth_only { "Auth only" } else { "Default" }}</span>
                                </div>
                            }
                        }
                    />
                    <For
                        each=move || env_rows.get()
                        key=|row| row.id
                        children=move |row| {
                            let id = row.id;
                            view! {
                                <div class="options-env-row">
                                    <input
                                        type="text"
                                        placeholder="VARIABLE_NAME"
                                        prop:value=row.name
                                        on:input=move |event| update_environment_name(
                                            set_env_rows,
                                            id,
                                            event_target_value(&event),
                                        )
                                    />
                                    <input
                                        type="text"
                                        placeholder="Value"
                                        prop:value=row.value
                                        on:input=move |event| update_environment_value(
                                            set_env_rows,
                                            id,
                                            event_target_value(&event),
                                        )
                                    />
                                    <button
                                        type="button"
                                        class="icon-button danger"
                                        title="Remove environment variable"
                                        aria-label="Remove environment variable"
                                        on:click=move |_| set_env_rows.update(|rows| rows.retain(|row| row.id != id))
                                    >
                                        <TrashIcon />
                                    </button>
                                </div>
                            }
                        }
                    />
                </div>
            </div>

            <div class="options-field-group">
                <div class="options-field-heading">
                    <div>
                        <strong>"Arguments"</strong>
                        <small>"Add one argv value per row; no shell parsing or expansion is performed."</small>
                    </div>
                    <button
                        type="button"
                        class="icon-button"
                        title="Add argument"
                        aria-label="Add argument"
                        on:click=move |_| add_argument_row(set_arg_rows, next_id, set_next_id)
                    >
                        <PlusIcon />
                    </button>
                </div>

                <div class="options-rows">
                    <For
                        each=move || {
                            default_options
                                .get()
                                .args
                                .into_iter()
                                .enumerate()
                                .collect::<Vec<(usize, String)>>()
                        }
                        key=|(index, value)| (*index, value.clone())
                        children=move |(_, value)| view! {
                            <div class="options-argument-row readonly">
                                <input type="text" prop:value=value readonly disabled />
                                <span class="options-row-note">"Default"</span>
                            </div>
                        }
                    />
                    <For
                        each=move || arg_rows.get()
                        key=|row| row.id
                        children=move |row| {
                            let id = row.id;
                            view! {
                                <div class="options-argument-row">
                                    <input
                                        type="text"
                                        placeholder="--argument or value"
                                        prop:value=row.value
                                        on:input=move |event| set_arg_rows.update(|rows| {
                                            if let Some(row) = rows.iter_mut().find(|row| row.id == id) {
                                                row.value = event_target_value(&event);
                                            }
                                        })
                                    />
                                    <button
                                        type="button"
                                        class="icon-button danger"
                                        title="Remove argument"
                                        aria-label="Remove argument"
                                        on:click=move |_| set_arg_rows.update(|rows| rows.retain(|row| row.id != id))
                                    >
                                        <TrashIcon />
                                    </button>
                                </div>
                            }
                        }
                    />
                </div>
            </div>
        </section>
    }
}

fn environment_rows(
    environment: BTreeMap<String, String>,
    next_id: &mut u32,
) -> Vec<EnvironmentRow> {
    environment
        .into_iter()
        .map(|(name, value)| {
            let row = EnvironmentRow {
                id: *next_id,
                name,
                value,
            };
            *next_id += 1;
            row
        })
        .collect()
}

fn argument_rows(arguments: Vec<String>, next_id: &mut u32) -> Vec<ArgumentRow> {
    arguments
        .into_iter()
        .map(|value| {
            let row = ArgumentRow {
                id: *next_id,
                value,
            };
            *next_id += 1;
            row
        })
        .collect()
}

fn add_environment_row(
    set_rows: WriteSignal<Vec<EnvironmentRow>>,
    next_id: ReadSignal<u32>,
    set_next_id: WriteSignal<u32>,
) {
    let id = next_id.get_untracked();
    set_next_id.set(id + 1);
    set_rows.update(|rows| {
        rows.push(EnvironmentRow {
            id,
            name: String::new(),
            value: String::new(),
        });
    });
}

fn add_argument_row(
    set_rows: WriteSignal<Vec<ArgumentRow>>,
    next_id: ReadSignal<u32>,
    set_next_id: WriteSignal<u32>,
) {
    let id = next_id.get_untracked();
    set_next_id.set(id + 1);
    set_rows.update(|rows| {
        rows.push(ArgumentRow {
            id,
            value: String::new(),
        });
    });
}

fn update_environment_name(set_rows: WriteSignal<Vec<EnvironmentRow>>, id: u32, name: String) {
    set_rows.update(|rows| {
        if let Some(row) = rows.iter_mut().find(|row| row.id == id) {
            row.name = name;
        }
    });
}

fn update_environment_value(set_rows: WriteSignal<Vec<EnvironmentRow>>, id: u32, value: String) {
    set_rows.update(|rows| {
        if let Some(row) = rows.iter_mut().find(|row| row.id == id) {
            row.value = value;
        }
    });
}

fn collect_options(
    build_env: ReadSignal<Vec<EnvironmentRow>>,
    build_args: ReadSignal<Vec<ArgumentRow>>,
    run_env: ReadSignal<Vec<EnvironmentRow>>,
    run_args: ReadSignal<Vec<ArgumentRow>>,
) -> Result<ProfileOptions, String> {
    Ok(ProfileOptions {
        build: ProcessOptions {
            env: collect_environment(build_env.get(), "compilation")?,
            args: collect_arguments(build_args.get()),
        },
        run: ProcessOptions {
            env: collect_environment(run_env.get(), "executable")?,
            args: collect_arguments(run_args.get()),
        },
    })
}

fn collect_environment(
    rows: Vec<EnvironmentRow>,
    label: &str,
) -> Result<BTreeMap<String, String>, String> {
    let mut environment = BTreeMap::new();
    for row in rows {
        let name = row.name.trim();
        if name.is_empty() && row.value.is_empty() {
            continue;
        }
        if name.is_empty() {
            return Err(format!(
                "The {label} environment contains a variable without a name."
            ));
        }
        if environment.insert(name.to_owned(), row.value).is_some() {
            return Err(format!(
                "The {label} variable '{name}' is defined more than once."
            ));
        }
    }
    Ok(environment)
}

fn collect_arguments(rows: Vec<ArgumentRow>) -> Vec<String> {
    rows.into_iter()
        .map(|row| row.value)
        .filter(|value| !value.is_empty())
        .collect()
}

fn js_error_to_string(error: JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "Unexpected launcher error".to_owned())
}
