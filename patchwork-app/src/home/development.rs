use std::collections::BTreeSet;

use crate::{
    icons::{FolderIcon, PlusIcon, TrashIcon},
    model::{DependencyPage, DevelopmentConfig, DevelopmentLibrary},
    tauri_bridge::{
        discover_development_checkout, load_dependency_page_for_profile, load_profile_development,
        select_development_modpack_file, select_folder, update_profile_development,
    },
};
use leptos::prelude::*;
use patchwork_ui::RefreshCwIcon;

use super::js_error_to_string;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct DevelopmentDraft {
    enabled: bool,
    exclusions: String,
    rows: Vec<DevelopmentRow>,
    next_id: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DevelopmentRow {
    key: u64,
    kind: RowKind,
    source: String,
    path: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowKind {
    Checkout,
    Mod,
    Modpack,
    Library,
}

impl DevelopmentDraft {
    fn from_config(config: DevelopmentConfig) -> Self {
        let mut draft = Self {
            enabled: config.enabled,
            exclusions: config.exclude.join("\n"),
            ..Self::default()
        };
        for checkout in config.checkouts {
            draft.push(RowKind::Checkout, checkout.git, checkout.path);
        }
        for (source, path) in config.mods {
            draft.push(RowKind::Mod, source, path);
        }
        for (source, path) in config.modpacks {
            draft.push(RowKind::Modpack, source, path);
        }
        for library in config.libraries {
            draft.push(RowKind::Library, library.git, library.path);
        }
        draft
    }

    fn push(&mut self, kind: RowKind, source: String, path: String) {
        self.rows.push(DevelopmentRow {
            key: self.next_id,
            kind,
            source,
            path,
        });
        self.next_id += 1;
    }

    fn config(&self) -> Result<DevelopmentConfig, String> {
        let mut config = DevelopmentConfig {
            enabled: self.enabled,
            exclude: self
                .exclusions
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect(),
            ..DevelopmentConfig::default()
        };
        let mut libraries = BTreeSet::new();
        for row in &self.rows {
            let source = row.source.trim();
            let path = row.path.trim();
            let label = match row.kind {
                RowKind::Checkout => "optional Git source URL",
                RowKind::Mod => "mod ID",
                RowKind::Modpack => "modpack ID",
                RowKind::Library => "Git source URL",
            };
            if source.is_empty() && row.kind != RowKind::Checkout {
                return Err(format!(
                    "Enter a {label} for every row, or remove the empty row."
                ));
            }
            if path.is_empty() {
                return Err(format!("Choose or enter a local path for {source}."));
            }
            if row.kind == RowKind::Checkout {
                if config.checkouts.iter().any(|c| c.path == path) {
                    return Err(format!("Duplicate checkout directory: {path}"));
                }
                config.checkouts.push(DevelopmentLibrary {
                    git: source.into(),
                    path: path.into(),
                });
                continue;
            }
            if row.kind == RowKind::Library {
                if !libraries.insert((source, path)) {
                    return Err(format!(
                        "Duplicate library checkout: {source} at {path}. Remove one of these rows."
                    ));
                }
                config.libraries.push(DevelopmentLibrary {
                    git: source.to_owned(),
                    path: path.to_owned(),
                });
                continue;
            }
            let entries = match row.kind {
                RowKind::Mod => &mut config.mods,
                RowKind::Modpack => &mut config.modpacks,
                RowKind::Library | RowKind::Checkout => unreachable!(),
            };
            if entries.insert(source.to_owned(), path.to_owned()).is_some() {
                return Err(format!(
                    "Duplicate ID: {source}. Keep one override per {label}."
                ));
            }
        }
        Ok(config)
    }
}

#[derive(Clone, Copy)]
struct EditorState {
    draft: RwSignal<DevelopmentDraft>,
    saved: RwSignal<Option<DevelopmentDraft>>,
    loading: RwSignal<bool>,
    saving: RwSignal<bool>,
    picking: RwSignal<bool>,
    error: RwSignal<Option<String>>,
    notice: RwSignal<Option<String>>,
    generation: RwSignal<u64>,
    profile: Memo<Option<String>>,
}

impl EditorState {
    fn current(self, generation: u64, profile: &str) -> bool {
        self.generation.try_get_untracked() == Some(generation)
            && self.profile.try_get_untracked().flatten().as_deref() == Some(profile)
    }

    fn busy(self) -> bool {
        self.loading.get() || self.saving.get() || self.picking.get() || self.saved.get().is_none()
    }

    fn changed(self) {
        self.error.set(None);
        self.notice.set(None);
    }
}

#[component]
pub(super) fn DevelopmentEditor(
    page: ReadSignal<Option<DependencyPage>>,
    set_page: WriteSignal<Option<DependencyPage>>,
) -> impl IntoView {
    let live_sources = expect_context::<crate::app::LiveSources>();
    let profile = Memo::new(move |_| {
        page.get()
            .filter(|page| page.editable_profile)
            .map(|page| page.id)
    });
    let state = EditorState {
        draft: RwSignal::new(DevelopmentDraft::default()),
        saved: RwSignal::new(None),
        loading: RwSignal::new(true),
        saving: RwSignal::new(false),
        picking: RwSignal::new(false),
        error: RwSignal::new(None),
        notice: RwSignal::new(None),
        generation: RwSignal::new(0),
        profile,
    };
    let reload = RwSignal::new(0_u64);
    on_cleanup(move || {
        state.generation.try_update(|value| *value += 1);
    });
    Effect::new(move |_| {
        let profile = profile.get();
        reload.get();
        state.generation.update(|value| *value += 1);
        let generation = state.generation.get_untracked();
        state.saved.set(None);
        state.draft.set(DevelopmentDraft::default());
        state.loading.set(true);
        state.saving.set(false);
        state.picking.set(false);
        state.changed();
        let Some(profile) = profile else {
            return;
        };
        leptos::task::spawn_local(async move {
            let result = load_profile_development(&profile).await;
            if !state.current(generation, &profile) {
                return;
            }
            match result {
                Ok(config) => {
                    let draft = DevelopmentDraft::from_config(config);
                    state.saved.set(Some(draft.clone()));
                    state.draft.set(draft);
                }
                Err(error) => state.error.set(Some(js_error_to_string(error))),
            }
            state.loading.set(false);
        });
    });

    let save = move |_| {
        if state.busy() {
            return;
        }
        let Some(profile) = profile.get_untracked() else {
            return;
        };
        let draft = state.draft.get_untracked();
        let config = match draft.config() {
            Ok(config) => config,
            Err(error) => {
                state.error.set(Some(error));
                return;
            }
        };
        let generation = state.generation.get_untracked();
        state.saving.set(true);
        state.changed();
        leptos::task::spawn_local(async move {
            let result = update_profile_development(&profile, config).await;
            if !state.current(generation, &profile) {
                return;
            }
            match result {
                Ok(()) => {
                    live_sources.profile_updates_revision.update(|v| *v += 1);
                    state.saved.set(Some(draft));
                    state.notice.set(Some("Local development saved.".into()));
                    let refreshed =
                        load_dependency_page_for_profile("profile", &profile, &profile).await;
                    if !state.current(generation, &profile) {
                        return;
                    }
                    match refreshed {
                        Ok(page) => set_page.set(Some(page)),
                        Err(error) => state.error.set(Some(format!("Saved, but could not refresh dependencies: {}. Reload the profile to retry.", js_error_to_string(error)))),
                    }
                }
                Err(error) => state.error.set(Some(js_error_to_string(error))),
            }
            state.saving.set(false);
        });
    };

    view! {
        <section class="development-editor" aria-label="Local development" aria-busy=move || state.loading.get() || state.saving.get()>
            <div class="development-heading">
                <div><h2>"Local development"</h2></div>
                <div class="development-actions">
                    <button type="button" class="secondary-action" disabled=move || state.busy() on:click=move |_| link_checkout(state)>
                        <FolderIcon />"Link checkout"
                    </button>
                    <button type="button" class="icon-button" title="Discard changes and reload" aria-label="Discard local development changes and reload"
                        disabled=move || state.loading.get() || state.saving.get() || state.picking.get()
                        on:click=move |_| reload.update(|value| *value += 1)><RefreshCwIcon /></button>
                    <button type="button" class="secondary-action" disabled=move || state.busy() || state.saved.get().as_ref() == Some(&state.draft.get()) on:click=save>
                        {move || if state.saving.get() { "Saving..." } else { "Save development" }}
                    </button>
                </div>
            </div>
            {move || state.error.get().map(|message| view! { <p class="profile-options-feedback error" role="alert">{message}</p> })}
            {move || state.notice.get().map(|message| view! { <p class="profile-options-feedback success" role="status">{message}</p> })}
            <Show when=move || !state.loading.get() fallback=|| view! { <div class="options-loading"><span class="button-spinner"></span>"Loading development settings..."</div> }>
                <fieldset class="development-fields" disabled=move || state.busy()>
                    <label class="development-enable">
                        <input class="profile-switch" type="checkbox" role="switch" prop:checked=move || state.draft.get().enabled
                            on:change=move |event| {
                                state.draft.update(|draft| draft.enabled = event_target_checked(&event));
                                state.changed();
                            } />
                        <span>"Enable local overrides"</span>
                    </label>
                    <DevelopmentRows kind=RowKind::Checkout title="Checkouts" source_label="Git source URL (optional)" path_label="Checkout directory" state />
                    <DevelopmentRows kind=RowKind::Mod title="Manual mod overrides" source_label="Mod ID" path_label="Crate directory" state />
                    <DevelopmentRows kind=RowKind::Modpack title="Manual modpack overrides" source_label="Modpack ID" path_label="Modpack TOML path" state />
                    <DevelopmentRows kind=RowKind::Library title="Libraries" source_label="Git source URL" path_label="Checkout directory" state />
                    <label class="development-exclusions">
                        <span>"Automatic source exclusions"</span>
                        <textarea rows="4" aria-label="Automatic source exclusions, one ID per line" placeholder={"mod-id\nmodpack/pack-id\nlibrary/package-name"}
                            prop:value=move || state.draft.get().exclusions
                            on:input=move |event| { state.draft.update(|draft| draft.exclusions = event_target_value(&event)); state.changed(); } />
                    </label>
                </fieldset>
            </Show>
        </section>
    }
}

#[component]
fn DevelopmentRows(
    kind: RowKind,
    title: &'static str,
    source_label: &'static str,
    path_label: &'static str,
    state: EditorState,
) -> impl IntoView {
    view! {
        <details class="development-group" open=true>
            <summary>{title}<span>{move || state.draft.get().rows.iter().filter(|r| r.kind == kind).count()}</span></summary>
            <div class="options-field-heading">
                <button type="button" class="icon-button" title=format!("Add {source_label}") aria-label=format!("Add {source_label}")
                    on:click=move |_| {
                        state.draft.update(|draft| draft.push(kind, String::new(), String::new()));
                        state.changed();
                    }><PlusIcon /></button>
                <button type="button" class="icon-button danger" title=format!("Remove all {title} entries (files are kept)") aria-label=format!("Remove all {title} entries")
                    disabled=move || !state.draft.get().rows.iter().any(|row| row.kind == kind)
                    on:click=move |_| { state.draft.update(|draft| draft.rows.retain(|row| row.kind != kind)); state.changed(); }><TrashIcon /></button>
            </div>
            <Show when=move || !state.draft.get().rows.iter().any(|row| row.kind == kind)>
                <p class="development-empty">"No local overrides"</p>
            </Show>
            <For each=move || { state.draft.get().rows.into_iter().filter(|row| row.kind == kind).collect::<Vec<_>>() } key=|row| row.key
                children=move |row| {
                    let key = row.key;
                    view! {
                        <div class="development-row">
                            <input type="text" aria-label=source_label placeholder=source_label spellcheck="false" prop:value=row.source
                                on:input=move |event| {
                                    state.draft.update(|draft| { if let Some(row) = draft.rows.iter_mut().find(|row| row.key == key) { row.source = event_target_value(&event); } });
                                    state.changed();
                                } />
                            <input type="text" aria-label=path_label placeholder=path_label spellcheck="false"
                                prop:value=move || state.draft.get().rows.iter().find(|row| row.key == key).map(|row| row.path.clone()).unwrap_or_default()
                                on:input=move |event| {
                                    state.draft.update(|draft| { if let Some(row) = draft.rows.iter_mut().find(|row| row.key == key) { row.path = event_target_value(&event); } });
                                    state.changed();
                                } />
                            <button type="button" class="icon-button" title=format!("Choose {path_label}") aria-label=format!("Choose {path_label}")
                                on:click=move |_| pick_path(state, kind, key)><FolderIcon /></button>
                            <button type="button" class="icon-button danger" title="Remove override" aria-label=format!("Remove {source_label} override")
                                on:click=move |_| {
                                    state.draft.update(|draft| draft.rows.retain(|row| row.key != key));
                                    state.changed();
                                }><TrashIcon /></button>
                        </div>
                    }
                }
            />
        </details>
    }
}

fn pick_path(state: EditorState, kind: RowKind, key: u64) {
    if state.busy() {
        return;
    }
    let Some(profile) = state.profile.get_untracked() else {
        return;
    };
    let generation = state.generation.get_untracked();
    state.picking.set(true);
    leptos::task::spawn_local(async move {
        let result = if kind == RowKind::Modpack {
            select_development_modpack_file().await
        } else {
            select_folder().await
        };
        if !state.current(generation, &profile) {
            return;
        }
        match result {
            Ok(Some(path)) => {
                state.draft.update(|draft| {
                    if let Some(row) = draft.rows.iter_mut().find(|row| row.key == key) {
                        row.path = path;
                    }
                });
                state.changed();
            }
            Ok(None) => {}
            Err(error) => state.error.set(Some(js_error_to_string(error))),
        }
        state.picking.set(false);
    });
}

fn link_checkout(state: EditorState) {
    if state.busy() {
        return;
    }
    let Some(profile) = state.profile.get_untracked() else {
        return;
    };
    let generation = state.generation.get_untracked();
    state.picking.set(true);
    state.changed();
    leptos::task::spawn_local(async move {
        let result = match select_folder().await {
            Ok(Some(path)) => discover_development_checkout(&profile, &path)
                .await
                .map(Some),
            Ok(None) => Ok(None),
            Err(error) => Err(error),
        };
        if !state.current(generation, &profile) {
            return;
        }
        match result {
            Ok(Some(config)) => {
                let additions = DevelopmentDraft::from_config(config);
                state.draft.update(|draft| {
                    draft.enabled = true;
                    for row in additions.rows {
                        if let Some(existing) = draft.rows.iter_mut().find(|r| {
                            r.kind == row.kind
                                && (if r.kind == RowKind::Checkout {
                                    r.path == row.path
                                } else {
                                    r.source == row.source
                                        && (r.kind != RowKind::Library || r.path == row.path)
                                })
                        }) {
                            existing.path = row.path;
                            if row.kind == RowKind::Checkout {
                                existing.source = row.source;
                            }
                        } else {
                            draft.push(row.kind, row.source, row.path);
                        }
                    }
                });
                state.notice.set(Some(
                    "Checkout linked. Review the path and Git URL, then save.".into(),
                ));
            }
            Ok(None) => {}
            Err(error) => state.error.set(Some(js_error_to_string(error))),
        }
        state.picking.set(false);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn checkout_and_exclusions_round_trip_without_creating_manual_mods() {
        let config = DevelopmentConfig {
            enabled: true,
            checkouts: vec![DevelopmentLibrary {
                path: "/code/game".into(),
                git: String::new(),
            }],
            exclude: vec![
                "remote-mod".into(),
                "modpack/remote-pack".into(),
                "library/helper".into(),
            ],
            ..Default::default()
        };
        let draft = DevelopmentDraft::from_config(config.clone());
        assert_eq!(draft.config().unwrap(), config);
        assert!(draft.config().unwrap().mods.is_empty());
    }

    #[test]
    fn allows_distinct_checkouts_of_the_same_git_source() {
        let mut draft = DevelopmentDraft::default();
        draft.push(
            RowKind::Library,
            "https://example.com/libs.git".into(),
            "/code/first".into(),
        );
        draft.push(
            RowKind::Library,
            "https://example.com/libs.git".into(),
            "/code/second".into(),
        );
        let config = draft.config().unwrap();
        assert_eq!(config.libraries.len(), 2);
        assert_eq!(
            DevelopmentDraft::from_config(config.clone())
                .config()
                .unwrap(),
            config
        );
        draft.push(
            RowKind::Library,
            " https://example.com/libs.git ".into(),
            " /code/first ".into(),
        );
        assert!(
            draft
                .config()
                .unwrap_err()
                .contains("Duplicate library checkout")
        );
    }

    #[test]
    fn config_round_trip_preserves_disabled_overrides() {
        let config = DevelopmentConfig {
            enabled: false,
            checkouts: Vec::new(),
            exclude: Vec::new(),
            mods: BTreeMap::from([("example".into(), "/code/example".into())]),
            modpacks: BTreeMap::from([("pack".into(), "/code/pack.toml".into())]),
            libraries: vec![DevelopmentLibrary {
                git: "https://example.com/lib.git".into(),
                path: "/code/lib".into(),
            }],
        };
        assert_eq!(
            DevelopmentDraft::from_config(config.clone())
                .config()
                .unwrap(),
            config
        );
    }

    #[test]
    fn rejects_duplicate_sources_and_incomplete_rows() {
        let mut draft = DevelopmentDraft::default();
        draft.push(RowKind::Mod, "example".into(), "".into());
        assert!(draft.config().is_err());
        draft.rows[0].path = "/code/mod".into();
        draft.push(RowKind::Mod, " example ".into(), "/code/other".into());
        assert!(draft.config().unwrap_err().contains("Duplicate ID"));
        draft.rows[1].kind = RowKind::Modpack;
        assert!(draft.config().is_ok());
    }
}
