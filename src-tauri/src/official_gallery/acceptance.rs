use super::*;
use crate::integration::check;
use serde_json::Value;
use std::io::{Cursor, Write};

const NOTE: &str = "official.souldesk.daily-note";
const CLOCK: &str = "official.souldesk.local-clock";

// 合法，但永远不报告就绪的本地验收组件。
fn quiet_zip(installed: &Value, version: &str) -> AppResult<Vec<u8>> {
    let mut manifest = installed.clone();
    manifest["version"] = json!(version);
    manifest
        .as_object_mut()
        .ok_or("组件清单无效")?
        .remove("digest");
    let entry = manifest["entry"].as_str().ok_or("组件清单缺少入口")?;
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("manifest.json", options).map_err(err)?;
    writer
        .write_all(&serde_json::to_vec(&manifest).map_err(err)?)
        .map_err(err)?;
    writer.start_file(entry, options).map_err(err)?;
    writer
        .write_all(b"<!doctype html><meta charset=utf-8><title>Gallery acceptance</title>")
        .map_err(err)?;
    Ok(writer.finish().map_err(err)?.into_inner())
}

fn package<'a>(snapshot: &'a Value, id: &str) -> Option<&'a Value> {
    snapshot["packages"]
        .as_array()?
        .iter()
        .find(|item| item["id"] == id)
}

pub(crate) async fn run(
    app: &AppHandle,
    main: &WebviewWindow,
    report: &mut Value,
) -> AppResult<()> {
    let core = app.state::<Core>();
    let store = &core.store;
    let catalog_state = official_catalog(main.clone(), app.state(), app.state()).await?;
    if let Some(error) = catalog_state.error {
        return Err(format!("联网取得签名目录失败：{error}"));
    }
    let mut catalog = catalog_state.catalog.ok_or("签名目录为空")?;
    report["scope"] = json!("官方线上目录与 Releases、真实 WebView2 试加载、独立测试库");
    report["catalogRevision"] = json!(catalog.revision);
    report["catalogUrl"] = json!(CATALOG_URL);
    report["network"] = json!("本机系统网络设置");
    let note = catalog
        .widgets
        .iter()
        .find(|item| item.id == NOTE)
        .ok_or("目录缺少桌面便签")?;
    let clock = catalog
        .widgets
        .iter()
        .find(|item| item.id == CLOCK)
        .ok_or("目录缺少本地时钟")?;
    let first = note
        .versions
        .iter()
        .find(|release| release.version == "0.1.0" && release.status == ReleaseStatus::Published)
        .ok_or("目录缺少初始便签版本")?;
    let host = version(&app.package_info().version.to_string()).ok_or("宿主版本无效")?;
    let updated = latest(note, host).ok_or("无兼容便签版本")?;
    let clock_release = latest(clock, host).ok_or("无兼容时钟版本")?;
    report["versions"] = json!({
        "便签旧版": first.version, "便签新版": updated.version, "时钟": clock_release.version,
        "便签旧版摘要": first.sha256, "便签新版摘要": updated.sha256, "时钟摘要": clock_release.sha256,
    });
    check(
        report,
        "生产目录验签并缓存",
        catalog_state.online && cached(store).is_some(),
        json!(catalog.revision),
    )?;
    let network = network_config(store)?;
    let baseline_zip = download(&network, first).await?;
    let mut damaged = baseline_zip.clone();
    damaged[0] ^= 1;
    let initial = store.snapshot()?;
    check(
        report,
        "坏摘要拒绝进入安装",
        validate_bytes(first, &damaged).is_err() && store.snapshot()? == initial,
        json!(first.version),
    )?;
    let first_release = first.clone();
    crate::packages::install_verified(app, baseline_zip, move |manifest| {
        check_manifest(manifest, NOTE, &first_release)
    })
    .await?;
    let first_snapshot = store.snapshot()?;
    check(
        report,
        "旧版安装与实例分离",
        first_snapshot["instances"] == initial["instances"]
            && package(&first_snapshot, NOTE).is_some_and(|item| item["digest"] == first.sha256),
        json!(first.version),
    )?;
    for text in ["验收实例甲", "验收实例乙"] {
        let instance = crate::create_instance(app.clone(), NOTE.into()).await?;
        // 两份设置由验收数据预置，升级继续走正式试加载和事务。
        store
            .db
            .lock()
            .map_err(err)?
            .execute(
                "UPDATE widget_instances SET settings=?1 WHERE id=?2",
                rusqlite::params![json!({ "text": text }).to_string(), instance],
            )
            .map_err(err)?;
    }
    let previous = store.snapshot()?;
    install_official(
        app.clone(),
        main.clone(),
        app.state(),
        NOTE.into(),
        updated.sha256.clone(),
    )
    .await?;
    let after_update = store.snapshot()?;
    let active = package(&after_update, NOTE).ok_or("升级后的组件包未登记")?;
    check(
        report,
        "线上升级保留两份面板实例配置",
        updated.version != first.version
            && active["digest"] == updated.sha256
            && previous["instances"].as_array().is_some_and(|items| {
                items.len() == 2 && items.iter().all(|item| item["location"] == "panel")
            })
            && after_update["instances"] == previous["instances"],
        json!({"version":updated.version,"instances":2}),
    )?;

    let result =
        crate::packages::install_verified(app, quiet_zip(active, &updated.version)?, |_| Ok(()))
            .await;
    check(
        report,
        "同版本内容冲突保留已安装包",
        result
            .as_ref()
            .is_err_and(|error| error.contains("VERSION_INCOMPATIBLE"))
            && store.snapshot()? == after_update,
        json!(result.err()),
    )?;
    let probe_version = format!(
        "{}.0.0",
        version(&updated.version).ok_or("组件版本无效")?[0]
            .checked_add(1)
            .ok_or("无法构造测试版本")?
    );
    let result =
        crate::packages::install_verified(app, quiet_zip(active, &probe_version)?, |_| Ok(()))
            .await;
    check(
        report,
        "真实隐藏试加载超时保留旧包和两份配置",
        result
            .as_ref()
            .is_err_and(|error| error.contains("未在10秒内"))
            && store.snapshot()? == after_update,
        json!(result.err()),
    )?;

    install_official(
        app.clone(),
        main.clone(),
        app.state(),
        CLOCK.into(),
        clock_release.sha256.clone(),
    )
    .await?;
    let after_install = store.snapshot()?;
    check(
        report,
        "线上首次安装登记包并等待用户添加实例",
        package(&after_install, CLOCK).is_some_and(|item| item["digest"] == clock_release.sha256)
            && after_install["instances"] == previous["instances"],
        json!(clock_release.version),
    )?;

    let (mut trusted, _) = cached(store).ok_or("可信缓存缺失")?;
    trusted.signature = STANDARD.encode([0u8; 64]);
    check(
        report,
        "验签失败仍保留可信目录",
        verify(&trusted, &PUBLIC_KEY).is_err()
            && cached(store).is_some_and(|(_, cached)| cached.revision == catalog.revision),
        json!(catalog.revision),
    )?;
    store.save_preference("network", r#"{"mode":"http","host":"127.0.0.1","port":1}"#)?;
    let offline = official_catalog(main.clone(), app.state(), app.state()).await?;
    check(
        report,
        "断网读取持久化签名目录",
        !offline.online
            && offline
                .catalog
                .is_some_and(|saved| saved.revision == catalog.revision),
        json!(offline.error),
    )?;
    let result = install_official(
        app.clone(),
        main.clone(),
        app.state(),
        CLOCK.into(),
        clock_release.sha256.clone(),
    )
    .await;
    check(
        report,
        "离线安装必须等待在线复核",
        result
            .as_ref()
            .is_err_and(|error| error.contains("在线复核失败"))
            && store.snapshot()? == after_install,
        json!(result.err()),
    )?;
    let result = crate::unregister_package(app.clone(), NOTE.into()).await;
    check(
        report,
        "离线卸载保护仍有实例的组件",
        result
            .as_ref()
            .is_err_and(|error| error.contains("2 个实例"))
            && store.snapshot()? == after_install,
        json!(result.err()),
    )?;

    let clock = catalog
        .widgets
        .iter_mut()
        .find(|item| item.id == CLOCK)
        .ok_or("目录缺少本地时钟")?;
    clock
        .versions
        .iter_mut()
        .for_each(|item| item.status = ReleaseStatus::Withdrawn);
    check(
        report,
        "模拟撤回阻止线上安装并独立保留本地登记",
        latest(clock, host).is_none() && package(&store.snapshot()?, CLOCK).is_some(),
        json!(CLOCK),
    )?;
    crate::unregister_package(app.clone(), CLOCK.into()).await?;
    let remaining = store.snapshot()?;
    let clock_history: u32 = store
        .db
        .lock()
        .map_err(err)?
        .query_row(
            "SELECT COUNT(*) FROM widget_versions WHERE package_id=?1",
            [CLOCK],
            |row| row.get(0),
        )
        .map_err(err)?;
    check(
        report,
        "离线卸载无实例组件并保留历史资源",
        package(&remaining, CLOCK).is_none()
            && clock_history == 1
            && remaining["instances"] == previous["instances"],
        json!({"history":clock_history}),
    )?;
    Ok(())
}
