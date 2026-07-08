// Bootstrap for the in-browser GDS -> .rtla convert Web Worker (lane v8-6c).
//
// Trunk builds crates/web/src/bin/convert_worker.rs as a `no-modules` wasm-bindgen
// bundle emitted at the stable names convert_worker.js / convert_worker_bg.wasm. That
// bundle only DEFINES the global `wasm_bindgen` init function; it does not boot itself.
// This tiny classic worker script imports it and calls it, which instantiates the wasm
// and runs the bin's `main` (installing the message handler that does the conversion and
// the OPFS write). It is committed (rather than generated) because Trunk emits no spawn
// shim for `data-type="worker"`.
//
// The app spawns this with `new Worker('convert-worker-boot.js')`. Both URLs below are
// relative to this script, so they resolve at the dev root and under a subpath deploy
// (e.g. gh-pages /reticle/) alike.
importScripts("./convert_worker.js");

// Pass the wasm URL explicitly: a worker has no document.currentScript for the bundle to
// derive it from.
wasm_bindgen("./convert_worker_bg.wasm");
