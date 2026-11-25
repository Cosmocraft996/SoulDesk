use crate::AppResult;
use tauri::AppHandle;

pub(crate) fn launch(app: &AppHandle, restore: bool) -> AppResult<()> {
    #[cfg(not(windows))]
    {
        let _ = (app, restore);
        Err("桌面格子需要 Windows".into())
    }
    #[cfg(windows)]
    {
        use crate::Core;
        use std::process::Command;
        use tauri::Manager;

        let core = app.state::<Core>();
        let root = local_grid_path(&core.store.root.join("desktop-grid"))?;
        let isolated = core.integration.is_some() || core.verification.is_some();
        let renderer = if restore || isolated {
            None
        } else {
            let directory = app
                .path()
                .resource_dir()
                .map_err(|error| format!("找不到桌面格子资源目录：{error}"))?
                .join("desktop-grid-winui");
            let executable = directory.join("SoulDeskGridWinUI.exe");
            if !executable.is_file() || !directory.join("SoulDeskGridWinUI.dll").is_file() {
                return Err("当前安装包缺少独立桌面格子运行文件。".into());
            }
            Some(local_grid_path(&executable)?)
        };
        let executable = prepare(&root, !restore && !isolated)?;
        let mut command = Command::new(executable);
        command.arg(if restore { "--restore" } else { "--open" });
        command.arg(root);
        if isolated {
            command.arg("--isolated");
        }
        if !restore {
            command.args(["--parent", &std::process::id().to_string(), "0"]);
        }
        if let Some(renderer) = renderer {
            command.arg("--renderer-exe").arg(renderer);
        }
        command
            .spawn()
            .map_err(|error| format!("无法启动桌面格子：{error}"))?;
        Ok(())
    }
}

#[cfg(windows)]
fn local_grid_path(path: &std::path::Path) -> AppResult<std::path::PathBuf> {
    use std::path::{Component, PathBuf, Prefix};

    // Tauri 的安装资源路径经过规范化。格子引擎只接受带盘符的传统 Win32 路径。
    let mut parts = path.components();
    match (parts.next(), parts.next()) {
        (Some(Component::Prefix(prefix)), Some(Component::RootDir)) => match prefix.kind() {
            Prefix::Disk(_) => Ok(path.to_owned()),
            Prefix::VerbatimDisk(letter) => {
                let mut result = PathBuf::from(format!("{}:\\", char::from(letter)));
                result.extend(parts);
                Ok(result)
            }
            _ => Err("桌面格子需要使用本机磁盘路径。".into()),
        },
        _ => Err("桌面格子需要使用本机磁盘路径。".into()),
    }
}

pub(crate) fn preview(root: &std::path::Path) -> AppResult<()> {
    #[cfg(not(windows))]
    {
        let _ = root;
        Err("桌面格子需要 Windows".into())
    }
    #[cfg(all(windows, not(feature = "desktop-grid-winui-preview")))]
    {
        std::process::Command::new(prepare(root, false)?)
            .arg("--open")
            .arg(root)
            .args(["--isolated", "--demo"])
            .spawn()
            .map_err(|error| format!("无法启动格子体验：{error}"))?;
        Ok(())
    }
    #[cfg(all(windows, feature = "desktop-grid-winui-preview"))]
    {
        run_winui_preview(root)
    }
}

#[cfg(all(windows, feature = "desktop-grid-winui-preview"))]
fn run_winui_preview(root: &std::path::Path) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    use std::{env, fs, process::Command};

    let executable = env::current_exe().map_err(|error| error.to_string())?;
    let directory = executable
        .parent()
        .ok_or("找不到 SoulDesk 调试包目录")?
        .join("desktop-grid-winui");
    // 只接受在正常 Explorer 桌面通过鼠标拖放、折叠和退出验收的版本。
    for (file, expected) in [
        (
            "SoulDeskGridWinUI.exe",
            "6eab4bb2db251dbff5703afffea794f6ee274231e1a610f950669d2f414c3f80",
        ),
        (
            "SoulDeskGridWinUI.dll",
            "04f1e99cc34388abbe7ba8f92eef07f91ad44c6951d892394b7c52fa6dbbcf3a",
        ),
    ] {
        let bytes = fs::read(directory.join(file))
            .map_err(|error| format!("找不到独立格子程序 {file}：{error}"))?;
        if format!("{:x}", Sha256::digest(bytes)) != expected {
            return Err(format!("独立格子程序与通过验收的版本不一致：{file}"));
        }
    }
    let state = root.join("deskbox-preview-data");
    let status = Command::new(directory.join("SoulDeskGridWinUI.exe"))
        .current_dir(&directory)
        .arg("--souldesk-grid-preview")
        .arg(state)
        .status()
        .map_err(|error| format!("无法启动独立格子程序：{error}"))?;
    if !status.success() {
        return Err(format!("独立格子程序启动或退出失败（{status}）"));
    }
    Ok(())
}

#[cfg(windows)]
fn prepare(root: &std::path::Path, with_guard: bool) -> AppResult<std::path::PathBuf> {
    use sha2::{Digest, Sha256};

    const HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/desktop-grid.exe"));
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).map_err(|error| error.to_string())?;
    let version = format!("{:x}", Sha256::digest(HELPER));
    let executable = bin.join(format!("{version}.exe"));
    unpack(&executable, HELPER)?;
    if with_guard {
        unpack(&executable.with_extension("guard.exe"), HELPER)?;
    }
    Ok(executable)
}

#[cfg(windows)]
fn unpack(path: &std::path::Path, bytes: &[u8]) -> AppResult<()> {
    use std::{fs, io::Write};

    if fs::read(path).ok().as_deref() == Some(bytes) {
        return Ok(());
    }
    let pending = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let write = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&pending)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&pending, path)
    })();
    if let Err(error) = write {
        let _ = fs::remove_file(&pending);
        if fs::read(path).ok().as_deref() != Some(bytes) {
            return Err(format!("无法准备桌面格子：{error}"));
        }
    }
    Ok(())
}
