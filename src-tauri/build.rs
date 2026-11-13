fn main() {
    let attributes =
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "snapshot",
            "import_package",
            "create_instance",
            "manage_instance",
            "reorder_instances",
            "unregister_package",
            "open_session",
            "close_session",
            "widget_request",
            "window_action",
            "set_shortcut",
            "set_autostart",
            "schedule_test",
            "business_settings",
            "report_view",
            "trial_failed",
            "desktop_probe_set_ratio",
            "desktop_probe_set_tint",
            "desktop_probe_action",
            "desktop_probe_observe",
            "desktop_widget_open_session",
            "desktop_widget_close_session",
            "desktop_widget_widget_request",
            "desktop_widget_trial_failed",
            "desktop_widget_view",
            "desktop_widget_activate",
        ]));
    tauri_build::try_build(attributes).expect("Tauri 构建配置无效");
}
