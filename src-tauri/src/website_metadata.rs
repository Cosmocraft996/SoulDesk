//! 收藏夹按需读取网页标题和图标。每次连接只使用已经核验的公网地址。
use crate::{content::web_url, store::AppResult};
use base64::{engine::general_purpose::STANDARD, Engine};
use dom_query::Document;
use encoding_rs::{Encoding, UTF_8};
use reqwest::{header, redirect::Policy, Response, StatusCode};
use serde_json::{json, Value};
use std::{net::IpAddr, time::Duration};
use tokio::{net::lookup_host, sync::Semaphore, time::timeout};
use url::{Host, Url};

const UNAVAILABLE: &str = "DATA_UNAVAILABLE: 无法读取网站信息";
const PUBLIC_ONLY: &str = "INVALID_ARGUMENT: 只读取公网 HTTP 或 HTTPS 网站";
const BROWSER_CHALLENGE: &str = "BROWSER_VERIFICATION: 网站要求浏览器验证";
const HEAD_LIMIT: usize = 512 * 1024;
const ICON_LIMIT: usize = 128 * 1024;
static LOOKUPS: Semaphore = Semaphore::const_new(4);

pub(crate) async fn lookup(args: Value) -> AppResult<Value> {
    let address = args
        .as_object()
        .filter(|fields| fields.len() == 1)
        .and_then(|fields| fields.get("url"))
        .and_then(Value::as_str)
        .ok_or("INVALID_ARGUMENT: 网站信息请求只接受网址")?;
    let url = public_url(web_url(address)?)?;
    let _permit = LOOKUPS
        .try_acquire()
        .map_err(|_| "BUSY: 网站信息读取繁忙")?;
    let (page_url, title, icon_url, challenged) =
        match timeout(Duration::from_secs(4), read_page(url.clone())).await {
            Ok(Ok((page_url, title, icon_url))) => (page_url, title, icon_url, false),
            Ok(Err(error)) => (url, None, None, error == BROWSER_CHALLENGE),
            Err(_) => (url, None, None, false),
        };
    let icon = timeout(Duration::from_secs(2), read_icon(&page_url, icon_url))
        .await
        .ok()
        .flatten();
    let status = match (challenged, title.is_some(), icon.is_some()) {
        (true, _, _) => "verification",
        (_, true, true) => "complete",
        (_, false, false) => "unavailable",
        _ => "partial",
    };
    Ok(json!({ "title": title, "icon": icon, "status": status }))
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => !matches!(
            ip.octets(),
            [0 | 10 | 127 | 224..=255, ..]
                | [100, 64..=127, ..]
                | [169, 254, ..]
                | [172, 16..=31, ..]
                | [192, 0, 0 | 2, _]
                | [192, 88, 99, _]
                | [192, 168, ..]
                | [198, 18 | 19, ..]
                | [198, 51, 100, _]
                | [203, 0, 113, _]
        ),
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            // 仅接受全球单播范围，排除转换、隧道和文档用途等保留网段。
            segments[0] & 0xe000 == 0x2000
                && !matches!(
                    segments,
                    [0x2001, 0..=0x01ff | 0x0db8, ..] | [0x2002 | 0x3fff, ..]
                )
        }
    }
}

pub(crate) fn public_url(mut url: Url) -> AppResult<Url> {
    let host_allowed = match url.host() {
        Some(Host::Domain(domain)) => {
            let domain = domain.trim_end_matches('.');
            domain.contains('.')
                && domain != "home.arpa"
                && ![".localhost", ".local", ".internal", ".home.arpa", ".onion"]
                    .iter()
                    .any(|suffix| domain.ends_with(suffix))
        }
        Some(Host::Ipv4(ip)) => public_ip(ip.into()),
        Some(Host::Ipv6(ip)) => public_ip(ip.into()),
        None => false,
    };
    if !host_allowed
        || !matches!(url.scheme(), "https" | "http")
        || url.port() == Some(0)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.as_str().len() > 4096
    {
        return Err(PUBLIC_ONLY.into());
    }
    url.set_fragment(None);
    Ok(url)
}

pub(crate) async fn public_addresses(url: &Url) -> AppResult<Vec<std::net::SocketAddr>> {
    let addresses = lookup_host((
        url.host_str().ok_or(PUBLIC_ONLY)?.trim_matches(['[', ']']),
        url.port_or_known_default().ok_or(PUBLIC_ONLY)?,
    ))
    .await
    .map_err(|_| UNAVAILABLE)?
    .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|addr| !public_ip(addr.ip())) {
        return Err(PUBLIC_ONLY.into());
    }
    Ok(addresses)
}

fn linked_target(url: Url, page: &Url) -> AppResult<Url> {
    let url = public_url(url)?;
    if url.port().is_some() && url.origin() != page.origin() {
        return Err(PUBLIC_ONLY.into());
    }
    Ok(url)
}

async fn get(mut url: Url, accept: &'static str) -> AppResult<(Url, Response)> {
    let initial = url.clone();
    for hop in 0..=3 {
        url = public_url(url)?;
        let mut builder = reqwest::Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .user_agent("SoulDesk Website Favorites");
        if let Some(domain) = url.domain() {
            // 域名保持用于 HTTP Host 与 TLS 校验，套接字使用这次核验过的 DNS 结果。
            let addresses = public_addresses(&url).await?;
            builder = builder.resolve_to_addrs(domain, &addresses);
        }
        let response = builder
            .build()
            .map_err(|_| UNAVAILABLE)?
            .get(url.clone())
            .header(header::ACCEPT, accept)
            .send()
            .await
            .map_err(|_| UNAVAILABLE)?;
        if response
            .headers()
            .get("cf-mitigated")
            .is_some_and(|value| value == "challenge")
        {
            return Err(BROWSER_CHALLENGE.into());
        }
        if response.status().is_success() {
            return Ok((url, response));
        }
        if hop == 3
            || !matches!(
                response.status(),
                StatusCode::MOVED_PERMANENTLY
                    | StatusCode::FOUND
                    | StatusCode::SEE_OTHER
                    | StatusCode::TEMPORARY_REDIRECT
                    | StatusCode::PERMANENT_REDIRECT
            )
        {
            break;
        }
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or(UNAVAILABLE)?;
        url = linked_target(url.join(location).map_err(|_| UNAVAILABLE)?, &initial)?;
    }
    Err(UNAVAILABLE.into())
}

fn content_type(response: &Response) -> &str {
    response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
}

async fn read_page(url: Url) -> AppResult<(Url, Option<String>, Option<Url>)> {
    let (url, mut response) = get(url, "text/html, application/xhtml+xml").await?;
    let content_type = content_type(&response);
    let mime = content_type.split(';').next().unwrap_or("").trim();
    if !mime.is_empty()
        && !mime.eq_ignore_ascii_case("text/html")
        && !mime.eq_ignore_ascii_case("application/xhtml+xml")
    {
        return Ok((url, None, None));
    }
    let encoding = declared_encoding(content_type);
    let mut bytes = Vec::new();
    while bytes.len() < HEAD_LIMIT {
        let Some(chunk) = response.chunk().await.map_err(|_| UNAVAILABLE)? else {
            break;
        };
        bytes.extend_from_slice(&chunk[..chunk.len().min(HEAD_LIMIT - bytes.len())]);
    }
    let (title, icon) = page_fields(&bytes, encoding, &url);
    Ok((url, title, icon))
}

fn declared_encoding(content_type: &str) -> Option<&'static Encoding> {
    content_type.split(';').skip(1).find_map(|part| {
        let (key, value) = part.split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("charset")
            .then(|| encoding_label(value))
            .flatten()
    })
}

fn encoding_label(label: &str) -> Option<&'static Encoding> {
    Encoding::for_label(label.trim().trim_matches(['"', '\'']).as_bytes())
}

fn page_fields(
    bytes: &[u8],
    header_encoding: Option<&'static Encoding>,
    url: &Url,
) -> (Option<String>, Option<Url>) {
    let encoding = header_encoding
        .or_else(|| {
            let head = String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]);
            let doc = Document::from(head.as_ref());
            doc.select_single("head meta[charset]")
                .attr("charset")
                .and_then(|label| encoding_label(&label))
                .or_else(|| {
                    doc.select_single("head meta[http-equiv='content-type' i]")
                        .attr("content")
                        .and_then(|content| declared_encoding(&content))
                })
        })
        .unwrap_or(UTF_8);
    let (html, _, _) = encoding.decode(bytes);
    let doc = Document::from(html.as_ref());
    let title = clean_title(&doc.select_single("head title").text()).or_else(|| {
        doc.select_single("head meta[property='og:title' i]")
            .attr("content")
            .and_then(|value| clean_title(&value))
    });
    let base = doc
        .select_single("head base[href]")
        .attr("href")
        .and_then(|href| url.join(href.trim()).ok())
        .filter(|base| matches!(base.scheme(), "https" | "http"))
        .unwrap_or_else(|| url.clone());
    let icons = doc.select("head link[rel~='icon' i][href]");
    let touch_icons = doc.select("head link[rel~='apple-touch-icon' i][href]");
    let icon = icons
        .nodes()
        .iter()
        .rev()
        .chain(touch_icons.nodes().iter().rev())
        .filter_map(|node| node.attr("href"))
        .filter(|href| !href.trim().is_empty() && !href.trim().starts_with('#'))
        .find_map(|href| {
            base.join(href.trim())
                .ok()
                .and_then(|icon| linked_target(icon, url).ok())
        });
    (title, icon)
}

pub(crate) fn clean_title(value: &str) -> Option<String> {
    let title = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut remaining = 64_u8;
    let title: String = title
        .chars()
        .filter(|ch| !ch.is_control())
        .map_while(|ch| {
            remaining = remaining.checked_sub(ch.len_utf16() as u8)?;
            Some(ch)
        })
        .collect();
    (!title.is_empty()).then_some(title)
}

async fn read_icon(page_url: &Url, icon: Option<Url>) -> Option<String> {
    let fallback = page_url.join("/favicon.ico").ok()?;
    let urls = icon
        .filter(|url| url != &fallback)
        .into_iter()
        .chain([fallback]);
    for url in urls {
        if let Ok(icon) = download_icon(url).await {
            return Some(icon);
        }
    }
    None
}

async fn download_icon(url: Url) -> AppResult<String> {
    let (_, mut response) = get(url, "image/*").await?;
    let mime = match content_type(&response)
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/png" => "image/png",
        "image/webp" => "image/webp",
        "image/svg+xml" => "image/svg+xml",
        "image/jpeg" => "image/jpeg",
        "image/gif" => "image/gif",
        "image/avif" => "image/avif",
        "image/ico" | "image/icon" | "image/x-icon" | "image/vnd.microsoft.icon" => "image/x-icon",
        "" | "application/octet-stream" => "",
        _ => return Err(UNAVAILABLE.into()),
    };
    if response
        .content_length()
        .is_some_and(|len| len > ICON_LIMIT as u64)
    {
        return Err(UNAVAILABLE.into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| UNAVAILABLE)? {
        if chunk.len() > ICON_LIMIT - bytes.len() {
            return Err(UNAVAILABLE.into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let mime = match (mime, bytes.as_slice()) {
        ("", [0, 0, 1, 0, ..]) => "image/x-icon",
        ("", [137, b'P', b'N', b'G', 13, 10, 26, 10, ..]) => "image/png",
        ("", _) | (_, []) => return Err(UNAVAILABLE.into()),
        (mime, _) => mime,
    };
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_title_and_icon_follow_the_page_base() {
        let url = Url::parse("https://example.com/articles/start").unwrap();
        let html = br#"
            <head><base href="../assets/"><title> Web &amp; Docs </title>
            <link rel="shortcut ICON" href="icons/brand.png">
            <link rel="apple-touch-icon" href="/large.png">
            <link rel="icon" href="file:///favicon.ico"></head>
            <body><svg><title>Decorative icon</title></svg></body>
        "#;
        let (title, icon) = page_fields(html, None, &url);
        assert_eq!(title.as_deref(), Some("Web & Docs"));
        assert_eq!(
            icon.unwrap().as_str(),
            "https://example.com/assets/icons/brand.png"
        );
    }

    #[test]
    fn legacy_chinese_charset_and_empty_titles_have_safe_fallbacks() {
        let url = Url::parse("https://example.com/").unwrap();
        let html =
            r#"<head><meta charset="gbk"><meta property="og:title" content="中文文章"></head>"#;
        let (bytes, _, _) = encoding_rs::GBK.encode(html);
        let (title, icon) = page_fields(&bytes, None, &url);
        assert_eq!(title.as_deref(), Some("中文文章"));
        assert!(icon.is_none());
        assert_eq!(
            clean_title(&"🌍".repeat(40))
                .unwrap()
                .encode_utf16()
                .count(),
            64
        );
    }

    #[test]
    fn page_and_icon_targets_require_public_addresses() {
        for input in [
            "http://localhost/",
            "http://home.arpa/",
            "https://box.local./",
            "https://example.com:0/icon.png",
            "http://127.0.0.1/",
            "http://172.16.1.1/",
            "http://169.254.169.254/",
            "http://100.64.1.1/",
            "http://[::ffff:127.0.0.1]/",
            "http://[2002:7f00:1::1]/",
        ] {
            assert!(public_url(Url::parse(input).unwrap()).is_err(), "{input}");
        }
        for input in [
            "https://example.com/#fragment",
            "https://example.com:8443/icon.png",
            "http://8.8.8.8/",
            "http://8.8.8.8:3000/",
            "https://[2606:4700:4700::1111]/",
        ] {
            assert!(public_url(Url::parse(input).unwrap())
                .unwrap()
                .fragment()
                .is_none());
        }
    }

    #[test]
    fn linked_custom_ports_stay_on_the_pages_origin() {
        let page = Url::parse("http://example.com:3000/").unwrap();
        assert!(linked_target(page.join("/favicon.svg").unwrap(), &page).is_ok());
        assert!(linked_target(
            Url::parse("https://cdn.example.com/icon.png").unwrap(),
            &page
        )
        .is_ok());
        assert!(linked_target(Url::parse("http://example.com:3001/").unwrap(), &page).is_err());
        assert!(
            linked_target(Url::parse("http://other.example.com:3000/").unwrap(), &page).is_err()
        );
    }

    #[tokio::test]
    #[ignore = "需要可访问的公网验收网址"]
    async fn public_website_returns_title_and_image() {
        let url = std::env::var("SOULDESK_WEBSITE_TEST_URL").unwrap();
        let title = std::env::var("SOULDESK_WEBSITE_TEST_TITLE").unwrap();
        let result = lookup(json!({ "url": url })).await.unwrap();
        assert_eq!(result["title"].as_str(), Some(title.as_str()));
        assert!(result["icon"]
            .as_str()
            .is_some_and(|icon| icon.starts_with("data:image/")));
    }
}
