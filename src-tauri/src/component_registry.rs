//! 固定能力和请求归属的单一登记；登记不等于授权，各网关继续执行实例/会话校验。

pub(crate) const LEGACY_CONTENT_READ: &str = "content.read";
pub(crate) const HTTPS_IMAGES: &str = "images.https";
pub(crate) const OPEN_WEB_LINK: &str = "links.openWeb";
pub(crate) const WEBSITE_METADATA: &str = "links.metadata";
pub(crate) const WEBSITE_VERIFY: &str = "links.verify";

const CONTENT_SOURCE_CAPABILITIES: &[(&str, &str)] = &[
    ("bangumi", "content.source.bangumi"),
    ("bilibili", "content.source.bilibili"),
    ("bilibili-rcmd", "content.source.bilibili-rcmd"),
    ("cls-telegraph", "content.source.cls-telegraph"),
    ("douyin", "content.source.douyin"),
    ("tou-tiao", "content.source.tou-tiao"),
    ("weibo", "content.source.weibo"),
    ("zhihu", "content.source.zhihu"),
    ("itime-calendar", "content.source.itime-calendar"),
    ("forex", "content.source.forex"),
    ("forex-rates", "content.source.forex"),
    ("metal", "content.source.metal"),
    ("gold", "content.source.gold"),
    ("gold-trends", "content.source.gold-trends"),
    ("a-index", "content.source.a-index"),
    ("coin-index", "content.source.coin-index"),
    ("stock-search", "content.source.stock-search"),
    ("stock-minute", "content.source.stock-minute"),
    (
        "stock-search-eastmoney",
        "content.source.stock-search-eastmoney",
    ),
    ("stock-quote", "content.source.stock-quote"),
];

pub(crate) fn content_source_capability(source: &str) -> Option<&'static str> {
    CONTENT_SOURCE_CAPABILITIES
        .iter()
        .find_map(|&(name, capability)| (name == source).then_some(capability))
}

pub(crate) fn is_scoped_content_capability(name: &str) -> bool {
    name == HTTPS_IMAGES
        || CONTENT_SOURCE_CAPABILITIES
            .iter()
            .any(|&(_, capability)| capability == name)
}

pub(crate) const CAPABILITIES: &[&str] = &[
    "storage.instance",
    "storage.shared",
    "storage.document",
    "documents.user",
    "documents.open",
    "documents.save",
    "documents.drop",
    "media.directory",
    "files.launcher",
    "assets.local",
    crate::asset_permissions::SCOPED,
    crate::asset_permissions::STYLES,
    crate::asset_permissions::IMAGES,
    crate::asset_permissions::FETCH,
    crate::asset_permissions::WASM,
    crate::asset_permissions::EVAL,
    crate::asset_permissions::WORKER,
    crate::asset_permissions::BLOB_WORKER,
    "web.embed",
    "clipboard.read",
    "clipboard.write",
    crate::clipboard_host::CAPABILITY,
    crate::clipboard_host::CONTROL,
    crate::clipboard_host::HISTORY_ACCESS,
    crate::clipboard_host::COPY,
    crate::clipboard_host::PASTE,
    crate::clipboard_host::SEARCH_ACCESS,
    crate::clipboard_host::TOOLS,
    "assistant.web",
    "jobs.manage",
    "market.read",
    "market.chart",
    crate::binance_account::READ,
    crate::binance_account::TRADE,
    "alerts.manage",
    LEGACY_CONTENT_READ,
    "content.live",
    "weather.read",
    "ai.generate",
    "system.metrics",
    "reminders.water",
    "reminders.labor",
    "reminders.sit",
    "reminders.phone",
    "notifications.island",
    crate::notification_sdk::SEND,
    crate::notification_sdk::CALL,
    "todos.manage",
    "focus.pomodoro",
    "focus.pomodoro.window",
    "keyboard.display",
    "servers.monitor",
    crate::cloud_workbench::CAPABILITY,
    crate::cloud_workbench::WAKE,
    crate::cloud_workbench::MANAGE,
    "links.open",
    OPEN_WEB_LINK,
    WEBSITE_METADATA,
    WEBSITE_VERIFY,
];

pub(crate) fn known_capability(name: &str) -> bool {
    CAPABILITIES.contains(&name) || is_scoped_content_capability(name)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gateway {
    PomodoroWindow,
    Jobs,
    Reminders,
    Keyboard,
    Assistant,
    Details,
    CoinLive,
    Weather,
    BinanceAccount,
    Ai,
    Clipboard,
    Metrics,
    Server,
    CloudWorkbench,
    Labor,
    Water,
    Todo,
    Pomodoro,
    Content,
    Documents,
    Media,
    Files,
    Market,
    Standard,
}

/// 精确方法先匹配；较具体的番茄专用窗前缀先于普通番茄业务。
pub(crate) fn gateway(method: &str) -> Gateway {
    use Gateway::*;
    for (name, owner) in [
        ("view.openDetails", Details),
        ("view.resizeMarket", Details),
        ("content.live", CoinLive),
        ("system.metrics", Metrics),
        ("server.request", Server),
        ("cloudWorkbench.summary", CloudWorkbench),
        ("cloudWorkbench.openConsole", CloudWorkbench),
        ("cloudWorkbench.details", CloudWorkbench),
        ("cloudWorkbench.history", CloudWorkbench),
        ("cloudWorkbench.refresh", CloudWorkbench),
        ("cloudWorkbench.wake", CloudWorkbench),
        ("cloudWorkbench.openService", CloudWorkbench),
        ("labor.get", Labor),
        ("notifications.send", Reminders),
    ] {
        if method == name {
            return owner;
        }
    }
    for (prefix, owner) in [
        ("cloudWorkbench.manage.", CloudWorkbench),
        ("pomodoro.window.", PomodoroWindow),
        ("jobs.", Jobs),
        ("island.", Reminders),
        ("sit.", Reminders),
        ("phone.", Reminders),
        ("keyboard.", Keyboard),
        ("assistant.", Assistant),
        ("weather.", Weather),
        ("binanceAccount.", BinanceAccount),
        ("ai.", Ai),
        ("clipboard.", Clipboard),
        ("water.", Water),
        ("todo.", Todo),
        ("pomodoro.", Pomodoro),
        ("content.", Content),
        ("links.", Content),
        ("documents.", Documents),
        ("media.", Media),
        ("files.", Files),
        ("market.", Market),
        ("watchlist.", Market),
        ("alerts.", Market),
    ] {
        if method.starts_with(prefix) {
            return owner;
        }
    }
    Standard
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_catalog_is_unique_and_unknown_names_remain_denied() {
        let scopes: std::collections::HashSet<_> = CONTENT_SOURCE_CAPABILITIES
            .iter()
            .map(|&(_, cap)| cap)
            .collect();
        let sources: std::collections::HashSet<_> = CONTENT_SOURCE_CAPABILITIES
            .iter()
            .map(|&(source, _)| source)
            .collect();
        // 多个内容入口可以有意共享同一权限，来源名称必须唯一。
        assert_eq!(sources.len(), CONTENT_SOURCE_CAPABILITIES.len());
        let names: std::collections::HashSet<_> = CAPABILITIES
            .iter()
            .copied()
            .chain(scopes.iter().copied())
            .chain([HTTPS_IMAGES])
            .collect();
        assert_eq!(names.len(), CAPABILITIES.len() + scopes.len() + 1);
        assert!(known_capability("storage.instance"));
        assert!(!known_capability("shell.execute"));
        assert!(!known_capability("storage.*"));
        assert!(!known_capability("content.source.*"));
        assert!(known_capability("content.source.coin-index"));
        assert!(known_capability("notifications.send"));
        assert!(known_capability("notifications.call"));
    }

    #[test]
    fn special_routes_preserve_their_owner_without_expanding_method_permissions() {
        assert_eq!(
            gateway("pomodoro.window.configure"),
            Gateway::PomodoroWindow
        );
        assert_eq!(gateway("pomodoro.get"), Gateway::Pomodoro);
        assert_eq!(gateway("content.live"), Gateway::CoinLive);
        assert_eq!(gateway("content.get"), Gateway::Content);
        assert_eq!(gateway("content.live.other"), Gateway::Content);
        assert_eq!(gateway("ai.toolInfo"), Gateway::Ai);
        assert_eq!(gateway("clipboard.status"), Gateway::Clipboard);
        assert_eq!(gateway("jobs.search"), Gateway::Jobs);
        assert_eq!(gateway("island.configure"), Gateway::Reminders);
        assert_eq!(gateway("notifications.send"), Gateway::Reminders);
        assert_eq!(gateway("phone.configure"), Gateway::Reminders);
        assert_eq!(gateway("watchlist.get"), Gateway::Market);
        assert_eq!(gateway("alerts.save"), Gateway::Market);
        assert_eq!(gateway("view.openDetails"), Gateway::Details);
        assert_eq!(gateway("settings.get"), Gateway::Standard);
        assert_eq!(gateway("library.bind"), Gateway::Standard);
        assert_eq!(gateway("shell.execute"), Gateway::Standard);
    }
}
