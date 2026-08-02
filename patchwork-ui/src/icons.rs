use leptos::prelude::*;

#[component]
pub fn HomeIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="m4 11 8-7 8 7"></path>
            <path d="M6 10v9h12v-9"></path>
            <path d="M10 19v-5h4v5"></path>
        </svg>
    }
}

#[component]
pub fn SearchIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M10.8 18.2a7.4 7.4 0 1 0 0-14.8 7.4 7.4 0 0 0 0 14.8Z"></path>
            <path d="m16.2 16.2 4.4 4.4"></path>
        </svg>
    }
}

#[component]
pub fn UploadIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M12 16V4"></path>
            <path d="m7 9 5-5 5 5"></path>
            <path d="M5 20h14"></path>
        </svg>
    }
}

#[component]
pub fn GithubIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M15 22v-4a4.8 4.8 0 0 0-1-3.3c3.2-.4 6.5-1.6 6.5-7A5.5 5.5 0 0 0 19 3.6 5.1 5.1 0 0 0 18.9 0s-1.2-.4-3.9 1.5a13.2 13.2 0 0 0-7 0C5.3-.4 4.1 0 4.1 0A5.1 5.1 0 0 0 4 3.6a5.5 5.5 0 0 0-1.5 3.8c0 5.4 3.3 6.6 6.5 7A4.8 4.8 0 0 0 8 18v4"></path>
            <path d="M8 19c-3 .9-3-1.5-4-2"></path>
        </svg>
    }
}

#[component]
pub fn ArrowRightToBracketIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4"></path>
            <path d="m10 17 5-5-5-5"></path>
            <path d="M15 12H3"></path>
        </svg>
    }
}

#[component]
pub fn UserIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M20 21a8 8 0 0 0-16 0"></path>
            <path d="M12 13a5 5 0 1 0 0-10 5 5 0 0 0 0 10Z"></path>
        </svg>
    }
}

#[component]
pub fn RefreshCwIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M21 12a9 9 0 0 0-15.6-6.1L3 8"></path>
            <path d="M3 3v5h5"></path>
            <path d="M3 12a9 9 0 0 0 15.6 6.1L21 16"></path>
            <path d="M16 16h5v5"></path>
        </svg>
    }
}
