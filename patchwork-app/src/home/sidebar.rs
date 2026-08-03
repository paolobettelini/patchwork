use crate::{
    icons::{PlusIcon, TrashIcon},
    model::LauncherModpack,
};
use leptos::prelude::*;

#[component]
pub(super) fn ProfilesSidebar(
    modpacks: ReadSignal<Vec<LauncherModpack>>,
    selected_modpack: ReadSignal<usize>,
    set_selected_modpack: WriteSignal<usize>,
    set_delete_candidate: WriteSignal<Option<LauncherModpack>>,
    set_show_create_modal: WriteSignal<bool>,
    set_create_error: WriteSignal<Option<String>>,
    set_new_modpack_id: WriteSignal<String>,
    set_new_modpack_name: WriteSignal<String>,
    set_new_modpack_description: WriteSignal<String>,
    set_new_modpack_icon: WriteSignal<Option<String>>,
    set_new_modpack_icon_preview: WriteSignal<Option<String>>,
) -> impl IntoView {
    view! {
        <aside class="profiles-panel">
            <div class="panel-title-row">
                <div>
                    <h2>"Modpacks"</h2>
                </div>
            </div>

            <button
                type="button"
                class="create-profile"
                on:click=move |_| {
                    set_create_error.set(None);
                    set_new_modpack_id.set(String::new());
                    set_new_modpack_name.set(String::new());
                    set_new_modpack_description.set(String::new());
                    set_new_modpack_icon.set(None);
                    set_new_modpack_icon_preview.set(None);
                    set_show_create_modal.set(true);
                }
            >
                <PlusIcon />
                <span>"Create new modpack"</span>
            </button>

            <div class="profile-scroll" aria-label="Modpacks">
                <Show
                    when=move || !modpacks.get().is_empty()
                    fallback=move || {
                        view! {
                            <div class="empty-modpacks">
                                <strong>"No modpacks yet"</strong>
                                <span>"Create one, import one, point Settings → Installation → Profiles to a folder with .toml files, or download one fromt the Browse tab."</span>
                            </div>
                        }
                    }
                >
                    <For
                        each=move || enumerated_modpacks(modpacks)
                        key=|(index, modpack)| {
                            format!("{index}:{}:{}", modpack.id, modpack.icon_version)
                        }
                        children=move |(index, modpack): (usize, LauncherModpack)| {
                            view! {
                                <ModpackButton
                                    index
                                    modpack
                                    selected_modpack
                                    set_selected_modpack
                                    set_delete_candidate
                                />
                            }
                        }
                    />
                </Show>
            </div>
        </aside>
    }
}

#[component]
fn ModpackButton(
    index: usize,
    modpack: LauncherModpack,
    selected_modpack: ReadSignal<usize>,
    set_selected_modpack: WriteSignal<usize>,
    set_delete_candidate: WriteSignal<Option<LauncherModpack>>,
) -> impl IntoView {
    let icon_src = modpack
        .icon_data_url
        .clone()
        .unwrap_or_else(|| "/logo.png".to_string());
    let delete_modpack = modpack.clone();

    view! {
        <div
            class=move || profile_class(selected_modpack.get() == index)
            style=format!("--profile-color: {}", modpack.accent)
            on:click=move |_| set_selected_modpack.set(index)
        >
            <span class="profile-icon" aria-hidden="true">
                <img src=icon_src alt="" />
            </span>
            <span class="profile-card-copy">
                <strong>{modpack.name}</strong>
                <span>{modpack.id.clone()}</span>
                <em>{format!("Dep: {}", modpack.dependencies)}</em>
            </span>
            <button
                type="button"
                class="profile-delete-button"
                title="Delete modpack"
                on:click=move |event| {
                    event.stop_propagation();
                    set_delete_candidate.set(Some(delete_modpack.clone()));
                }
            >
                <TrashIcon />
            </button>
        </div>
    }
}

fn enumerated_modpacks(
    modpacks: ReadSignal<Vec<LauncherModpack>>,
) -> Vec<(usize, LauncherModpack)> {
    modpacks.get().into_iter().enumerate().collect()
}

fn profile_class(is_active: bool) -> &'static str {
    if is_active {
        "profile-card selected"
    } else {
        "profile-card"
    }
}
