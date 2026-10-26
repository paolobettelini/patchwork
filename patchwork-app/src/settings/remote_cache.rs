use crate::{
    cache_settings::{CacheAuthentication, CacheCredentials, CacheMode, RemoteCacheSettings},
    model::LauncherSettings,
    tauri_bridge::{remote_cache_credentials_status, save_remote_cache, test_remote_cache},
};
use leptos::prelude::*;

#[component]
pub(super) fn RemoteCachePanel(
    settings: ReadSignal<Option<LauncherSettings>>,
    set_settings: WriteSignal<Option<LauncherSettings>>,
) -> impl IntoView {
    let draft = RwSignal::new(RemoteCacheSettings::default());
    let initialized = RwSignal::new(false);
    let access_key = RwSignal::new(String::new());
    let secret_key = RwSignal::new(String::new());
    let saved_credentials = RwSignal::new(false);
    let pending = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let notice = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        if initialized.get_untracked() {
            return;
        }
        if let Some(settings) = settings.get() {
            draft.set(settings.remote_cache);
            initialized.set(true);
            leptos::task::spawn_local(async move {
                if let Ok(saved) = remote_cache_credentials_status().await {
                    saved_credentials.set(saved);
                }
            });
        }
    });
    let credentials = move || {
        if access_key.get_untracked().is_empty() && secret_key.get_untracked().is_empty() {
            None
        } else {
            Some(CacheCredentials {
                access_key: access_key.get_untracked(),
                secret_key: secret_key.get_untracked(),
            })
        }
    };
    let save = move |_| {
        pending.set(true);
        error.set(None);
        notice.set(None);
        let config = draft.get_untracked();
        let credentials = credentials();
        leptos::task::spawn_local(async move {
            match save_remote_cache(config, credentials).await {
                Ok(updated) => {
                    set_settings.set(Some(updated));
                    access_key.set(String::new());
                    secret_key.set(String::new());
                    saved_credentials.set(remote_cache_credentials_status().await.unwrap_or(false));
                    notice.set(Some("Remote cache settings saved.".into()));
                }
                Err(e) => error.set(Some(super::js_error_to_string(e))),
            }
            pending.set(false);
        });
    };
    let test = move |_| {
        pending.set(true);
        error.set(None);
        notice.set(None);
        let config = draft.get_untracked();
        let credentials = credentials();
        leptos::task::spawn_local(async move {
            match test_remote_cache(config, credentials).await {
                Ok(message) => notice.set(Some(message)),
                Err(e) => error.set(Some(super::js_error_to_string(e))),
            }
            pending.set(false);
        });
    };
    view! {
        <div class="settings-section remote-cache-section">
            <div class="section-heading"><h2>"Remote cache"</h2></div>
            <fieldset disabled=move || pending.get() || !initialized.get()>
                <label class="remote-cache-toggle">
                    <input type="checkbox" role="switch" prop:checked=move || draft.get().enabled
                        on:change=move |e| draft.update(|c| c.enabled = event_target_checked(&e)) />
                    <span>"Enable remote cache"</span>
                </label>
                <div class="remote-cache-fields">
                    <label class="path-input"><span>"Endpoint"</span>
                        <input type="url" placeholder="https://cache.example.com" prop:value=move || draft.get().endpoint
                            on:input=move |e| draft.update(|c| c.endpoint = event_target_value(&e)) />
                    </label>
                    <label class="path-input"><span>"Bucket"</span>
                        <input type="text" prop:value=move || draft.get().bucket
                            on:input=move |e| draft.update(|c| c.bucket = event_target_value(&e)) />
                    </label>
                    <label class="path-input"><span>"Cache mode"</span>
                        <select prop:value=move || if draft.get().mode == CacheMode::ReadOnly { "read" } else { "write" }
                            on:change=move |e| draft.update(|c| c.mode = if event_target_value(&e) == "write" { CacheMode::ReadWrite } else { CacheMode::ReadOnly })>
                            <option value="read">"Read only"</option><option value="write">"Read write"</option>
                        </select>
                    </label>
                    <label class="path-input"><span>"Authentication"</span>
                        <select prop:value=move || if draft.get().authentication == CacheAuthentication::None { "none" } else { "key" }
                            on:change=move |e| draft.update(|c| c.authentication = if event_target_value(&e) == "key" { CacheAuthentication::AccessKey } else { CacheAuthentication::None })>
                            <option value="none">"None"</option><option value="key">"Access key / Secret key"</option>
                        </select>
                    </label>
                </div>
                <Show when=move || draft.get().authentication == CacheAuthentication::AccessKey>
                    <div class="remote-cache-fields">
                        <label class="path-input"><span>"Access key"</span>
                            <input type="password" autocomplete="off" prop:value=move || access_key.get()
                                placeholder=move || if saved_credentials.get() { "Stored; leave blank to keep" } else { "Access key" }
                                on:input=move |e| access_key.set(event_target_value(&e)) />
                        </label>
                        <label class="path-input"><span>"Secret key"</span>
                            <input type="password" autocomplete="new-password" prop:value=move || secret_key.get()
                                placeholder=move || if saved_credentials.get() { "Stored; leave blank to keep" } else { "Secret key" }
                                on:input=move |e| secret_key.set(event_target_value(&e)) />
                        </label>
                    </div>
                </Show>
                <details class="remote-cache-advanced">
                    <summary>"Advanced"</summary>
                    <div class="remote-cache-fields">
                        <label class="path-input"><span>"Region"</span>
                            <input type="text" prop:value=move || draft.get().region
                                on:input=move |e| draft.update(|c| c.region = event_target_value(&e)) />
                        </label>
                        <label class="path-input"><span>"Key prefix"</span>
                            <input type="text" prop:value=move || draft.get().key_prefix
                                on:input=move |e| draft.update(|c| c.key_prefix = event_target_value(&e)) />
                        </label>
                        <label class="path-input"><span>"Base directories"</span><input type="text" value="Automatic" readonly /></label>
                    </div>
                    <label class="remote-cache-toggle">
                        <input type="checkbox" role="switch" prop:checked=move || draft.get().local_cache
                            on:change=move |e| draft.update(|c| c.local_cache = event_target_checked(&e)) />
                        <span>"Use local cache in addition to remote cache"</span>
                    </label>
                </details>
                <div class="cache-actions">
                    <button type="button" class="secondary-action" on:click=save>"Save settings"</button>
                    <button type="button" class="secondary-action" on:click=test>"Test connection"</button>
                </div>
            </fieldset>
            <Show when=move || pending.get()><p role="status">"Working..."</p></Show>
            {move || error.get().map(|message| view! { <p class="cache-feedback error" role="alert">{message}</p> })}
            {move || notice.get().map(|message| view! { <p class="cache-feedback success" role="status">{message}</p> })}
        </div>
    }
}
