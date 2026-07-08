//! The in-browser GDS -> `.rtla` convert Web Worker (lane v8-6c).
//!
//! Trunk builds this bin as a Web Worker (`data-type="worker"` in `index.html`). It runs
//! the conversion off the main thread so a large GDS never blocks the UI, and writes the
//! finished archive into the Origin Private File System (OPFS) under a stable name, where
//! the app can reopen it through the streaming archive path.
//!
//! # Protocol
//!
//! The worker posts `{ type: "ready" }` once its message handler is installed. The app
//! then posts a job `{ gds: ArrayBuffer | Uint8Array, name?: string }`. The worker
//! replies with, in order, `{ type: "status", stage }` messages, then either
//! `{ type: "done", path, records, bytes }` (the OPFS-relative path of the archive) or
//! `{ type: "error", message }`.
//!
//! # Why a worker owns the OPFS write
//!
//! `FileSystemSyncAccessHandle` -- the simplest way to write a whole archive at once --
//! is only available in a Worker, and `navigator.storage` here is the worker scope's, not
//! `window`'s (which is absent in a worker). So the OPFS write lives here rather than in
//! the frozen `reticle-index` tile-cache glue, which is `window`-bound.

/// Native builds of the worker bin do nothing; it is only meaningful compiled to wasm by
/// Trunk. A no-op keeps `cargo build`/`clippy --workspace` green on the host.
#[cfg(not(target_arch = "wasm32"))]
fn main() {}

/// wasm entry: install the message handler and announce readiness (Trunk's worker
/// bundle runs this on instantiation).
#[cfg(target_arch = "wasm32")]
fn main() {
    wasm::start();
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use js_sys::{Object, Reflect, Uint8Array};
    use wasm_bindgen::JsCast;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{
        DedicatedWorkerGlobalScope, FileSystemDirectoryHandle, FileSystemFileHandle,
        FileSystemGetDirectoryOptions, FileSystemGetFileOptions, FileSystemSyncAccessHandle,
        MessageEvent,
    };

    use web::convert::convert_gds_to_rtla;

    /// The OPFS subdirectory converted archives are written into.
    const ARCHIVE_DIR: &str = "archives";

    /// Default archive filename when the job carries no `name`.
    const DEFAULT_NAME: &str = "converted.rtla";

    pub fn start() {
        console_error_panic_hook::set_once();
        let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();

        let handler_scope = scope.clone();
        let onmessage = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            let scope = handler_scope.clone();
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(message) = run_job(&scope, event.data()).await {
                    post(&scope, "error", &[("message", JsValue::from_str(&message))]);
                }
            });
        });
        scope.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        // The closure must outlive `start`; the worker lives for the page's lifetime.
        onmessage.forget();

        // Tell the app the worker is ready to receive a job.
        post(&scope, "ready", &[]);
    }

    /// Runs one conversion job: decode the GDS bytes, convert to `.rtla`, and write the
    /// archive into OPFS, posting status and a final `done` message.
    async fn run_job(scope: &DedicatedWorkerGlobalScope, data: JsValue) -> Result<(), String> {
        let gds_value = Reflect::get(&data, &JsValue::from_str("gds"))
            .map_err(|_| "job message has no `gds` field".to_owned())?;
        let gds = to_bytes(&gds_value)?;
        let name = Reflect::get(&data, &JsValue::from_str("name"))
            .ok()
            .and_then(|v| v.as_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_NAME.to_owned());

        post(
            scope,
            "status",
            &[("stage", JsValue::from_str("converting"))],
        );
        let (bytes, summary) = convert_gds_to_rtla(&gds).map_err(|e| e.to_string())?;

        post(scope, "status", &[("stage", JsValue::from_str("writing"))]);
        let path = write_opfs(scope, &name, &bytes).await?;

        post(
            scope,
            "done",
            &[
                ("path", JsValue::from_str(&path)),
                ("records", JsValue::from_f64(summary.record_count as f64)),
                ("bytes", JsValue::from_f64(bytes.len() as f64)),
            ],
        );
        Ok(())
    }

    /// Extracts the input bytes from the job's `gds` field, which the app may send as an
    /// `ArrayBuffer` or a `Uint8Array`.
    fn to_bytes(value: &JsValue) -> Result<Vec<u8>, String> {
        if value.is_instance_of::<Uint8Array>() {
            Ok(value.unchecked_ref::<Uint8Array>().to_vec())
        } else if value.is_instance_of::<js_sys::ArrayBuffer>() {
            Ok(Uint8Array::new(value).to_vec())
        } else {
            Err("`gds` is neither an ArrayBuffer nor a Uint8Array".to_owned())
        }
    }

    /// Writes `bytes` to `<ARCHIVE_DIR>/<name>` in OPFS, returning that OPFS-relative
    /// path. Uses a synchronous access handle (worker-only) to write the whole archive at
    /// once, truncating any prior file so a reconvert fully replaces it.
    async fn write_opfs(
        scope: &DedicatedWorkerGlobalScope,
        name: &str,
        bytes: &[u8],
    ) -> Result<String, String> {
        let storage = scope.navigator().storage();
        let root: FileSystemDirectoryHandle = JsFuture::from(storage.get_directory())
            .await
            .map_err(|e| format!("OPFS unavailable: {}", describe(&e)))?
            .unchecked_into();

        let dir_options = FileSystemGetDirectoryOptions::new();
        dir_options.set_create(true);
        let dir: FileSystemDirectoryHandle =
            JsFuture::from(root.get_directory_handle_with_options(ARCHIVE_DIR, &dir_options))
                .await
                .map_err(|e| format!("open OPFS dir: {}", describe(&e)))?
                .unchecked_into();

        let file_options = FileSystemGetFileOptions::new();
        file_options.set_create(true);
        let file: FileSystemFileHandle =
            JsFuture::from(dir.get_file_handle_with_options(name, &file_options))
                .await
                .map_err(|e| format!("open OPFS file: {}", describe(&e)))?
                .unchecked_into();

        let sync: FileSystemSyncAccessHandle = JsFuture::from(file.create_sync_access_handle())
            .await
            .map_err(|e| format!("open sync access handle: {}", describe(&e)))?
            .unchecked_into();

        // Replace any previous archive at this name, then write from the start.
        let result: Result<(), String> = (|| {
            sync.truncate_with_f64(0.0)
                .map_err(|e| format!("truncate: {}", describe(&e)))?;
            sync.write_with_u8_array(bytes)
                .map_err(|e| format!("write: {}", describe(&e)))?;
            sync.flush()
                .map_err(|e| format!("flush: {}", describe(&e)))?;
            Ok(())
        })();
        sync.close();
        result?;

        Ok(format!("{ARCHIVE_DIR}/{name}"))
    }

    /// Posts a `{ type, ...fields }` message to the app.
    fn post(scope: &DedicatedWorkerGlobalScope, kind: &str, fields: &[(&str, JsValue)]) {
        let object = Object::new();
        let _ = Reflect::set(
            &object,
            &JsValue::from_str("type"),
            &JsValue::from_str(kind),
        );
        for (key, value) in fields {
            let _ = Reflect::set(&object, &JsValue::from_str(key), value);
        }
        let _ = scope.post_message(&object);
    }

    /// Renders a `JsValue` error into a short string.
    fn describe(value: &JsValue) -> String {
        value
            .as_string()
            .or_else(|| {
                value
                    .dyn_ref::<js_sys::Error>()
                    .map(|e| String::from(e.message()))
            })
            .unwrap_or_else(|| "error".to_owned())
    }
}
