use crate::{
    icons::{FolderIcon, TrashIcon},
    model::{LauncherCacheUsage, LauncherModpack, LauncherSettings, SettingsTab},
    tauri_bridge::{
        clear_launcher_cache, launcher_cache_usage, list_modpacks, select_folder,
        select_settings_file, update_launcher_backend, update_launcher_local_folders,
        update_launcher_path, update_launcher_theme,
    },
};
use leptos::prelude::*;
use patchwork_ui::THEMES;
use wasm_bindgen::JsValue;

#[derive(Clone, Debug, Eq, PartialEq)]
struct DynamicEntry {
    id: u32,
    value: String,
}

#[derive(Clone, Copy)]
enum PathPicker {
    Folder,
    SettingsFile,
}

impl DynamicEntry {
    fn empty(id: u32) -> Self {
        Self {
            id,
            value: String::new(),
        }
    }
}

#[component]
pub(crate) fn SettingsPage(
    active_theme: ReadSignal<&'static str>,
    set_active_theme: WriteSignal<&'static str>,
    settings: ReadSignal<Option<LauncherSettings>>,
    set_settings: WriteSignal<Option<LauncherSettings>>,
    set_modpacks: WriteSignal<Vec<LauncherModpack>>,
    set_selected_modpack: WriteSignal<usize>,
    on_backend_changed: Callback<()>,
) -> impl IntoView {
    let (settings_tab, set_settings_tab) = signal(SettingsTab::General);
    let (local_entries, set_local_entries) = signal(Vec::<DynamicEntry>::new());
    let (_local_next_id, set_local_next_id) = signal(0_u32);
    let (local_entries_initialized, set_local_entries_initialized) = signal(false);
    let (cache_usage, set_cache_usage) = signal(LauncherCacheUsage::default());
    let (cache_pending, set_cache_pending) = signal(None::<String>);
    let (cache_error, set_cache_error) = signal(None::<String>);
    let (cache_notice, set_cache_notice) = signal(None::<String>);
    let (measured_cache_paths, set_measured_cache_paths) = signal(None::<(String, String, String)>);

    Effect::new(move |_| {
        if local_entries_initialized.get() {
            return;
        }
        let Some(settings) = settings.get() else {
            return;
        };
        let mut entries = settings
            .local_folders
            .into_iter()
            .enumerate()
            .map(|(id, value)| DynamicEntry {
                id: id as u32,
                value,
            })
            .collect::<Vec<_>>();
        let next_id = entries.len() as u32;
        entries.push(DynamicEntry::empty(next_id));
        set_local_entries.set(entries);
        set_local_next_id.set(next_id + 1);
        set_local_entries_initialized.set(true);
    });

    Effect::new(move |_| {
        let Some(paths) = settings.with(|settings| {
            settings.as_ref().map(|settings| {
                (
                    settings.cargo_target_dir.clone(),
                    settings.build_cache.clone(),
                    settings.bin_cache.clone(),
                )
            })
        }) else {
            return;
        };
        if measured_cache_paths.get_untracked().as_ref() == Some(&paths) {
            return;
        }
        set_measured_cache_paths.set(Some(paths));
        leptos::task::spawn_local(async move {
            match launcher_cache_usage().await {
                Ok(usage) => set_cache_usage.set(usage),
                Err(error) => set_cache_error.set(Some(js_error_to_string(error))),
            }
        });
    });

    view! {
        <div class="settings-layout">
            <div class="settings-header">
                <div>
                    <p class="eyebrow">"Application settings"</p>
                    <h1>"Settings"</h1>
                </div>

                <div class="segmented-tabs" aria-label="Settings sections">
                    <button
                        type="button"
                        class=move || segmented_class(settings_tab.get() == SettingsTab::General)
                        on:click=move |_| set_settings_tab.set(SettingsTab::General)
                    >
                        "General"
                    </button>
                    <button
                        type="button"
                        class=move || segmented_class(settings_tab.get() == SettingsTab::Registries)
                        on:click=move |_| set_settings_tab.set(SettingsTab::Registries)
                    >
                        "Registries"
                    </button>
                    <button
                        type="button"
                        class=move || segmented_class(settings_tab.get() == SettingsTab::Installation)
                        on:click=move |_| set_settings_tab.set(SettingsTab::Installation)
                    >
                        "Installation"
                    </button>
                </div>
            </div>

            <section class=move || settings_panel_class(settings_tab.get() == SettingsTab::General)>
                <div class="settings-section">
                    <div class="section-heading">
                        <h2>"Launcher themes"</h2>
                        <span>"Visual style"</span>
                    </div>

                    <div class="theme-grid">
                        <For
                            each=move || THEMES
                            key=|theme| theme.0
                            children=move |(theme_id, theme_name): (&'static str, &'static str)| {
                                view! {
                                    <button
                                        type="button"
                                        class=move || theme_card_class(active_theme.get() == theme_id)
                                        data-preview-theme=theme_id
                                        on:click=move |_| {
                                            leptos::task::spawn_local(async move {
                                                if let Ok(updated_settings) = update_launcher_theme(theme_id).await {
                                                    set_settings.set(Some(updated_settings));
                                                    set_active_theme.set(theme_id);
                                                }
                                            });
                                        }
                                    >
                                        <span class="theme-preview" aria-hidden="true">
                                            <i></i>
                                            <i></i>
                                            <i></i>
                                            <i></i>
                                        </span>
                                        <strong>{theme_name}</strong>
                                    </button>
                                }
                            }
                        />
                    </div>
                </div>
            </section>

            <section class=move || settings_panel_class(settings_tab.get() == SettingsTab::Registries)>
                <div class="settings-section">
                    <div class="section-heading">
                        <h2>"Backend"</h2>
                        <span>"Patchwork service"</span>
                    </div>
                    <BackendField
                        value=move || settings.with(|settings| {
                            settings
                                .as_ref()
                                .map(|settings| settings.backend.clone())
                                .unwrap_or_default()
                        })
                        set_settings
                        on_backend_changed
                    />
                </div>

                <div class="settings-section">
                    <div class="section-heading">
                        <h2>"Local folders"</h2>
                        <span>"Folders scanned for local mods and modpacks"</span>
                    </div>
                    <LocalFolderFields
                        entries=local_entries
                        set_entries=set_local_entries
                        set_next_id=set_local_next_id
                        set_settings
                    />
                </div>
            </section>

            <section class=move || settings_panel_class(settings_tab.get() == SettingsTab::Installation)>
                <div class="settings-section installation-section">
                    <div class="section-heading">
                        <h2>"Installation"</h2>
                        <span>"Cache and build locations"</span>
                    </div>

                    <div class="path-list">
                        <PathInput
                            label="Cargo target dir"
                            description="Rust/Cargo project-manager build directory used while compiling generated projects."
                            field="cargo_target_dir"
                            picker=PathPicker::Folder
                            value=move || settings.with(|settings| settings.as_ref().map(|settings| settings.cargo_target_dir.clone()).unwrap_or_default())
                            set_settings
                            set_modpacks
                            set_selected_modpack
                        />
                        <PathInput
                            label="Mod cache"
                            description="Local cache containing downloaded mods."
                            field="mod_cache"
                            picker=PathPicker::Folder
                            value=move || settings.with(|settings| settings.as_ref().map(|settings| settings.mod_cache.clone()).unwrap_or_default())
                            set_settings
                            set_modpacks
                            set_selected_modpack
                        />
                        <PathInput
                            label="Modpacks cache"
                            description="Local cache containing downloaded modpacks. This is not shown in the launcher yet."
                            field="modpacks_cache"
                            picker=PathPicker::Folder
                            value=move || settings.with(|settings| settings.as_ref().map(|settings| settings.modpacks_cache.clone()).unwrap_or_default())
                            set_settings
                            set_modpacks
                            set_selected_modpack
                        />
                        <PathInput
                            label="Build cache"
                            description="Composed modpack output directory used before building/running."
                            field="build_cache"
                            picker=PathPicker::Folder
                            value=move || settings.with(|settings| settings.as_ref().map(|settings| settings.build_cache.clone()).unwrap_or_default())
                            set_settings
                            set_modpacks
                            set_selected_modpack
                        />
                        <PathInput
                            label="Binary cache"
                            description="Built profile executables stored outside Cargo target."
                            field="bin_cache"
                            picker=PathPicker::Folder
                            value=move || settings.with(|settings| settings.as_ref().map(|settings| settings.bin_cache.clone()).unwrap_or_default())
                            set_settings
                            set_modpacks
                            set_selected_modpack
                        />
                        <PathInput
                            label="Profiles"
                            description="User modpack profiles rendered on the launcher home page."
                            field="profiles_dir"
                            picker=PathPicker::Folder
                            value=move || settings.with(|settings| settings.as_ref().map(|settings| settings.profiles_dir.clone()).unwrap_or_default())
                            set_settings
                            set_modpacks
                            set_selected_modpack
                        />
                        <PathInput
                            label="Settings"
                            description="JSON file containing Patchwork launcher settings, including these paths and theme choices."
                            field="settings_file"
                            picker=PathPicker::SettingsFile
                            value=move || settings.with(|settings| settings.as_ref().map(|settings| settings.settings_file.clone()).unwrap_or_default())
                            set_settings
                            set_modpacks
                            set_selected_modpack
                        />
                    </div>

                    <div class="cache-actions">
                        <CacheAction
                            label="Clear cargo cache"
                            cache="cargo"
                            bytes=Signal::derive(move || cache_usage.get().cargo_cache_bytes)
                            cache_pending
                            set_cache_pending
                            set_cache_usage
                            set_cache_error
                            set_cache_notice
                        />
                        <CacheAction
                            label="Clear target cache"
                            cache="target"
                            bytes=Signal::derive(move || cache_usage.get().target_cache_bytes)
                            cache_pending
                            set_cache_pending
                            set_cache_usage
                            set_cache_error
                            set_cache_notice
                        />
                        <CacheAction
                            label="Clear build cache"
                            cache="build"
                            bytes=Signal::derive(move || cache_usage.get().build_cache_bytes)
                            cache_pending
                            set_cache_pending
                            set_cache_usage
                            set_cache_error
                            set_cache_notice
                        />
                        <CacheAction
                            label="Clear binary cache"
                            cache="bin"
                            bytes=Signal::derive(move || cache_usage.get().bin_cache_bytes)
                            cache_pending
                            set_cache_pending
                            set_cache_usage
                            set_cache_error
                            set_cache_notice
                        />
                    </div>
                    {move || cache_error.get().map(|error| view! {
                        <p class="cache-feedback error" role="alert">{error}</p>
                    })}
                    {move || cache_notice.get().map(|notice| view! {
                        <p class="cache-feedback success">{notice}</p>
                    })}
                </div>
            </section>
        </div>
    }
}

#[component]
fn CacheAction(
    label: &'static str,
    cache: &'static str,
    bytes: Signal<u64>,
    cache_pending: ReadSignal<Option<String>>,
    set_cache_pending: WriteSignal<Option<String>>,
    set_cache_usage: WriteSignal<LauncherCacheUsage>,
    set_cache_error: WriteSignal<Option<String>>,
    set_cache_notice: WriteSignal<Option<String>>,
) -> impl IntoView {
    let clear = move |_| {
        if cache_pending.get_untracked().is_some() {
            return;
        }
        let confirmed = web_sys::window()
            .and_then(|window| {
                window
                    .confirm_with_message(&format!("{label}? This cannot be undone."))
                    .ok()
            })
            .unwrap_or(false);
        if !confirmed {
            return;
        }

        set_cache_error.set(None);
        set_cache_notice.set(None);
        set_cache_pending.set(Some(cache.to_owned()));
        leptos::task::spawn_local(async move {
            match clear_launcher_cache(cache).await {
                Ok(usage) => {
                    set_cache_usage.set(usage);
                    set_cache_notice.set(Some(format!("{label} completed.")));
                }
                Err(error) => set_cache_error.set(Some(js_error_to_string(error))),
            }
            set_cache_pending.set(None);
        });
    };

    view! {
        <button
            type="button"
            class="danger-action cache-clear-action"
            disabled=move || cache_pending.get().is_some()
            on:click=clear
        >
            <TrashIcon />
            <span>{move || {
                if cache_pending.get().as_deref() == Some(cache) {
                    format!("Clearing... ({})", format_bytes(bytes.get()))
                } else {
                    format!("{label} ({})", format_bytes(bytes.get()))
                }
            }}</span>
        </button>
    }
}

#[component]
fn BackendField(
    value: impl Fn() -> String + Copy + Send + Sync + 'static,
    set_settings: WriteSignal<Option<LauncherSettings>>,
    on_backend_changed: Callback<()>,
) -> impl IntoView {
    let (error, set_error) = signal(None::<String>);
    view! {
        <label class="path-input registry-backend-input">
            <span class="path-label">
                <strong>"Service URL"</strong>
                <small>"Used for Browse, account, GitHub and publishing requests."</small>
            </span>
            <input
                type="url"
                inputmode="url"
                placeholder="http://127.0.0.1:8080"
                prop:value=value
                on:change=move |event| {
                    let backend = event_target_value(&event);
                    leptos::task::spawn_local(async move {
                        match update_launcher_backend(&backend).await {
                            Ok(updated) => {
                                set_error.set(None);
                                set_settings.set(Some(updated));
                                on_backend_changed.run(());
                            }
                            Err(error) => set_error.set(Some(js_error_to_string(error))),
                        }
                    });
                }
            />
            {move || error.get().map(|error| view! { <em class="field-error">{error}</em> })}
        </label>
    }
}

#[component]
fn LocalFolderFields(
    entries: ReadSignal<Vec<DynamicEntry>>,
    set_entries: WriteSignal<Vec<DynamicEntry>>,
    set_next_id: WriteSignal<u32>,
    set_settings: WriteSignal<Option<LauncherSettings>>,
) -> impl IntoView {
    view! {
        <div class="field-stack">
            <For
                each=move || entries.get()
                key=|entry| entry.id
                children=move |entry: DynamicEntry| {
                    let row_id = entry.id;
                    let value = entry.value;

                    view! {
                        <div class="input-row">
                            <input
                                type="text"
                                prop:value=value
                                placeholder="/path/to/local/mods"
                                on:input=move |event| {
                                    update_dynamic_entries(
                                        set_entries,
                                        set_next_id,
                                        row_id,
                                        event_target_value(&event),
                                    );
                                }
                                on:change=move |_| persist_local_folders(entries, set_settings)
                            />
                            <button
                                type="button"
                                class="icon-button"
                                aria-label="Select local folder"
                                on:click=move |_| {
                                    leptos::task::spawn_local(async move {
                                        if let Ok(Some(path)) = select_folder().await {
                                            update_dynamic_entries(
                                                set_entries,
                                                set_next_id,
                                                row_id,
                                                path,
                                            );
                                            persist_local_folders(entries, set_settings);
                                        }
                                    });
                                }
                            >
                                <FolderIcon />
                            </button>
                            <button
                                type="button"
                                class="icon-button danger"
                                aria-label="Remove local folder"
                                on:click=move |_| {
                                    remove_dynamic_entry(set_entries, set_next_id, row_id);
                                    persist_local_folders(entries, set_settings);
                                }
                            >
                                <TrashIcon />
                            </button>
                        </div>
                    }
                }
            />
        </div>
    }
}

#[component]
fn PathInput(
    label: &'static str,
    description: &'static str,
    field: &'static str,
    picker: PathPicker,
    value: impl Fn() -> String + Copy + Send + Sync + 'static,
    set_settings: WriteSignal<Option<LauncherSettings>>,
    set_modpacks: WriteSignal<Vec<LauncherModpack>>,
    set_selected_modpack: WriteSignal<usize>,
) -> impl IntoView {
    let (error, set_error) = signal(None::<String>);

    view! {
        <label class="path-input">
            <span class="path-label">
                <strong>{label}</strong>
                <small>{description}</small>
            </span>
            <div class="input-row">
                <input
                    type="text"
                    prop:value=value
                    on:change=move |event| {
                        let next_path = event_target_value(&event);
                        save_path_setting(
                            field,
                            next_path,
                            set_settings,
                            set_modpacks,
                            set_selected_modpack,
                            set_error,
                        );
                    }
                />
                <button
                    type="button"
                    class="icon-button"
                    aria-label="Select folder"
                    on:click=move |_| {
                        leptos::task::spawn_local(async move {
                            let selected = match picker {
                                PathPicker::Folder => select_folder().await,
                                PathPicker::SettingsFile => select_settings_file().await,
                            };
                            if let Ok(Some(selected_path)) = selected {
                                save_path_setting(
                                    field,
                                    selected_path,
                                    set_settings,
                                    set_modpacks,
                                    set_selected_modpack,
                                    set_error,
                                );
                            }
                        });
                    }
                >
                    <FolderIcon />
                </button>
            </div>
            {move || error.get().map(|error| view! { <em class="field-error">{error}</em> })}
        </label>
    }
}

fn save_path_setting(
    field: &'static str,
    value: String,
    set_settings: WriteSignal<Option<LauncherSettings>>,
    set_modpacks: WriteSignal<Vec<LauncherModpack>>,
    set_selected_modpack: WriteSignal<usize>,
    set_error: WriteSignal<Option<String>>,
) {
    leptos::task::spawn_local(async move {
        match update_launcher_path(field, &value).await {
            Ok(updated_settings) => {
                set_error.set(None);
                set_settings.set(Some(updated_settings));

                if field == "profiles_dir" {
                    match list_modpacks().await {
                        Ok(modpacks) => {
                            set_selected_modpack.set(0);
                            set_modpacks.set(modpacks);
                        }
                        Err(error) => set_error.set(Some(js_error_to_string(error))),
                    }
                }
            }
            Err(error) => set_error.set(Some(js_error_to_string(error))),
        }
    });
}

fn persist_local_folders(
    entries: ReadSignal<Vec<DynamicEntry>>,
    set_settings: WriteSignal<Option<LauncherSettings>>,
) {
    let folders = entries
        .get_untracked()
        .into_iter()
        .map(|entry| entry.value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    leptos::task::spawn_local(async move {
        if let Ok(updated) = update_launcher_local_folders(folders).await {
            set_settings.set(Some(updated));
        }
    });
}

fn update_dynamic_entries(
    set_entries: WriteSignal<Vec<DynamicEntry>>,
    set_next_id: WriteSignal<u32>,
    id: u32,
    value: String,
) {
    let mut next_value = Some(value);
    let mut should_add_empty = false;

    set_entries.update(|entries| {
        if let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) {
            if let Some(value) = next_value.take() {
                entry.value = value;
            }
        }

        should_add_empty = entries
            .last()
            .map(|entry| !entry.value.trim().is_empty())
            .unwrap_or(true);
    });

    if should_add_empty {
        append_empty_entry(set_entries, set_next_id);
    }
}

fn remove_dynamic_entry(
    set_entries: WriteSignal<Vec<DynamicEntry>>,
    set_next_id: WriteSignal<u32>,
    id: u32,
) {
    let mut should_add_empty = false;

    set_entries.update(|entries| {
        if entries.len() > 1 {
            entries.retain(|entry| entry.id != id);
        } else if let Some(entry) = entries.first_mut() {
            entry.value.clear();
        }

        should_add_empty =
            entries.is_empty() || entries.iter().all(|entry| !entry.value.trim().is_empty());
    });

    if should_add_empty {
        append_empty_entry(set_entries, set_next_id);
    }
}

fn append_empty_entry(set_entries: WriteSignal<Vec<DynamicEntry>>, set_next_id: WriteSignal<u32>) {
    let mut id = 0;

    set_next_id.update(|next_id| {
        id = *next_id;
        *next_id += 1;
    });

    set_entries.update(move |entries| entries.push(DynamicEntry::empty(id)));
}

fn segmented_class(is_active: bool) -> &'static str {
    if is_active {
        "segmented-tab active"
    } else {
        "segmented-tab"
    }
}

fn settings_panel_class(is_active: bool) -> &'static str {
    if is_active {
        "settings-panel active"
    } else {
        "settings-panel"
    }
}

fn theme_card_class(is_active: bool) -> &'static str {
    if is_active {
        "theme-card selected"
    } else {
        "theme-card"
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1_000.0;
    const MB: f64 = 1_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    let bytes = bytes as f64;
    if bytes >= GB {
        format!("{:.1} GB", bytes / GB)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes / MB)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes / KB)
    } else {
        format!("{} B", bytes as u64)
    }
}

fn js_error_to_string(error: JsValue) -> String {
    error
        .as_string()
        .unwrap_or_else(|| "Unexpected launcher error".to_string())
}
