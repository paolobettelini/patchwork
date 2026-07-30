use leptos::prelude::*;

#[component]
pub(crate) fn HomeIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="m4 11 8-7 8 7"></path>
            <path d="M6 10v9h12v-9"></path>
            <path d="M10 19v-5h4v5"></path>
        </svg>
    }
}

#[component]
pub(crate) fn GearIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <circle cx="12" cy="12" r="3.2"></circle>
            <path d="M12.2 2h-.4a2 2 0 0 0-2 2v.2a2 2 0 0 1-1 1.7l-.4.2a2 2 0 0 1-2 0l-.2-.1a2 2 0 0 0-2.7.7l-.2.4a2 2 0 0 0 .7 2.7l.2.1a2 2 0 0 1 1 1.8v.6a2 2 0 0 1-1 1.8l-.2.1a2 2 0 0 0-.7 2.7l.2.4a2 2 0 0 0 2.7.7l.2-.1a2 2 0 0 1 2 0l.4.2a2 2 0 0 1 1 1.7v.2a2 2 0 0 0 2 2h.4a2 2 0 0 0 2-2v-.2a2 2 0 0 1 1-1.7l.4-.2a2 2 0 0 1 2 0l.2.1a2 2 0 0 0 2.7-.7l.2-.4a2 2 0 0 0-.7-2.7l-.2-.1a2 2 0 0 1-1-1.8v-.6a2 2 0 0 1 1-1.8l.2-.1a2 2 0 0 0 .7-2.7l-.2-.4a2 2 0 0 0-2.7-.7l-.2.1a2 2 0 0 1-2 0l-.4-.2a2 2 0 0 1-1-1.7V4a2 2 0 0 0-2-2Z"></path>
        </svg>
    }
}

#[component]
pub(crate) fn SearchIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M10.8 18.2a7.4 7.4 0 1 0 0-14.8 7.4 7.4 0 0 0 0 14.8Z"></path>
            <path d="m16.2 16.2 4.4 4.4"></path>
        </svg>
    }
}

#[component]
pub(crate) fn ArrowRightToBracketIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M15 3h4a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2h-4"></path>
            <path d="m10 17 5-5-5-5"></path>
            <path d="M15 12H3"></path>
        </svg>
    }
}

#[component]
pub(crate) fn ArrowLeftIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M19 12H5"></path>
            <path d="m12 5-7 7 7 7"></path>
        </svg>
    }
}

#[component]
pub(crate) fn PlusIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M12 5v14"></path>
            <path d="M5 12h14"></path>
        </svg>
    }
}

#[component]
pub(crate) fn TrashIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M4 7h16"></path>
            <path d="M10 11v6"></path>
            <path d="M14 11v6"></path>
            <path d="M6 7l1 13h10l1-13"></path>
            <path d="M9 7V4h6v3"></path>
        </svg>
    }
}

#[component]
pub(crate) fn DownloadIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M12 4v11"></path>
            <path d="m7 10 5 5 5-5"></path>
            <path d="M5 20h14"></path>
        </svg>
    }
}

#[component]
pub(crate) fn PlayIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M8 5v14l11-7Z"></path>
        </svg>
    }
}

#[component]
pub(crate) fn StopIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M7 7h10v10H7Z"></path>
        </svg>
    }
}

#[component]
pub(crate) fn FolderIcon() -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d="M3 7.5A2.5 2.5 0 0 1 5.5 5H10l2 2h6.5A2.5 2.5 0 0 1 21 9.5v7A2.5 2.5 0 0 1 18.5 19h-13A2.5 2.5 0 0 1 3 16.5v-9Z"></path>
        </svg>
    }
}
