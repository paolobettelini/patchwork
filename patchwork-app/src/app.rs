use crate::{
    browse::BrowsePage,
    home::HomePage,
    icons::{ArrowRightToBracketIcon, GearIcon, HomeIcon, SearchIcon},
    model::{AppTab, LauncherModpack, LauncherSettings},
    settings::SettingsPage,
    tauri_bridge::{list_modpacks, load_launcher_settings},
};
use leptos::prelude::*;

#[component]
pub(crate) fn App() -> impl IntoView {
    let (active_tab, set_active_tab) = signal(AppTab::Home);
    let (selected_modpack, set_selected_modpack) = signal(0_usize);
    let (active_theme, set_active_theme) = signal("dark");
    let (settings, set_settings) = signal(None::<LauncherSettings>);
    let (modpacks, set_modpacks) = signal(Vec::<LauncherModpack>::new());

    leptos::task::spawn_local(async move {
        if let Ok(loaded_settings) = load_launcher_settings().await {
            set_active_theme.set(theme_id_or_default(&loaded_settings.theme));
            set_settings.set(Some(loaded_settings));
        }

        if let Ok(loaded_modpacks) = list_modpacks().await {
            set_selected_modpack.set(0);
            set_modpacks.set(loaded_modpacks);
        }
    });

    view! {
        <div class="app-shell" data-theme=move || active_theme.get()>
            <TopBar active_tab set_active_tab />

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
                    <BrowsePage />
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
fn TopBar(active_tab: ReadSignal<AppTab>, set_active_tab: WriteSignal<AppTab>) -> impl IntoView {
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
                    class=move || top_tab_class(active_tab.get() == AppTab::Settings)
                    on:click=move |_| set_active_tab.set(AppTab::Settings)
                >
                    <GearIcon />
                    <span>"Settings"</span>
                </button>
            </nav>

            <div class="topbar-actions">
                <button type="button" class="sign-in-button">
                    <span>"Sign in"</span>
                    <ArrowRightToBracketIcon />
                </button>
            </div>
        </header>
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
