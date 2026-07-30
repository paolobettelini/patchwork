use crate::model::{
    DependencyPage, LauncherModpack, LauncherSettings, PatchworkConsoleEvent, PatchworkTaskStatus,
    SelectedIconFile,
};
use serde::{Serialize, de::DeserializeOwned};
use wasm_bindgen::{JsCast, prelude::*};
use wasm_bindgen_futures::JsFuture;

const PATCHWORK_CONSOLE_EVENT: &str = "patchwork-console";

pub(crate) async fn select_folder() -> Result<Option<String>, JsValue> {
    invoke("select_folder", &()).await
}

pub(crate) async fn select_settings_file() -> Result<Option<String>, JsValue> {
    invoke("select_settings_file", &()).await
}

pub(crate) async fn select_icon_file() -> Result<Option<SelectedIconFile>, JsValue> {
    invoke("select_icon_file", &()).await
}

pub(crate) async fn load_launcher_settings() -> Result<LauncherSettings, JsValue> {
    invoke("load_launcher_settings", &()).await
}

pub(crate) async fn update_launcher_path(
    field: &str,
    value: &str,
) -> Result<LauncherSettings, JsValue> {
    #[derive(Serialize)]
    struct Args<'a> {
        field: &'a str,
        value: &'a str,
    }

    invoke("update_launcher_path", &Args { field, value }).await
}

pub(crate) async fn update_launcher_theme(theme: &str) -> Result<LauncherSettings, JsValue> {
    #[derive(Serialize)]
    struct Args<'a> {
        theme: &'a str,
    }

    invoke("update_launcher_theme", &Args { theme }).await
}

pub(crate) async fn list_modpacks() -> Result<Vec<LauncherModpack>, JsValue> {
    invoke("list_modpacks", &()).await
}

pub(crate) async fn update_profile_metadata(
    profile_id: &str,
    name: Option<&str>,
    description: Option<&str>,
) -> Result<DependencyPage, JsValue> {
    #[derive(Serialize)]
    struct Args<'a> {
        #[serde(rename = "profileId")]
        profile_id: &'a str,
        name: Option<&'a str>,
        description: Option<&'a str>,
    }

    invoke(
        "update_profile_metadata",
        &Args {
            profile_id,
            name,
            description,
        },
    )
    .await
}

pub(crate) async fn create_modpack(
    id: &str,
    name: &str,
    description: &str,
    icon_path: Option<&str>,
) -> Result<LauncherModpack, JsValue> {
    #[derive(Serialize)]
    struct Args<'a> {
        id: &'a str,
        name: &'a str,
        description: &'a str,
        #[serde(rename = "iconPath")]
        icon_path: Option<&'a str>,
    }

    invoke(
        "create_modpack",
        &Args {
            id,
            name,
            description,
            icon_path,
        },
    )
    .await
}

pub(crate) async fn import_modpack() -> Result<Option<LauncherModpack>, JsValue> {
    invoke("import_modpack", &()).await
}

pub(crate) async fn delete_modpack(modpack_id: &str) -> Result<(), JsValue> {
    #[derive(Serialize)]
    struct Args<'a> {
        #[serde(rename = "modpackId")]
        modpack_id: &'a str,
    }

    invoke("delete_modpack", &Args { modpack_id }).await
}

pub(crate) async fn select_modpack_icon(
    modpack_id: &str,
) -> Result<Option<LauncherModpack>, JsValue> {
    #[derive(Serialize)]
    struct Args<'a> {
        #[serde(rename = "modpackId")]
        modpack_id: &'a str,
    }

    invoke("select_modpack_icon", &Args { modpack_id }).await
}

pub(crate) async fn load_dependency_page(kind: &str, id: &str) -> Result<DependencyPage, JsValue> {
    #[derive(Serialize)]
    struct Args<'a> {
        kind: &'a str,
        id: &'a str,
    }

    invoke("load_dependency_page", &Args { kind, id }).await
}

pub(crate) async fn toggle_profile_ignore(
    profile_id: &str,
    mod_id: &str,
) -> Result<DependencyPage, JsValue> {
    #[derive(Serialize)]
    struct Args<'a> {
        #[serde(rename = "profileId")]
        profile_id: &'a str,
        #[serde(rename = "modId")]
        mod_id: &'a str,
    }

    invoke("toggle_profile_ignore", &Args { profile_id, mod_id }).await
}

pub(crate) async fn patchwork_task_status(
    profile_id: &str,
    build_mode: &str,
    include_output: bool,
) -> Result<PatchworkTaskStatus, JsValue> {
    #[derive(Serialize)]
    struct Args<'a> {
        #[serde(rename = "profileId")]
        profile_id: &'a str,
        #[serde(rename = "buildMode")]
        build_mode: &'a str,
        #[serde(rename = "includeOutput")]
        include_output: bool,
    }

    invoke(
        "patchwork_task_status",
        &Args {
            profile_id,
            build_mode,
            include_output,
        },
    )
    .await
}

pub(crate) async fn start_patchwork_action(
    profile_id: &str,
    action: &str,
    build_mode: &str,
) -> Result<bool, JsValue> {
    #[derive(Serialize)]
    struct Args<'a> {
        #[serde(rename = "profileId")]
        profile_id: &'a str,
        action: &'a str,
        #[serde(rename = "buildMode")]
        build_mode: &'a str,
    }

    invoke(
        "start_patchwork_action",
        &Args {
            profile_id,
            action,
            build_mode,
        },
    )
    .await
}

pub(crate) async fn stop_patchwork_action(profile_id: &str) -> Result<bool, JsValue> {
    #[derive(Serialize)]
    struct Args<'a> {
        #[serde(rename = "profileId")]
        profile_id: &'a str,
    }

    invoke("stop_patchwork_action", &Args { profile_id }).await
}

pub(crate) fn listen_patchwork_console<F>(mut callback: F) -> Result<(), JsValue>
where
    F: FnMut(PatchworkConsoleEvent) + 'static,
{
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window is not available"))?;
    let tauri = js_sys::Reflect::get(&window, &JsValue::from_str("__TAURI__"))?;
    let event_api = js_sys::Reflect::get(&tauri, &JsValue::from_str("event"))?;
    let listen = js_sys::Reflect::get(&event_api, &JsValue::from_str("listen"))?
        .dyn_into::<js_sys::Function>()?;

    let closure = Closure::wrap(Box::new(move |event: JsValue| {
        let Ok(payload) = js_sys::Reflect::get(&event, &JsValue::from_str("payload")) else {
            return;
        };
        if let Ok(event) = serde_wasm_bindgen::from_value::<PatchworkConsoleEvent>(payload) {
            callback(event);
        }
    }) as Box<dyn FnMut(JsValue)>);

    let _ = listen.call2(
        &event_api,
        &JsValue::from_str(PATCHWORK_CONSOLE_EVENT),
        closure.as_ref().unchecked_ref(),
    )?;
    closure.forget();
    Ok(())
}

async fn invoke<T, A>(command: &str, args: &A) -> Result<T, JsValue>
where
    T: DeserializeOwned,
    A: Serialize + ?Sized,
{
    let window = web_sys::window().ok_or_else(|| JsValue::from_str("window is not available"))?;
    let tauri = js_sys::Reflect::get(&window, &JsValue::from_str("__TAURI__"))?;
    let core = js_sys::Reflect::get(&tauri, &JsValue::from_str("core"))?;
    let invoke = js_sys::Reflect::get(&core, &JsValue::from_str("invoke"))?
        .dyn_into::<js_sys::Function>()?;
    let args = serde_wasm_bindgen::to_value(args)?;
    let args = if args.is_null() || args.is_undefined() {
        js_sys::Object::new().into()
    } else {
        args
    };
    let promise = invoke
        .call2(&JsValue::NULL, &JsValue::from_str(command), &args)?
        .dyn_into::<js_sys::Promise>()?;
    let value = JsFuture::from(promise).await?;

    serde_wasm_bindgen::from_value(value).map_err(|error| JsValue::from_str(&error.to_string()))
}
