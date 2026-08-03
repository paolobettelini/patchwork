use crate::model::DependencyPage;
use leptos::prelude::*;

#[component]
pub(super) fn DetailsPanel(page: ReadSignal<Option<DependencyPage>>) -> impl IntoView {
    view! {
        <div class="local-details-panel">
            {move || page.get().map(|page| {
                let kind = match page.kind.as_str() {
                    "profile" => "Profile",
                    "modpack" => "Modpack",
                    _ => "Mod",
                };
                let publication = page.published_at.unwrap_or_else(|| "-".to_owned());
                let publisher = page.publisher_name.unwrap_or_else(|| "-".to_owned());
                let repository = page.repository_url.unwrap_or_else(|| "-".to_owned());
                let repository_path = page.repository_path.unwrap_or_else(|| "-".to_owned());
                let source_commit = page.source_commit.unwrap_or_else(|| "-".to_owned());
                let source_tree_oid = page.source_tree_oid.unwrap_or_else(|| "-".to_owned());
                let manifest_sha256 = page.manifest_sha256.unwrap_or_else(|| "-".to_owned());
                view! {
                    <dl class="local-details-grid">
                        <div><dt>"Type"</dt><dd>{kind}</dd></div>
                        <div><dt>"ID"</dt><dd><code>{page.id}</code></dd></div>
                        <div><dt>"Version"</dt><dd>{page.version}</dd></div>
                        <div><dt>"Published"</dt><dd>{publication}</dd></div>
                        <div><dt>"Publisher"</dt><dd>{publisher}</dd></div>
                        <div><dt>"Downloads"</dt><dd>{format_downloads(page.downloads)}</dd></div>
                        <div class="wide"><dt>"Description"</dt><dd>{page.description}</dd></div>
                        <div class="wide"><dt>"Repository"</dt><dd><code>{repository}</code></dd></div>
                        <div><dt>"Repository path"</dt><dd><code>{repository_path}</code></dd></div>
                        <div><dt>"Commit"</dt><dd><code>{source_commit}</code></dd></div>
                        <div class="wide"><dt>"Source tree OID"</dt><dd><code>{source_tree_oid}</code></dd></div>
                        <div class="wide"><dt>"Manifest SHA-256"</dt><dd><code>{manifest_sha256}</code></dd></div>
                    </dl>
                }
            })}
        </div>
    }
}

pub(super) fn format_downloads(downloads: Option<i64>) -> String {
    let Some(downloads) = downloads else {
        return "-".to_owned();
    };
    if downloads >= 1_000_000 {
        format!("{:.1}M", downloads as f64 / 1_000_000.0)
    } else if downloads >= 1_000 {
        format!("{:.1}K", downloads as f64 / 1_000.0)
    } else {
        downloads.to_string()
    }
}
