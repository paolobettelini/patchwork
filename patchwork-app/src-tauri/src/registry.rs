use patchwork_registry_types::{
    RegistryPublishRequest, RegistryPublishResponse, RegistryScan, RegistryScanJobStarted,
    RegistryScanProgress, RegistryScanRequest,
};
use serde::de::DeserializeOwned;
use tauri::State;

use crate::auth::{authenticated_server_and_token, endpoint_url};
use crate::model::AppState;

#[tauri::command]
pub(crate) async fn registry_create_scan(
    state: State<'_, AppState>,
    input: RegistryScanRequest,
) -> Result<RegistryScan, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let url = endpoint_url(&server_url, "/registry/scans")?;
        let request = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .send_json(serde_json::to_value(input).map_err(|error| error.to_string())?);
        parse_json_response(request, "repository scan failed")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_start_scan(
    state: State<'_, AppState>,
    input: RegistryScanRequest,
) -> Result<RegistryScanJobStarted, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let url = endpoint_url(&server_url, "/registry/scan-jobs")?;
        let request = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .send_json(serde_json::to_value(input).map_err(|error| error.to_string())?);
        parse_json_response(request, "could not start repository scan")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_scan_progress(
    state: State<'_, AppState>,
    job_id: String,
) -> Result<RegistryScanProgress, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let url = endpoint_url(&server_url, &format!("/registry/scan-jobs/{job_id}"))?;
        let request = ureq::get(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call();
        parse_json_response(request, "could not load repository scan progress")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_get_scan(
    state: State<'_, AppState>,
    scan_id: String,
) -> Result<RegistryScan, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let url = endpoint_url(&server_url, &format!("/registry/scans/{scan_id}"))?;
        let request = ureq::get(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call();
        parse_json_response(request, "could not load registry scan")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_publish_scan(
    state: State<'_, AppState>,
    scan_id: String,
    input: RegistryPublishRequest,
) -> Result<RegistryPublishResponse, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let url = endpoint_url(&server_url, &format!("/registry/scans/{scan_id}/publish"))?;
        let request = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .set("Content-Type", "application/json")
            .send_json(serde_json::to_value(input).map_err(|error| error.to_string())?);
        parse_json_response(request, "registry publish failed")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_rescan_mod(
    state: State<'_, AppState>,
    mod_id: String,
) -> Result<RegistryScan, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let encoded_id: String = url::form_urlencoded::byte_serialize(mod_id.as_bytes()).collect();
        let url = endpoint_url(&server_url, &format!("/registry/mods/{encoded_id}/rescan"))?;
        let request = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call();
        parse_json_response(request, "registry rescan failed")
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub(crate) async fn registry_start_rescan(
    state: State<'_, AppState>,
    mod_id: String,
) -> Result<RegistryScanJobStarted, String> {
    let (server_url, token) = authenticated_server_and_token(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let encoded_id: String = url::form_urlencoded::byte_serialize(mod_id.as_bytes()).collect();
        let url = endpoint_url(
            &server_url,
            &format!("/registry/mods/{encoded_id}/rescan-job"),
        )?;
        let request = ureq::post(&url)
            .set("Authorization", &format!("Bearer {token}"))
            .call();
        parse_json_response(request, "could not start registry rescan")
    })
    .await
    .map_err(|error| error.to_string())?
}

fn parse_json_response<T: DeserializeOwned>(
    response: Result<ureq::Response, ureq::Error>,
    fallback: &str,
) -> Result<T, String> {
    match response {
        Ok(response) => response.into_json().map_err(|error| error.to_string()),
        Err(ureq::Error::Status(_, response)) => Err(response
            .into_string()
            .unwrap_or_else(|_| fallback.to_owned())),
        Err(error) => Err(error.to_string()),
    }
}
