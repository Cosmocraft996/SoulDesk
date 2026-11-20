//! 官方目录通过签名与修订号确认，组件下载进入普通包的试加载事务。
pub(crate) mod acceptance;

use crate::{
    business, network,
    store::{err, AppResult, Store},
    widgets::Manifest,
    Core,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::{redirect, Client, Url};
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::HashSet, time::Duration};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

const CATALOG_URL: &str = "https://cosmocraft996.github.io/SoulDesk-Widgets/catalog.json";
const SIGNATURE_URL: &str = "https://cosmocraft996.github.io/SoulDesk-Widgets/catalog.json.sig";
const RELEASE_ROOT: &str = "https://github.com/Cosmocraft996/SoulDesk-Widgets/releases/download/";
const CACHE_KEY: &str = "official_widget_catalog_v1";
const MAX_CATALOG: usize = 1024 * 1024;
const MAX_ZIP: usize = 20 * 1024 * 1024;
// 与组件仓库 docs/catalog-public-key.pem 中的公开 Ed25519 公钥一致。
const PUBLIC_KEY: [u8; 32] = [
    0xa7, 0x2e, 0x09, 0x5d, 0xd9, 0x38, 0x3e, 0xf2, 0xfc, 0x72, 0xdd, 0xf2, 0xec, 0x3c, 0x95, 0x98,
    0xb9, 0xab, 0xde, 0x98, 0xb5, 0xd1, 0x92, 0xb9, 0xc7, 0xeb, 0x03, 0x69, 0x24, 0x37, 0x58, 0x58,
];

#[derive(Default)]
pub struct Gallery(tokio::sync::Mutex<()>);

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    schema_version: u32,
    revision: u64,
    widgets: Vec<Entry>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    id: String,
    name: String,
    summary: String,
    category: String,
    versions: Vec<Release>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Release {
    version: String,
    min_host_version: String,
    capabilities: Vec<String>,
    release_notes: String,
    status: ReleaseStatus,
    download_url: String,
    size_bytes: usize,
    sha256: String,
}

#[derive(Clone, Copy, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ReleaseStatus {
    Published,
    Withdrawn,
}

#[derive(Deserialize, Serialize)]
struct SignedCatalog {
    raw: String,
    signature: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GalleryResponse {
    catalog: Option<Catalog>,
    online: bool,
    error: Option<String>,
}

fn version(value: &str) -> Option<[u64; 3]> {
    let mut parts = value.split('.');
    let mut read = || {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let version = [read()?, read()?, read()?];
    parts.next().is_none().then_some(version)
}

fn valid_id(id: &str) -> bool {
    id.len() <= 80
        && id.contains('.')
        && id.split('.').all(|part| !part.is_empty())
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
}

fn valid_release_url(id: &str, release: &Release) -> bool {
    let Some(path) = release.download_url.strip_prefix(RELEASE_ROOT) else {
        return false;
    };
    let Some((tag, file)) = path.split_once('/') else {
        return false;
    };
    tag.starts_with("widget-")
        && tag.ends_with(&format!("-v{}", release.version))
        && tag
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
        && file == format!("{id}-{}.zip", release.version)
}

fn verify(signed: &SignedCatalog, key: &[u8; 32]) -> AppResult<Catalog> {
    if signed.raw.len() > MAX_CATALOG || signed.signature.len() != 88 {
        return Err("目录或签名大小无效".into());
    }
    let signature = STANDARD
        .decode(&signed.signature)
        .map_err(|_| "目录签名格式无效")?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(signed.raw.as_bytes(), &signature)
        .map_err(|_| "官方目录验签失败")?;
    let catalog: Catalog = serde_json::from_str(&signed.raw).map_err(|_| "目录格式无效")?;
    if catalog.schema_version != 1 || catalog.revision == 0 || catalog.revision > (1_u64 << 53) - 1
    {
        return Err("目录格式或修订号无效".into());
    }
    let mut ids = HashSet::new();
    for widget in &catalog.widgets {
        if !valid_id(&widget.id) || !ids.insert(&widget.id) || widget.name.trim().is_empty() {
            return Err("目录组件标识或名称无效".into());
        }
        let mut versions = HashSet::new();
        for release in &widget.versions {
            let capabilities = release.capabilities.iter().collect::<HashSet<_>>();
            if !version(&release.version).is_some_and(|v| versions.insert(v))
                || version(&release.min_host_version).is_none()
                || release.size_bytes == 0
                || release.size_bytes > MAX_ZIP
                || release.sha256.len() != 64
                || !release
                    .sha256
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                || capabilities.len() != release.capabilities.len()
                || release.capabilities.iter().any(|cap| cap.is_empty())
                || !valid_release_url(&widget.id, release)
            {
                return Err(format!("组件版本记录无效：{}", widget.id));
            }
        }
    }
    Ok(catalog)
}

fn latest(entry: &Entry, host: [u64; 3]) -> Option<&Release> {
    entry
        .versions
        .iter()
        .filter(|item| {
            item.status == ReleaseStatus::Published
                && version(&item.min_host_version).is_some_and(|min| min <= host)
        })
        .max_by_key(|item| version(&item.version))
}

fn cached(store: &Store) -> Option<(SignedCatalog, Catalog)> {
    let input = store.preference(CACHE_KEY)?;
    let signed: SignedCatalog = serde_json::from_str(&input).ok()?;
    let catalog = verify(&signed, &PUBLIC_KEY).ok()?;
    Some((signed, catalog))
}

fn network_config(store: &Store) -> AppResult<network::NetworkConfig> {
    let input = store.preference("network");
    let settings = match input {
        Some(raw) => serde_json::from_str(&raw).map_err(err)?,
        None => json!({ "mode": "system" }),
    };
    business::resolve_network(&settings)
}

fn verify_revision(
    previous: Option<&(SignedCatalog, Catalog)>,
    next: &SignedCatalog,
    catalog: &Catalog,
) -> AppResult<()> {
    if previous.is_some_and(|(old, data)| {
        catalog.revision < data.revision
            || (catalog.revision == data.revision && old.raw != next.raw)
    }) {
        return Err("官方目录修订号倒退或同修订号内容发生变化".into());
    }
    Ok(())
}

async fn refresh(
    store: &Store,
    network: &network::NetworkConfig,
    previous: Option<&(SignedCatalog, Catalog)>,
) -> AppResult<Catalog> {
    const HEADERS: &[(&str, &str)] = &[("Cache-Control", "no-cache")];
    let (raw, signature) = tokio::try_join!(
        network::http_get_bounded_headers(network, CATALOG_URL, MAX_CATALOG, HEADERS),
        network::http_get_bounded_headers(network, SIGNATURE_URL, 64, HEADERS)
    )?;
    let signed = SignedCatalog {
        raw: String::from_utf8(raw).map_err(|_| "目录必须为 UTF-8")?,
        signature: STANDARD.encode(signature),
    };
    let catalog = verify(&signed, &PUBLIC_KEY)?;
    verify_revision(previous, &signed, &catalog)?;
    if previous.is_none_or(|(_, previous)| previous.revision != catalog.revision) {
        store.save_preference(CACHE_KEY, &serde_json::to_string(&signed).map_err(err)?)?;
    }
    Ok(catalog)
}

fn release_destination(url: &Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}

fn validate_bytes(release: &Release, bytes: &[u8]) -> AppResult<()> {
    if bytes.len() != release.size_bytes {
        return Err("ZIP 下载不完整，大小与目录不符".into());
    }
    if format!("{:x}", Sha256::digest(bytes)) != release.sha256 {
        return Err("ZIP 的 SHA-256 与目录不符".into());
    }
    Ok(())
}

async fn download(network: &network::NetworkConfig, release: &Release) -> AppResult<Vec<u8>> {
    let mut builder = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(30))
        .redirect(redirect::Policy::custom(|next| {
            if next.previous().len() <= 4 && release_destination(next.url()) {
                next.follow()
            } else {
                next.error("组件下载重定向超出允许范围")
            }
        }));
    if let Some(proxy) = network.proxy_url() {
        builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(err)?);
    }
    let result = builder
        .build()
        .map_err(err)?
        .get(&release.download_url)
        .send()
        .await
        .map_err(|_| "无法下载组件 ZIP，请检查网络")?;
    read_zip(result, release).await
}

async fn read_zip(mut result: reqwest::Response, release: &Release) -> AppResult<Vec<u8>> {
    if !result.status().is_success() {
        return Err(format!("组件下载失败，HTTP {}", result.status()));
    }
    if result
        .content_length()
        .is_some_and(|size| size != release.size_bytes as u64)
    {
        return Err("ZIP 下载大小与目录不符".into());
    }
    let mut bytes = Vec::with_capacity(release.size_bytes);
    while let Some(chunk) = result.chunk().await.map_err(|_| "组件下载中断，请重试")? {
        if chunk.len() > release.size_bytes.saturating_sub(bytes.len()) {
            return Err("ZIP 下载大小超过目录记录".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    validate_bytes(release, &bytes)?;
    Ok(bytes)
}

fn management(window: &WebviewWindow) -> AppResult<()> {
    if window.label() != "main" {
        return Err("官方广场仅在组件管理窗口可用".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn official_catalog(
    window: WebviewWindow,
    core: State<'_, Core>,
    state: State<'_, Gallery>,
    cached_only: Option<bool>,
) -> AppResult<GalleryResponse> {
    management(&window)?;
    if cached_only.unwrap_or(false) {
        return Ok(GalleryResponse {
            catalog: cached(&core.store).map(|(_, catalog)| catalog),
            online: false,
            error: None,
        });
    }
    let _guard = state.0.lock().await;
    let previous = cached(&core.store);
    let fetched = match network_config(&core.store) {
        Ok(network) => refresh(&core.store, &network, previous.as_ref()).await,
        Err(error) => Err(error),
    };
    Ok(match fetched {
        Ok(catalog) => GalleryResponse {
            catalog: Some(catalog),
            online: true,
            error: None,
        },
        Err(error) => GalleryResponse {
            catalog: previous.map(|(_, catalog)| catalog),
            online: false,
            error: Some(error),
        },
    })
}

fn check_manifest(manifest: &Manifest, widget_id: &str, release: &Release) -> AppResult<()> {
    if manifest.id != widget_id
        || manifest.version != release.version
        || manifest.capabilities != release.capabilities
    {
        return Err("组件包的 ID、版本或权限与官方目录不符".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn install_official(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, Gallery>,
    package_id: String,
    expected_sha256: String,
    expected_local_digest: Option<String>,
) -> AppResult<()> {
    management(&window)?;
    let _guard = state.0.lock().await;
    let core = app.state::<Core>();
    let previous = cached(&core.store);
    let network = network_config(&core.store)?;
    let catalog = refresh(&core.store, &network, previous.as_ref())
        .await
        .map_err(|error| format!("在线复核失败：{error}"))?;
    let host = version(&app.package_info().version.to_string()).ok_or("宿主版本无效")?;
    let release = catalog
        .widgets
        .iter()
        .find(|entry| entry.id == package_id)
        .and_then(|entry| latest(entry, host))
        .filter(|release| release.sha256 == expected_sha256)
        .ok_or("选中的版本已变化、撤回或不兼容，请刷新目录后重新确认")?;
    let bytes = download(&network, release).await?;
    let release = release.clone();
    crate::packages::install_verified(&app, bytes, expected_local_digest, move |manifest| {
        check_manifest(manifest, &package_id, &release)
    })
    .await?;
    // 登记成功后仍返回真实结果；管理页也会在安装结束时主动刷新。
    let _ = app.emit("host:changed", ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn release(ver: &str, minimum: &str, status: ReleaseStatus) -> Release {
        Release {
            version: ver.into(),
            min_host_version: minimum.into(),
            capabilities: vec!["storage.instance".into()],
            release_notes: "更新说明".into(),
            status,
            download_url: format!(
                "{RELEASE_ROOT}widget-test-v{ver}/official.souldesk.test-{ver}.zip"
            ),
            size_bytes: 4,
            sha256: format!("{:x}", Sha256::digest(b"test")),
        }
    }

    fn data(revision: u64) -> Catalog {
        use ReleaseStatus::{Published, Withdrawn};
        Catalog {
            schema_version: 1,
            revision,
            widgets: vec![Entry {
                id: "official.souldesk.test".into(),
                name: "测试组件".into(),
                summary: "测试组件".into(),
                category: "效率".into(),
                versions: vec![
                    release("0.1.0", "0.5.2", Published),
                    release("0.2.0", "0.6.0", Published),
                    release("0.3.0", "0.5.2", Withdrawn),
                ],
            }],
        }
    }

    fn signed(raw: String, pair: &Ed25519KeyPair) -> SignedCatalog {
        let signature = STANDARD.encode(pair.sign(raw.as_bytes()).as_ref());
        SignedCatalog { raw, signature }
    }

    #[test]
    fn catalog_signature_and_revision_are_checked_before_display() {
        let pair = Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap();
        let key = pair.public_key().as_ref().try_into().unwrap();
        let old = signed(serde_json::to_string(&data(2)).unwrap(), &pair);
        let accepted = verify(&old, key).unwrap();
        let cache = (old, accepted);
        let newer = signed(serde_json::to_string(&data(3)).unwrap(), &pair);
        assert!(verify_revision(Some(&cache), &newer, &verify(&newer, key).unwrap()).is_ok());
        let older = signed(serde_json::to_string(&data(1)).unwrap(), &pair);
        assert!(verify_revision(Some(&cache), &older, &verify(&older, key).unwrap()).is_err());
        let changed = signed(format!("{} ", cache.0.raw), &pair);
        assert!(verify_revision(Some(&cache), &changed, &verify(&changed, key).unwrap()).is_err());
        let tampered = SignedCatalog {
            raw: changed.raw,
            signature: cache.0.signature.clone(),
        };
        assert!(verify(&tampered, key).is_err());
    }

    #[test]
    fn older_host_selects_highest_compatible_published_version() {
        let catalog = data(4);
        let entry = &catalog.widgets[0];
        assert!(latest(entry, [0, 5, 1]).is_none());
        assert_eq!(latest(entry, [0, 5, 2]).unwrap().version, "0.1.0");
        assert_eq!(latest(entry, [0, 6, 0]).unwrap().version, "0.2.0");
    }

    #[test]
    fn zip_size_and_digest_are_both_required() {
        let version = release("0.1.0", "0.5.2", ReleaseStatus::Published);
        assert!(validate_bytes(&version, b"test").is_ok());
        assert!(validate_bytes(&version, b"x").unwrap_err().contains("大小"));
        assert!(validate_bytes(&version, b"fail")
            .unwrap_err()
            .contains("SHA-256"));
    }

    #[tokio::test]
    async fn interrupted_download_cannot_return_archive() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(socket.read_u8().await.unwrap());
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nte")
                .await
                .unwrap();
            socket.shutdown().await.unwrap();
        });
        let response = Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(url)
            .send()
            .await
            .unwrap();
        let version = release("0.1.0", "0.5.2", ReleaseStatus::Published);
        assert!(read_zip(response, &version)
            .await
            .unwrap_err()
            .contains("中断"));
        server.await.unwrap();
    }
}
