//! 显式隔离验收使用的夹具与外部包。正常组件库仅读取本机登记。
use crate::store::{err, AppResult};
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{Cursor, Read, Write},
    path::PathBuf,
};

pub struct Preset {
    manifest: &'static str,
    files: &'static [(&'static str, &'static [u8])],
}

const VERIFICATION_FIXTURES: &[Preset] = &[
    Preset {
        manifest: include_str!("../../tests/fixtures/surface/manifest.json"),
        files: &[
            (
                "dist/index.html",
                include_bytes!("../../tests/fixtures/surface/index.html"),
            ),
            (
                "dist/style.css",
                include_bytes!("../../tests/fixtures/surface/style.css"),
            ),
            (
                "dist/app.js",
                include_bytes!("../../tests/fixtures/surface/app.js"),
            ),
        ],
    },
    Preset {
        manifest: include_str!("../../tests/fixtures/document/manifest.json"),
        files: &[
            (
                "dist/index.html",
                include_bytes!("../../tests/fixtures/document/index.html"),
            ),
            (
                "dist/style.css",
                include_bytes!("../../tests/fixtures/document/style.css"),
            ),
            (
                "dist/app.js",
                include_bytes!("../../tests/fixtures/document/app.js"),
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

pub enum Fixture {
    Local(&'static Preset),
    External(PathBuf),
    #[cfg(test)]
    Manifest(Value),
}

impl Fixture {
    pub fn archive(&self) -> AppResult<Vec<u8>> {
        match self {
            Self::Local(fixture) => fixture.archive(),
            Self::External(path) => fs::read(path).map_err(err),
            #[cfg(test)]
            Self::Manifest(manifest) => {
                let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
                let options = zip::write::SimpleFileOptions::default();
                zip.start_file("manifest.json", options).map_err(err)?;
                zip.write_all(&serde_json::to_vec(manifest).map_err(err)?)
                    .map_err(err)?;
                zip.start_file(manifest["entry"].as_str().ok_or("夹具缺少入口")?, options)
                    .map_err(err)?;
                zip.write_all(b"<!doctype html><title>Fixture</title>")
                    .map_err(err)?;
                zip.start_file("dist/host.js", options).map_err(err)?;
                zip.write_all(include_bytes!("../../sdk/host.js"))
                    .map_err(err)?;
                Ok(zip.finish().map_err(err)?.into_inner())
            }
        }
    }
}

fn external() -> AppResult<Vec<(Value, PathBuf)>> {
    let dir = std::env::var_os("SOULDESK_INTEGRATION_PACKAGES")
        .map(PathBuf::from)
        .ok_or("设置 SOULDESK_INTEGRATION_PACKAGES 指向广场构建产物的绝对目录")?;
    if !dir.is_absolute() || !dir.is_dir() {
        return Err("隔离验收需要外部 ZIP 的绝对目录".into());
    }
    let mut packages = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for entry in fs::read_dir(dir).map_err(err)? {
        let entry = entry.map_err(err)?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("zip")
            || !entry.file_type().map_err(err)?.is_file()
        {
            continue;
        }
        let file = File::open(&path).map_err(err)?;
        if file.metadata().map_err(err)?.len() > 20 * 1024 * 1024 {
            return Err(format!("验收包超过大小限制：{}", path.display()));
        }
        let mut archive = zip::ZipArchive::new(file).map_err(err)?;
        let mut manifest = Vec::new();
        archive
            .by_name("manifest.json")
            .map_err(err)?
            .take(64 * 1024)
            .read_to_end(&mut manifest)
            .map_err(err)?;
        let metadata: Value = serde_json::from_slice(&manifest).map_err(err)?;
        let id = metadata["id"].as_str().ok_or("验收包缺少组件 ID")?;
        if !ids.insert(id.to_owned()) {
            return Err(format!("验收目录中有重复组件 ID：{id}"));
        }
        packages.push((metadata, path));
    }
    if packages.is_empty() {
        return Err("验收目录中没有组件 ZIP".into());
    }
    packages.sort_by(|(left, _), (right, _)| left["id"].as_str().cmp(&right["id"].as_str()));
    Ok(packages)
}

pub fn find(id: &str) -> AppResult<Option<Fixture>> {
    if let Some(fixture) = verification_fixture(id)? {
        return Ok(Some(Fixture::Local(fixture)));
    }
    // 原生单元测试只依赖 ID、权限和设置契约，无需安装业务组件。
    #[cfg(test)]
    {
        let manifests: Vec<Value> = serde_json::from_str(include_str!(
            "../../tests/fixtures/component-manifests.json"
        ))
        .map_err(err)?;
        if let Some(manifest) = manifests.into_iter().find(|manifest| manifest["id"] == id) {
            return Ok(Some(Fixture::Manifest(manifest)));
        }
    }
    Ok(external()?
        .into_iter()
        .find(|(manifest, _)| manifest["id"] == id)
        .map(|(_, path)| Fixture::External(path)))
}

pub(crate) fn verification_fixture(id: &str) -> AppResult<Option<&'static Preset>> {
    for fixture in VERIFICATION_FIXTURES {
        if fixture.metadata()?["id"] == id {
            return Ok(Some(fixture));
        }
    }
    Ok(None)
}

pub(crate) fn catalog() -> AppResult<Vec<(Value, PathBuf)>> {
    external()
}
