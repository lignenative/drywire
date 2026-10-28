//! Tauri's build step: reads tauri.conf.json and the capabilities, and
//! embeds the Windows icon and manifest.

fn main() {
    tauri_build::build();
}
