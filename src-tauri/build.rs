fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(
            tauri_build::AppManifest::new().commands(&[
                "get_status",
                "restart_dsh",
                "read_log",
                "open_log_dir",
                "check_update",
                "update_dsh",
            ]),
        ),
    )
    .expect("failed to run tauri-build");
}
