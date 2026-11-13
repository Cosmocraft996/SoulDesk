//! 预置组件复用独立包源码，首次添加时仍经过正常校验和受限试加载。
use crate::store::{err, AppResult};
use serde_json::{json, Value};
use std::io::{Cursor, Write};

pub struct Preset {
    manifest: &'static str,
    description: &'static str,
    symbol: &'static str,
    files: &'static [(&'static str, &'static [u8])],
}

pub const PRESETS: &[Preset] = &[
    Preset {
        manifest: include_str!("../../examples/market/manifest.json"),
        description: "币安 USDT 永续自选行情、Last / Mark 价格、到价警报与触发历史。",
        symbol: "↗",
        files: &[
            (
                "dist/index.html",
                include_bytes!("../../examples/market/index.html"),
            ),
            (
                "dist/style.css",
                include_bytes!("../../examples/market/style.css"),
            ),
            (
                "dist/app.js",
                include_bytes!("../../examples/market/app.js"),
            ),
            (
                "dist/price.js",
                include_bytes!("../../examples/market/price.js"),
            ),
        ],
    },
    Preset {
        manifest: include_str!("../../examples/clock/manifest.json"),
        description: "显示本地时间与日期，可设置标题、显示或隐藏秒数。",
        symbol: "◷",
        files: &[
            (
                "dist/index.html",
                include_bytes!("../../examples/clock/index.html"),
            ),
            (
                "dist/style.css",
                include_bytes!("../../examples/clock/style.css"),
            ),
            (
                "dist/clock.js",
                include_bytes!("../../examples/clock/clock.js"),
            ),
        ],
    },
    Preset {
        manifest: include_str!("../../examples/template/manifest.json"),
        description: "自定义一段显示文字，每个实例独立保存，亦可用作组件开发示例。",
        symbol: "▤",
        files: &[
            (
                "dist/index.html",
                include_bytes!("../../examples/template/index.html"),
            ),
            (
                "dist/style.css",
                include_bytes!("../../examples/template/style.css"),
            ),
            (
                "dist/card.js",
                include_bytes!("../../examples/template/card.js"),
            ),
        ],
    },
];

impl Preset {
    fn metadata(&self) -> AppResult<Value> {
        serde_json::from_str(self.manifest).map_err(err)
    }

    pub fn archive(&self) -> AppResult<Vec<u8>> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (path, bytes) in [
            ("manifest.json", self.manifest.as_bytes()),
            (
                "dist/host.js",
                include_bytes!("../../sdk/host.js").as_slice(),
            ),
        ]
        .into_iter()
        .chain(self.files.iter().copied())
        {
            zip.start_file(path, options).map_err(err)?;
            zip.write_all(bytes).map_err(err)?;
        }
        Ok(zip.finish().map_err(err)?.into_inner())
    }
}

pub fn find(id: &str) -> AppResult<Option<&'static Preset>> {
    for preset in PRESETS {
        if preset.metadata()?["id"] == id {
            return Ok(Some(preset));
        }
    }
    Ok(None)
}

/// 目录只合并展示信息，不在启动或刷新时安装组件、创建实例或启动行情。
pub fn catalog(installed: &[Value]) -> AppResult<Vec<Value>> {
    let mut catalog = Vec::new();
    for preset in PRESETS {
        let manifest = preset.metadata()?;
        let active = installed.iter().find(|p| p["id"] == manifest["id"]);
        let mut entry = active.unwrap_or(&manifest).clone();
        entry["preset"] = json!(true);
        entry["installed"] = json!(active.is_some());
        entry["description"] = json!(preset.description);
        entry["symbol"] = json!(preset.symbol);
        catalog.push(entry);
    }
    for package in installed {
        if !catalog.iter().any(|p| p["id"] == package["id"]) {
            let mut entry = package.clone();
            entry["preset"] = json!(false);
            entry["installed"] = json!(true);
            catalog.push(entry);
        }
    }
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{store::Store, widgets};

    #[test]
    fn fresh_library_has_three_presets_without_installing_or_creating_instances() {
        let store =
            Store::open(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string())).unwrap();
        let before = store.snapshot().unwrap();
        let catalog = catalog(before["packages"].as_array().unwrap()).unwrap();
        assert_eq!(catalog.len(), 3);
        assert!(catalog
            .iter()
            .all(|p| p["preset"] == true && p["installed"] == false));
        assert_eq!(store.snapshot().unwrap(), before);
    }

    #[test]
    fn all_presets_load_through_package_pipeline_and_can_be_added_again_after_unregistering() {
        let store =
            Store::open(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string())).unwrap();
        for preset in PRESETS {
            let bytes = preset.archive().unwrap();
            assert_eq!(bytes, preset.archive().unwrap());
            let staged = widgets::stage_bytes(&store, bytes).unwrap();
            // 这是核心包管线检查；真实 WebView 消息桥试加载另行验收。
            let manifest = widgets::activate(&store, &staged).unwrap();
            for (path, _) in preset.files.iter().copied().chain(std::iter::once((
                "dist/host.js",
                include_bytes!("../../sdk/host.js").as_slice(),
            ))) {
                assert!(widgets::resource(&store, &format!("{}/{path}", manifest.id)).is_ok());
            }
            let id = widgets::create_instance(&store, &manifest.id).unwrap();
            assert!(widgets::stage_bytes(&store, preset.archive().unwrap())
                .unwrap()
                .unchanged());
            widgets::remove_instance(&store, &widgets::Sessions::default(), &id).unwrap();
            widgets::unregister_package(&store, &manifest.id).unwrap();
            let snapshot = store.snapshot().unwrap();
            let catalog = catalog(snapshot["packages"].as_array().unwrap()).unwrap();
            assert_eq!(catalog.len(), 3);
            assert!(catalog.iter().all(|p| p["installed"] == false));
            let staged = widgets::stage_bytes(&store, preset.archive().unwrap()).unwrap();
            widgets::activate(&store, &staged).unwrap();
            widgets::unregister_package(&store, &manifest.id).unwrap();
        }
    }

    #[test]
    fn catalog_preserves_imported_versions_and_keeps_external_packages() {
        let mut upgraded = PRESETS[0].metadata().unwrap();
        upgraded["version"] = json!("9.0.0");
        upgraded["name"] = json!("用户导入的行情版本");
        let catalog = catalog(&[
            upgraded.clone(),
            json!({"id":"local.custom.tool","version":"1.0.0"}),
        ])
        .unwrap();
        assert_eq!(catalog.len(), 4);
        assert_eq!(catalog[0]["name"], upgraded["name"]);
        assert_eq!(catalog[0]["version"], "9.0.0");
        assert_eq!(catalog[0]["installed"], true);
        assert_eq!(catalog[3]["preset"], false);
        assert_eq!(catalog[3]["installed"], true);
        assert!(find("local.custom.tool").unwrap().is_none());
    }
}
