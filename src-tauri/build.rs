fn main() {
  tauri_build::try_build(
    tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
      "show_main",
      "hide_popup",
      "toggle_popup",
      "get_global_shortcut",
      "set_global_shortcut",
      "setup_done",
    ])),
  )
  .expect("failed to run tauri-build");
}
