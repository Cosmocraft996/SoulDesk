fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        build_desktop_grid();
    }
    let attributes =
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&[
            "snapshot",
            "save_panel_layout",
            "panel_view",
            "manage_panel_group",
            "official_catalog",
            "install_official",
            "app_update",
            "set_panel",
            "drag_panel_edge",
            "surface_activate",
            "surface_pointer",
            "surface_failed",
            "island_surface",
            "notification_settings",
            "import_package",
            "create_instance",
            "manage_instance",
            "reorder_instances",
            "unregister_package",
            "open_session",
            "close_session",
            "widget_request",
            "assistant_open_session",
            "assistant_close_session",
            "assistant_widget_request",
            "assistant_surface_failed",
            "job_open_session",
            "job_close_session",
            "job_widget_request",
            "job_surface_failed",
            "window_action",
            "set_shortcut",
            "set_autostart",
            "schedule_test",
            "business_settings",
            "binance_account_settings",
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

fn build_desktop_grid() {
    use std::{env, fs, path::PathBuf, process::Command};

    let source = PathBuf::from("desktop-grid");
    println!("cargo:rerun-if-changed=desktop-grid");
    let framework = PathBuf::from(env::var_os("WINDIR").expect("缺少 Windows 系统目录"))
        .join("Microsoft.NET")
        .join("Framework64")
        .join("v4.0.30319");
    let wpf = framework.join("WPF");
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("缺少 Cargo 输出目录"))
        .join("desktop-grid.exe");
    let mut files: Vec<_> = fs::read_dir(&source)
        .expect("找不到桌面格子源码")
        .map(|entry| entry.expect("无法读取桌面格子源码").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "cs"))
        .collect();
    files.sort();
    let mut compiler = Command::new(framework.join("csc.exe"));
    compiler.args([
        "/nologo",
        "/target:winexe",
        "/platform:anycpu",
        "/optimize+",
        "/codepage:65001",
        "/utf8output",
        "/win32manifest:desktop-grid/windows.manifest",
        "/reference:System.Windows.Forms.dll",
        "/reference:System.Drawing.dll",
        "/reference:System.Web.Extensions.dll",
        "/reference:System.Data.dll",
        "/reference:Accessibility.dll",
    ]);
    for assembly in [
        "UIAutomationClient.dll",
        "UIAutomationTypes.dll",
        "WindowsBase.dll",
    ] {
        compiler.arg(format!("/reference:{}", wpf.join(assembly).display()));
    }
    let result = compiler
        .arg(format!("/out:{}", output.display()))
        .args(files)
        .output()
        .expect("无法启动 Windows .NET Framework C# 编译器");
    assert!(
        result.status.success(),
        "桌面格子编译失败\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
