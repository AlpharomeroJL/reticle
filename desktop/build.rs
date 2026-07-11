//! Build script: hands off to `tauri-build`, which reads `tauri.conf.json`,
//! embeds the Windows `.exe` icon/version resource from `icons/icon.ico`, and
//! runs the (empty, capability-free) ACL pipeline. See
//! `docs/decisions/0119-tauri-desktop.md`.

fn main() {
    tauri_build::build();
}
