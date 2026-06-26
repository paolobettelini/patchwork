use crate::icons::SearchIcon;
use leptos::prelude::*;

#[component]
pub(crate) fn BrowsePage() -> impl IntoView {
    view! {
        <div class="browse-layout">
            <section class="browse-card">
                <h1>"Browse"</h1>
                <div class="browse-search">
                    <SearchIcon />
                    <input type="search" placeholder="Search" aria-label="Search modpacks and mods" />
                </div>
            </section>
        </div>
    }
}
