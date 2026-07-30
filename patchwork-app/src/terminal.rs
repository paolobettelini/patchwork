use crate::model::DependencyPage;
use leptos::{html::Div, prelude::*};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = patchworkCreateTerminal)]
    fn js_create_terminal(element: JsValue, profile_id: &str, build_mode: &str) -> JsValue;

    #[wasm_bindgen(js_namespace = window, js_name = patchworkSetTerminalProfile)]
    fn js_set_terminal_profile(handle: &JsValue, profile_id: &str, build_mode: &str);

    #[wasm_bindgen(js_namespace = window, js_name = patchworkFitTerminal)]
    fn js_fit_terminal(handle: &JsValue);

    #[wasm_bindgen(js_namespace = window, js_name = patchworkDisposeTerminal)]
    fn js_dispose_terminal(handle: &JsValue);
}

#[component]
pub(crate) fn ConsoleTerminal(
    dependency_page: ReadSignal<Option<DependencyPage>>,
    build_mode: ReadSignal<&'static str>,
) -> impl IntoView {
    let host = NodeRef::<Div>::new();
    let (terminal_handle, set_terminal_handle) = signal(None::<JsValue>);
    let active_profile_id = Memo::new(move |_| {
        dependency_page
            .get()
            .filter(|page| page.editable_profile)
            .map(|page| page.id)
    });

    host.on_load(move |element| {
        let profile_id = active_profile_id.get().unwrap_or_default();
        let handle = js_create_terminal(element.into(), &profile_id, build_mode.get());
        set_terminal_handle.set(Some(handle));
    });

    Effect::new(move |_| {
        let Some(handle) = terminal_handle.get() else {
            return;
        };
        let profile_id = active_profile_id.get().unwrap_or_default();
        js_set_terminal_profile(&handle, &profile_id, build_mode.get());
    });

    Effect::new(move |_| {
        if let Some(handle) = terminal_handle.get() {
            js_fit_terminal(&handle);
        }
    });

    on_cleanup(move || {
        if let Some(handle) = terminal_handle.get_untracked() {
            js_dispose_terminal(&handle);
        }
    });

    view! { <div class="console-terminal" node_ref=host></div> }
}
