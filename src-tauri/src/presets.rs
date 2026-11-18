//! 预置组件复用独立包源码，首次添加时仍经过正常校验和受限试加载。
use crate::store::{err, AppResult};
use serde_json::{json, Value};
use std::io::{Cursor, Write};

pub struct Preset {
    manifest: &'static str,
    description: &'static str,
    symbol: &'static str,
    files: &'static [(&'static str, &'static [u8])],
    prebuilt: Option<&'static [u8]>,
}

// 两个进度组件共享移植后的静态资源，最终仍是各自独立的标准组件包。
macro_rules! widgetjs_progress {
    ($directory:literal, $description:literal, $symbol:literal) => {
        Preset {
            manifest: include_str!(concat!(
                "../../components/widgetjs/",
                $directory,
                "/manifest.json"
            )),
            description: $description,
            symbol: $symbol,
            prebuilt: None,
            files: &[
                (
                    "dist/index.html",
                    include_bytes!(concat!(
                        "../../components/widgetjs/",
                        $directory,
                        "/index.html"
                    )),
                ),
                (
                    "dist/SOURCE.txt",
                    include_bytes!(concat!(
                        "../../components/widgetjs/",
                        $directory,
                        "/SOURCE.txt"
                    )),
                ),
                (
                    "dist/LICENSE.txt",
                    include_bytes!("../../components/widgetjs/progress-common/LICENSE.txt"),
                ),
                (
                    "dist/app.js",
                    include_bytes!("../../components/widgetjs/progress-common/app.js"),
                ),
                (
                    "dist/periods.js",
                    include_bytes!("../../components/widgetjs/progress-common/periods.js"),
                ),
                (
                    "dist/style.css",
                    include_bytes!("../../components/widgetjs/progress-common/style.css"),
                ),
            ],
        }
    };
}

macro_rules! widgetjs_clock {
    ($directory:literal, $description:literal, $symbol:literal) => {
        Preset {
            manifest: include_str!(concat!(
                "../../components/widgetjs/",
                $directory,
                "/manifest.json"
            )),
            description: $description,
            symbol: $symbol,
            prebuilt: None,
            files: &[
                (
                    "dist/index.html",
                    include_bytes!(concat!(
                        "../../components/widgetjs/",
                        $directory,
                        "/index.html"
                    )),
                ),
                (
                    "dist/SOURCE.txt",
                    include_bytes!(concat!(
                        "../../components/widgetjs/",
                        $directory,
                        "/SOURCE.txt"
                    )),
                ),
                (
                    "dist/app.js",
                    include_bytes!("../../components/widgetjs/clock-bundle/app.js"),
                ),
                (
                    "dist/style.css",
                    include_bytes!("../../components/widgetjs/clock-bundle/style.css"),
                ),
            ],
        }
    };
}

macro_rules! widgetjs_hotspot {
    ($directory:literal, $description:literal, $symbol:literal) => {
        Preset {
            manifest: include_str!(concat!(
                "../../components/widgetjs/",
                $directory,
                "/manifest.json"
            )),
            description: $description,
            symbol: $symbol,
            prebuilt: None,
            files: &[
                (
                    "dist/index.html",
                    include_bytes!(concat!(
                        "../../components/widgetjs/",
                        $directory,
                        "/index.html"
                    )),
                ),
                (
                    "dist/SOURCE.txt",
                    include_bytes!(concat!(
                        "../../components/widgetjs/",
                        $directory,
                        "/SOURCE.txt"
                    )),
                ),
                (
                    "dist/LICENSE.txt",
                    include_bytes!("../../components/widgetjs/hotspot-source/LICENSE.txt"),
                ),
                (
                    "dist/app.js",
                    include_bytes!("../../components/widgetjs/hotspot-bundle/app.js"),
                ),
                (
                    "dist/style.css",
                    include_bytes!("../../components/widgetjs/hotspot-bundle/style.css"),
                ),
            ],
        }
    };
}

// 无编译器依赖的独立包，逐个列出实际交付资源。
macro_rules! widgetjs_static {
    ($directory:literal, $description:literal, $symbol:literal, [$($file:literal),*]) => {
        widgetjs_static!($directory,$description,$symbol,"dist/",[$($file),*],[])
    };
    ($directory:literal, $description:literal, $symbol:literal, $prefix:literal, [$($file:literal),*],[$($path:literal => $source:literal),*]) => {
        Preset {
            manifest: include_str!(concat!("../../components/widgetjs/", $directory, "/manifest.json")),
            description: $description,
            symbol: $symbol,
            prebuilt: None,
            files: &[
                (concat!($prefix,"index.html"), include_bytes!(concat!("../../components/widgetjs/", $directory, "/index.html"))),
                (concat!($prefix,"SOURCE.txt"), include_bytes!(concat!("../../components/widgetjs/", $directory, "/SOURCE.txt"))),
                $(
                    (concat!($prefix, $file), include_bytes!(concat!("../../components/widgetjs/", $directory, "/", $file))),
                )*
                $(
                    ($path, include_bytes!(concat!("../../components/widgetjs/", $source))),
                )*
            ],
        }
    };
}

macro_rules! widgetjs_photo {
    ($directory:literal, $description:literal, $symbol:literal) => {
        Preset {
            manifest: include_str!(concat!(
                "../../components/widgetjs/",
                $directory,
                "/manifest.json"
            )),
            description: $description,
            symbol: $symbol,
            prebuilt: None,
            files: &[
                (
                    "dist/index.html",
                    include_bytes!(concat!(
                        "../../components/widgetjs/",
                        $directory,
                        "/index.html"
                    )),
                ),
                (
                    "dist/SOURCE.txt",
                    include_bytes!(concat!(
                        "../../components/widgetjs/",
                        $directory,
                        "/SOURCE.txt"
                    )),
                ),
                (
                    "dist/app.js",
                    include_bytes!(concat!("../../components/widgetjs/", $directory, "/app.js")),
                ),
                (
                    "dist/style.css",
                    include_bytes!(concat!(
                        "../../components/widgetjs/",
                        $directory,
                        "/style.css"
                    )),
                ),
                (
                    "dist/model.js",
                    include_bytes!("../../components/widgetjs/photo-common/model.js"),
                ),
                (
                    "dist/verify.js",
                    include_bytes!("../../components/widgetjs/photo-common/verify.js"),
                ),
                (
                    "dist/fixture.json",
                    include_bytes!("../../components/widgetjs/photo-common/fixture.json"),
                ),
            ],
        }
    };
}

macro_rules! widgetjs_weather {
    ($directory:literal, $description:literal) => {
        Preset {
            manifest: include_str!(concat!(
                "../../components/widgetjs/",
                $directory,
                "/manifest.json"
            )),
            description: $description,
            symbol: "☀",
            prebuilt: None,
            files: &[
                (
                    "dist/index.html",
                    include_bytes!(concat!(
                        "../../components/widgetjs/",
                        $directory,
                        "/index.html"
                    )),
                ),
                (
                    "dist/SOURCE.txt",
                    include_bytes!(concat!(
                        "../../components/widgetjs/",
                        $directory,
                        "/SOURCE.txt"
                    )),
                ),
                (
                    "dist/app.js",
                    include_bytes!("../../components/widgetjs/weather-bundle/app.js"),
                ),
                (
                    "dist/style.css",
                    include_bytes!("../../components/widgetjs/weather-bundle/style.css"),
                ),
            ],
        }
    };
}

macro_rules! widgetjs_reminder {
    ($directory:literal,$description:literal,$symbol:literal) => {widgetjs_static!($directory,$description,$symbol,"dist/",[],[
        "dist/app.js" => "reminder-bundle/app.js",
        "dist/style.css" => "reminder-bundle/style.css",
        "dist/LICENSE.txt" => "reminder-source/LICENSE.txt",
        "dist/hangup.m4a" => "reminder-bundle/hangup.m4a",
        "dist/ringtone.m4a" => "reminder-bundle/ringtone.m4a",
        "dist/squidward.m4a" => "reminder-bundle/squidward.m4a",
        "dist/elvis.mp3" => "reminder-bundle/elvis.mp3",
        "dist/mrs_pan.m4a" => "reminder-bundle/mrs_pan.m4a",
        "dist/jackie_chan.wav" => "reminder-bundle/jackie_chan.wav",
        "dist/zhangyuge.jpg" => "reminder-bundle/zhangyuge.jpg",
        "dist/elvis.png" => "reminder-bundle/elvis.png",
        "dist/mrs_pan.jpg" => "reminder-bundle/mrs_pan.jpg",
        "dist/jackie_chan.jpg" => "reminder-bundle/jackie_chan.jpg"
    ])};
}
// 动态分块和资源由同一构建入口封装，原生预置直接使用完整独立包。
macro_rules! widgetjs_role {
    ($directory:literal, $family:literal, $description:literal, $symbol:literal) => {
        Preset {
            manifest: include_str!(concat!(
                "../../components/widgetjs/",
                $directory,
                "/manifest.json"
            )),
            description: $description,
            symbol: $symbol,
            files: &[],
            prebuilt: Some(include_bytes!(concat!(
                "../../components/widgetjs/",
                $family,
                "-bundle/",
                $directory,
                ".zip"
            ))),
        }
    };
}
macro_rules! widgetjs_ai {
    ($directory:literal, $description:literal, $symbol:literal) => {
        widgetjs_role!($directory, "ai", $description, $symbol)
    };
}
macro_rules! widgetjs_clipboard {
    ($directory:literal, $description:literal, $symbol:literal) => {
        widgetjs_role!($directory, "clipboard", $description, $symbol)
    };
}
pub const PRESETS: &[Preset] = &[
    widgetjs_static!("ai-job","原BOSS手动登录、职位采集与评分、分析/百科/导图/面试；自填模型，后台默认关闭。","⌕","dist/",[],[
        "dist/app.js"=>"job-bundle/app.js",
        "dist/style.css"=>"job-bundle/style.css",
        "dist/UPSTREAM_NOTICES.txt"=>"job-source/UPSTREAM_NOTICES.txt",
        "dist/THIRD_PARTY_LICENSES.txt"=>"job-bundle/THIRD_PARTY_LICENSES.txt",
        "dist/tencent-BiqmWx96.png"=>"job-bundle/tencent-BiqmWx96.png",
        "dist/qian_wen-D4M3cMdw.png"=>"job-bundle/qian_wen-D4M3cMdw.png",
        "dist/deepseek-D6inQfCg.png"=>"job-bundle/deepseek-D6inQfCg.png",
        "dist/spark-BQm4DXkW.png"=>"job-bundle/spark-BQm4DXkW.png",
        "dist/open-book-WP0Z2C7s.png"=>"job-bundle/open-book-WP0Z2C7s.png",
        "dist/page_with_curl_3d-OYr4Aw17.png"=>"job-bundle/page_with_curl_3d-OYr4Aw17.png",
        "dist/magnifying_glass_tilted_left_3d-QorywRbH.png"=>"job-bundle/magnifying_glass_tilted_left_3d-QorywRbH.png",
        "dist/robot_3d-fGSEHVzB.png"=>"job-bundle/robot_3d-fGSEHVzB.png",
        "dist/face_with_monocle_3d-D6ZXuWSF.png"=>"job-bundle/face_with_monocle_3d-D6ZXuWSF.png",
        "dist/exploding_head_3d-Cysscfe9.png"=>"job-bundle/exploding_head_3d-Cysscfe9.png"
    ]),
    widgetjs_ai!(
        "ai-assistant",
        "原八平台网页、1/2/4分屏、手动登录与独立侧边窗；后台和置顶默认关闭。",
        "🤖"
    ),
    widgetjs_clipboard!(
        "clipboard",
        "原文本历史、搜索/分页、收藏/标签和编辑；显式启用采集、快捷键与粘贴，默认关闭。",
        "▤"
    ),
    widgetjs_clipboard!(
        "clipboard-search",
        "原AI剪切板搜索条、三搜索平台、独立本地导图/翻译工具；自填模型，默认关闭。",
        "⌕"
    ),
    widgetjs_ai!(
        "ai-mindmap",
        "原AI思维导图、真实Markmap与节点展开/复制/PNG；自填模型连接，默认关闭。",
        "🧠"
    ),
    widgetjs_ai!(
        "ai-mermaid",
        "原11种图表、源码编辑与真实Mermaid/缩放/全屏/PNG；自填模型，默认关闭。",
        "▥"
    ),
    widgetjs_ai!(
        "ai-translate",
        "原20种语言双向翻译、流式结果/复制/主题与本地历史；自填模型，默认关闭。",
        "译"
    ),
    widgetjs_reminder!(
        "dynamic-island",
        "原胶囊通知、提醒、倒计时及来电显示；默认关闭，多实例共用顶部无焦点通知层。",
        "◕"
    ),
    widgetjs_reminder!(
        "sit-reminder",
        "原鼠标活动提醒及确认后的全屏休息；默认关闭，需启用灵动通知，Esc结束休息。",
        "⌛"
    ),
    widgetjs_reminder!(
        "phone-reminder",
        "四种原录音、头像、歌词、工作日与试听；默认关闭，需启用灵动通知。",
        "☎"
    ),
    widgetjs_static!(
        "key-stroke",
        "原彩色快捷键帽及CapsLock；显式启用后底部无焦点演示，松开4秒隐藏，默认关闭。",
        "⌨",
        "dist/",
        [],
        [
            "dist/app.js" => "key-stroke-bundle/app.js",
            "dist/style.css" => "key-stroke-bundle/style.css"
        ]
    ),
    widgetjs_static!(
        "stock",
        "原自选四列股票、三种指标及手工持仓，按实例保存；曲线仅示意，无交易账户。",
        "↗",
        "dist/",
        [],
        [
            "dist/app.js" => "stock-bundle/app.js",
            "dist/style.css" => "stock-bundle/style.css"
        ]
    ),
    widgetjs_weather!(
        "weather-small",
        "原小号和风天气，实时温度/风向风速/湿度；需配置自有凭据并启用。"
    ),
    widgetjs_weather!(
        "weather-medium",
        "原中号天气，实时观测、风/湿度/降水/气压；默认不联网。"
    ),
    widgetjs_weather!(
        "weather-large",
        "原大号天气、三日预报和生活指数；使用用户自己的和风服务。"
    ),
    Preset {
        manifest: include_str!("../../components/widgetjs/coin-medium/manifest.json"),
        description: "原14币种、OKX网页报价推送、精确涨跌、主题及本实例代理；原生共享连接。",
        symbol: "₿",
        prebuilt: None,
        files: &[
            (
                "dist/index.html",
                include_bytes!("../../components/widgetjs/coin-medium/index.html"),
            ),
            (
                "dist/SOURCE.txt",
                include_bytes!("../../components/widgetjs/coin-medium/SOURCE.txt"),
            ),
            (
                "dist/app.js",
                include_bytes!("../../components/widgetjs/coin-medium-bundle/app.js"),
            ),
            (
                "dist/style.css",
                include_bytes!("../../components/widgetjs/coin-medium-bundle/style.css"),
            ),
            (
                "dist/bitcoin.png",
                include_bytes!("../../components/widgetjs/coin-medium/bitcoin.png"),
            ),
            (
                "dist/ethereum.png",
                include_bytes!("../../components/widgetjs/coin-medium/ethereum.png"),
            ),
            (
                "dist/xrp.png",
                include_bytes!("../../components/widgetjs/coin-medium/xrp.png"),
            ),
            (
                "dist/bnb.png",
                include_bytes!("../../components/widgetjs/coin-medium/bnb.png"),
            ),
            (
                "dist/solana.png",
                include_bytes!("../../components/widgetjs/coin-medium/solana.png"),
            ),
            (
                "dist/tron.png",
                include_bytes!("../../components/widgetjs/coin-medium/tron.png"),
            ),
            (
                "dist/dogecoin.png",
                include_bytes!("../../components/widgetjs/coin-medium/dogecoin.png"),
            ),
            (
                "dist/sui.png",
                include_bytes!("../../components/widgetjs/coin-medium/sui.png"),
            ),
            (
                "dist/okb.png",
                include_bytes!("../../components/widgetjs/coin-medium/okb.png"),
            ),
            (
                "dist/trump.png",
                include_bytes!("../../components/widgetjs/coin-medium/trump.png"),
            ),
        ],
    },
    widgetjs_static!(
        "grid",
        "原文件收纳网格、系统缩略图/图标、目录与额外项目、排序和非破坏性排除。",
        "▦",
        [
            "app.js",
            "grid-model.js",
            "style.css",
            "file.ico",
            "file_fail.ico",
            "delete.png",
            "word.png",
            "excel.png",
            "ppt.png",
            "pdf.png",
            "image.png",
            "audio.png",
            "video.png"
        ]
    ),
    Preset {
        manifest: include_str!("../../examples/market/manifest.json"),
        description: "币安 USDT 永续自选行情、Last / Mark 价格、到价警报与触发历史。",
        symbol: "↗",
        prebuilt: None,
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
                "dist/compact.js",
                include_bytes!("../../examples/market/compact.js"),
            ),
            (
                "dist/price.js",
                include_bytes!("../../examples/market/price.js"),
            ),
        ],
    },
    widgetjs_progress!(
        "time-progress",
        "今天、本周、本月、今年的剩余时间百分比，可独立设置主题。",
        "◴"
    ),
    widgetjs_progress!(
        "wave-progress",
        "用动态波浪显示剩余进度，支持日、周、月、年及自定义事项。",
        "≈"
    ),
    widgetjs_clock!(
        "clock",
        "指针时钟，保留表盘与指针动画，可设置时区和时区标签。",
        "◷"
    ),
    widgetjs_clock!(
        "flip-clock",
        "带三组翻页动画的数字时钟，独立主题和实例设置。",
        "▦"
    ),
    widgetjs_clock!(
        "glitch-clock",
        "带故障动画的数字时钟，可点击切换故障效果。",
        "ϟ"
    ),
    widgetjs_clock!(
        "mickey-clock",
        "保留原有图形与手臂指针，随组件尺寸缩放。",
        "◴"
    ),
    widgetjs_static!(
        "energy-label",
        "原蓝白能耗标签、CPU/内存80/20权重五档、名称/型号、字体与等级动画；缺失数据不分级。",
        "▰",
        "dist/energy-label/",
        ["app.js", "energy-model.js", "style.css"],
        ["dist/hardware-common/metrics.js" => "hardware-common/metrics.js", "dist/hardware-common/verify.js" => "hardware-common/verify.js"]
    ),
    widgetjs_static!(
        "base-panel",
        "原四列环形监控面板、CPU/物理内存/同适配器GPU温度、网速与累计量；缺失及预热明确标识。",
        "◴",
        "dist/base-panel/",
        ["app.js", "panel-model.js", "style.css"],
        ["dist/hardware-common/metrics.js" => "hardware-common/metrics.js", "dist/hardware-common/verify.js" => "hardware-common/verify.js"]
    ),
    widgetjs_static!(
        "water-reminder",
        "原水波球、每日目标与杯数；核心后台定时提醒，隐藏后继续，Windows通知支持喝一杯。",
        "◒",
        [
            "app.js",
            "water-model.js",
            "wave-ball.js",
            "style.css",
            "verify.js"
        ]
    ),
    widgetjs_static!(
        "mindmap",
        "原AntV X6编辑、分组/表情/主题、串行保存与导入导出；可选本地共享，原云账户/同步及付费AI未接入。",
        "⑂",
        "dist/",
        [],
        [
            "dist/app.js" => "mindmap-bundle/app.js",
            "dist/style.css" => "mindmap-bundle/style.css",
            "dist/logo-B6MW8hNB.png" => "mindmap-bundle/logo-B6MW8hNB.png"
        ]
    ),
    widgetjs_static!(
        "server",
        "原服务器卡片、负载/内存/首块磁盘、IP/系统/到期信息、实例配置与主服务器同步；真实服务仍待验。",
        "▤",
        "dist/",
        [],
        [
            "dist/app.js" => "server-bundle/app.js",
            "dist/style.css" => "server-bundle/style.css",
            "dist/alibaba.png" => "server-bundle/alibaba.png",
            "dist/aws.png" => "server-bundle/aws.png",
            "dist/azure.png" => "server-bundle/azure.png",
            "dist/google.png" => "server-bundle/google.png",
            "dist/huawei.png" => "server-bundle/huawei.png",
            "dist/other.png" => "server-bundle/other.png",
            "dist/tencent.png" => "server-bundle/tencent.png",
            "dist/tianyi.png" => "server-bundle/tianyi.png",
            "dist/ucloud.png" => "server-bundle/ucloud.png",
            "dist/alibaba_cloud_linux.png" => "server-bundle/alibaba_cloud_linux.png",
            "dist/centos.png" => "server-bundle/centos.png",
            "dist/debian.png" => "server-bundle/debian.png",
            "dist/linux.png" => "server-bundle/linux.png",
            "dist/redhat.png" => "server-bundle/redhat.png",
            "dist/suse.png" => "server-bundle/suse.png",
            "dist/ubuntu.png" => "server-bundle/ubuntu.png",
            "dist/windows_server.png" => "server-bundle/windows_server.png"
        ]
    ),
    widgetjs_static!(
        "labor",
        "原进度条、11张表情和阶段飘字；上下班/夜班及核心5分钟静默更新倒计时。",
        "☺",
        "dist/",
        [],
        [
            "dist/app.js" => "labor-bundle/app.js",
            "dist/style.css" => "labor-bundle/style.css",
            "dist/LICENSE.txt" => "labor-source/LICENSE.txt",
            "dist/face_holding_back_tears_3d.png" => "labor-bundle/face_holding_back_tears_3d.png",
            "dist/face_with_rolling_eyes_3d.png" => "labor-bundle/face_with_rolling_eyes_3d.png",
            "dist/face_with_spiral_eyes_3d.png" => "labor-bundle/face_with_spiral_eyes_3d.png",
            "dist/face_with_steam_from_nose_3d.png" => "labor-bundle/face_with_steam_from_nose_3d.png",
            "dist/knocked-out_face_3d.png" => "labor-bundle/knocked-out_face_3d.png",
            "dist/partying_face_3d.png" => "labor-bundle/partying_face_3d.png",
            "dist/sleeping_face_3d.png" => "labor-bundle/sleeping_face_3d.png",
            "dist/sleepy_face_3d.png" => "labor-bundle/sleepy_face_3d.png",
            "dist/smiling_face_with_sunglasses_3d.png" => "labor-bundle/smiling_face_with_sunglasses_3d.png",
            "dist/star-struck_3d.png" => "labor-bundle/star-struck_3d.png",
            "dist/yawning_face_3d.png" => "labor-bundle/yawning_face_3d.png"
        ]
    ),
    widgetjs_clock!("text-clock", "时间、日期、星期与农历，支持设置字体。", "12"),
    widgetjs_static!(
        "note",
        "横线纸便签，自动保存，Ctrl＋滚轮调整字号。",
        "▤",
        ["app.js", "note-model.js", "style.css"]
    ),
    widgetjs_static!(
        "pomodoro",
        "原钟面、场景、统计与后台计时；可选共享库，专用置顶贴边窗及托盘默认关闭，原云同步未完成。",
        "◴",
        "dist/",
        [],
        [
            "dist/app.js" => "pomodoro-bundle/app.js",
            "dist/style.css" => "pomodoro-bundle/style.css",
            "dist/Rubik-Regular.ttf" => "pomodoro-bundle/Rubik-Regular.ttf",
            "dist/pomodoro-ding-BPamOesd.wav" => "pomodoro-bundle/pomodoro-ding-BPamOesd.wav"
        ]
    ),
    widgetjs_static!(
        "calendar-large",
        "原FullCalendar月视图、农历/公开节气、重复日程与编辑；默认私有，可选本地共享，原云账户/同步未接入。",
        "▦",
        "dist/",
        [],
        [
            "dist/app.js" => "calendar-large-bundle/app.js",
            "dist/style.css" => "calendar-large-bundle/style.css",
            "dist/ding-B9BOXEwg.mp3" => "calendar-large-bundle/ding-B9BOXEwg.mp3"
        ]
    ),
    widgetjs_static!(
        "todo",
        "原iTime待办列表、完成历史/音效、重复与核心提醒；默认私有，可选本地共享，原云账户/同步未接入。",
        "☑",
        "dist/",
        [],
        [
            "dist/app.js" => "todo-bundle/app.js",
            "dist/style.css" => "todo-bundle/style.css",
            "dist/ding-B9BOXEwg.mp3" => "todo-bundle/ding-B9BOXEwg.mp3"
        ]
    ),
    widgetjs_static!(
        "birthday-list",
        "原iTime气球生日列表、公历/农历、今日蛋糕和本地编辑；默认私有，可选本地共享，原云账户/同步未接入。",
        "♧",
        "dist/",
        [],
        [
            "dist/app.js" => "birthday-list-bundle/app.js",
            "dist/style.css" => "birthday-list-bundle/style.css",
            "dist/balloon-0HG1ynQp.png" => "birthday-list-bundle/balloon-0HG1ynQp.png"
        ]
    ),
    widgetjs_static!(
        "countdown",
        "原iTime纸张倒计时、日期/重复和本地事件编辑；默认私有，可选本地共享，原云账户/同步未接入。",
        "▣",
        "dist/",
        [],
        [
            "dist/app.js" => "countdown-bundle/app.js",
            "dist/style.css" => "countdown-bundle/style.css"
        ]
    ),
    widgetjs_static!(
        "countdown-list",
        "原iTime倒计时列表、编辑/拖动/排序和本地保存；默认私有，可选本地共享，原云账户/同步未接入。",
        "▤",
        "dist/",
        [],
        [
            "dist/app.js" => "countdown-bundle/app.js",
            "dist/style.css" => "countdown-bundle/style.css"
        ]
    ),
    widgetjs_static!(
        "calendar",
        "原iTime月历、农历/节日、休班和黄历详情；原固定公开源与实例主题。",
        "▦",
        "dist/",
        [],
        [
            "dist/app.js" => "calendar-bundle/app.js",
            "dist/style.css" => "calendar-bundle/style.css"
        ]
    ),
    widgetjs_static!(
        "deadline",
        "原Deadline SVG、期限进度、火焰/手臂动画与剩余天数；按实例保存日期和主题。",
        "⌛",
        "dist/",
        [],
        [
            "dist/app.js" => "deadline-bundle/app.js",
            "dist/style.css" => "deadline-bundle/style.css"
        ]
    ),
    widgetjs_hotspot!(
        "hotspot-bangumi",
        "B站追番时间表，日期选择、封面取色与点击打开。",
        "▦"
    ),
    widgetjs_hotspot!("hotspot-bilibili", "B站热搜榜单与热点跳转。", "热"),
    widgetjs_hotspot!(
        "hotspot-bilibili-rcmd",
        "B站公开视频推荐、封面与刷新。",
        "▶"
    ),
    widgetjs_hotspot!(
        "hotspot-cls-telegraph",
        "财联社公开电报，按源时间排序。",
        "讯"
    ),
    widgetjs_hotspot!("hotspot-douyin", "抖音热搜、热点标签与点击打开。", "热"),
    widgetjs_hotspot!("hotspot-tou-tiao", "头条热榜与新闻跳转。", "讯"),
    widgetjs_hotspot!("hotspot-weibo", "微博热搜、热度标签与搜索跳转。", "热"),
    widgetjs_hotspot!("hotspot-zhihu", "知乎热榜与问题跳转。", "知"),
    widgetjs_static!(
        "forex",
        "14种币种双向换算、汇率变动与每分钟更新，金额按精确十进制计算。",
        "汇",
        ["app.js", "currencies.js", "forex-model.js", "style.css"]
    ),
    widgetjs_static!(
        "metal",
        "BiQuote金/银/铜/铂金中间价、涨跌、来源时间与过期标记，保留原30秒刷新。",
        "金",
        ["app.js", "metal-model.js", "style.css"]
    ),
    Preset {
        manifest: include_str!("../../components/widgetjs/gold/manifest.json"),
        description: "固定Au99.99最新价、昨收/高低、分时画布和5秒刷新，精确涨跌与旧数据标记。",
        symbol: "Au",
        prebuilt: None,
        files: &[
            (
                "dist/index.html",
                include_bytes!("../../components/widgetjs/gold/index.html"),
            ),
            (
                "dist/SOURCE.txt",
                include_bytes!("../../components/widgetjs/gold/SOURCE.txt"),
            ),
            (
                "dist/app.js",
                include_bytes!("../../components/widgetjs/gold-bundle/app.js"),
            ),
            (
                "dist/style.css",
                include_bytes!("../../components/widgetjs/gold-bundle/style.css"),
            ),
        ],
    },
    Preset {
        manifest: include_str!("../../components/widgetjs/stock-medium/manifest.json"),
        description: "原上证/深证/创业板三行指数、60秒刷新与涨跌色，逐行保留读取错误。",
        symbol: "A",
        prebuilt: None,
        files: &[
            (
                "dist/index.html",
                include_bytes!("../../components/widgetjs/stock-medium/index.html"),
            ),
            (
                "dist/SOURCE.txt",
                include_bytes!("../../components/widgetjs/stock-medium/SOURCE.txt"),
            ),
            (
                "dist/app.js",
                include_bytes!("../../components/widgetjs/stock-medium-bundle/app.js"),
            ),
            (
                "dist/style.css",
                include_bytes!("../../components/widgetjs/stock-medium-bundle/style.css"),
            ),
        ],
    },
    Preset {
        manifest: include_str!("../../components/widgetjs/excalidraw/manifest.json"),
        description: "完整本地手绘画板、中文字体、图片与文件导入导出，按实例自动保存。",
        symbol: "✎",
        files: &[],
        prebuilt: Some(include_bytes!(
            "../../components/widgetjs/excalidraw-bundle/preset.zip"
        )),
    },
    Preset {
        manifest: include_str!("../../components/widgetjs/stock-small/manifest.json"),
        description:
            "默认港股01810，搜索选择证券、完整市场身份、分时末点/涨跌与画布，独立主题设置。",
        symbol: "↗",
        prebuilt: None,
        files: &[
            (
                "dist/index.html",
                include_bytes!("../../components/widgetjs/stock-small/index.html"),
            ),
            (
                "dist/SOURCE.txt",
                include_bytes!("../../components/widgetjs/stock-small/SOURCE.txt"),
            ),
            (
                "dist/app.js",
                include_bytes!("../../components/widgetjs/stock-small-bundle/app.js"),
            ),
            (
                "dist/style.css",
                include_bytes!("../../components/widgetjs/stock-small-bundle/style.css"),
            ),
        ],
    },
    Preset {
        manifest: include_str!("../../components/widgetjs/coin-small/manifest.json"),
        description: "原14币种的OKX公开指数价、实际采样曲线、精确涨跌、主题与本实例代理。",
        symbol: "₿",
        prebuilt: None,
        files: &[
            (
                "dist/index.html",
                include_bytes!("../../components/widgetjs/coin-small/index.html"),
            ),
            (
                "dist/SOURCE.txt",
                include_bytes!("../../components/widgetjs/coin-small/SOURCE.txt"),
            ),
            (
                "dist/app.js",
                include_bytes!("../../components/widgetjs/coin-small-bundle/app.js"),
            ),
            (
                "dist/style.css",
                include_bytes!("../../components/widgetjs/coin-small-bundle/style.css"),
            ),
            (
                "dist/bitcoin.png",
                include_bytes!("../../components/widgetjs/coin-small-bundle/bitcoin.png"),
            ),
        ],
    },
    widgetjs_photo!(
        "photo",
        "用户选择文件夹，随机/顺序轮播、前后切换与鼠标移动暂停。",
        "▧"
    ),
    widgetjs_photo!(
        "sticker",
        "原折角大头贴，本地图片副本或HTTPS图片地址，按实例保存。",
        "▣"
    ),
    Preset {
        manifest: include_str!("../../components/widgetjs/convert/manifest.json"),
        description: "HEIC本地解码，JPG/PNG/WEBP/BMP/ICO转换与缩放，批量ZIP和用户选择保存位置。",
        symbol: "▧",
        prebuilt: None,
        files: &[
            (
                "dist/index.html",
                include_bytes!("../../components/widgetjs/convert/index.html"),
            ),
            (
                "dist/SOURCE.txt",
                include_bytes!("../../components/widgetjs/convert/SOURCE.txt"),
            ),
            (
                "dist/HEIC-LICENSE.txt",
                include_bytes!("../../components/widgetjs/convert/HEIC-LICENSE.txt"),
            ),
            (
                "dist/ZIP-LICENSE.txt",
                include_bytes!("../../components/widgetjs/convert/ZIP-LICENSE.txt"),
            ),
            (
                "dist/app.js",
                include_bytes!("../../components/widgetjs/convert-bundle/app.js"),
            ),
            (
                "dist/style.css",
                include_bytes!("../../components/widgetjs/convert-bundle/style.css"),
            ),
        ],
    },
];

// 仅供显式历史验收及单元测试使用，不进入用户组件库或当前预置。
const VERIFICATION_FIXTURES: &[Preset] = &[
    Preset {
        manifest: include_str!("../../tests/fixtures/surface/manifest.json"),
        description: "验证组件包、实例设置及窗口生命周期。",
        symbol: "◷",
        prebuilt: None,
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
        description: "验证普通文档设置、消息桥及实例隔离。",
        symbol: "▤",
        prebuilt: None,
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
        if let Some(bytes) = self.prebuilt {
            return Ok(bytes.to_vec());
        }
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
    #[cfg(test)]
    if let Some(fixture) = verification_fixture(id)? {
        return Ok(Some(fixture));
    }
    Ok(None)
}

pub(crate) fn verification_fixture(id: &str) -> AppResult<Option<&'static Preset>> {
    for fixture in VERIFICATION_FIXTURES {
        if fixture.metadata()?["id"] == id {
            return Ok(Some(fixture));
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
        entry["availableVersion"] = manifest["version"].clone();
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
    use sha2::{Digest, Sha256};
    use std::io::Read;

    #[test]
    fn fresh_library_lists_all_presets_without_installing_or_creating_instances() {
        let store =
            Store::open(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string())).unwrap();
        let before = store.snapshot().unwrap();
        let catalog = catalog(before["packages"].as_array().unwrap()).unwrap();
        assert_eq!(catalog.len(), 58);
        assert!(catalog
            .iter()
            .all(|p| p["preset"] == true && p["installed"] == false));
        assert_eq!(store.snapshot().unwrap(), before);
    }

    #[test]
    fn all_presets_load_through_package_pipeline_and_can_be_added_again_after_unregistering() {
        let store =
            Store::open(std::env::temp_dir().join(uuid::Uuid::new_v4().to_string())).unwrap();
        // 独立包构建记录与Rust预置资源必须一致，避免两个入口各自正确却交付不同内容。
        let state: Value =
            serde_json::from_str(include_str!("../../components/widgetjs/build-state.json"))
                .unwrap();
        let records = state["packages"].as_array().unwrap();
        let mut matched = std::collections::HashSet::new();
        for preset in PRESETS {
            let bytes = preset.archive().unwrap();
            assert_eq!(bytes, preset.archive().unwrap());
            let mut archive = zip::ZipArchive::new(Cursor::new(bytes.clone())).unwrap();
            let staged = widgets::stage_bytes(&store, bytes).unwrap();
            // 这是核心包管线检查；真实 WebView 消息桥试加载另行验收。
            let manifest = widgets::activate(&store, &staged).unwrap();
            let mut entries = Vec::new();
            for index in 0..archive.len() {
                let mut file = archive.by_index(index).unwrap();
                let mut contents = Vec::new();
                file.read_to_end(&mut contents).unwrap();
                entries.push(json!([
                    file.name(),
                    contents.len(),
                    format!("{:x}", Sha256::digest(&contents))
                ]));
                if file.name() == "manifest.json" {
                    let original: Value = serde_json::from_slice(&contents).unwrap();
                    assert_eq!(original, preset.metadata().unwrap());
                } else if !file.is_dir() {
                    assert!(
                        widgets::resource(&store, &format!("{}/{}", manifest.id, file.name()))
                            .is_ok()
                    );
                    if file.name() == "dist/host.js" {
                        assert_eq!(contents, include_bytes!("../../sdk/host.js"));
                    }
                }
            }
            if manifest.id.starts_with("local.souldesk.widgetjs.") {
                let record = records.iter().find(|r| r["id"] == manifest.id).unwrap();
                entries.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));
                assert_eq!(record["version"], manifest.version);
                assert_eq!(record["files"], entries.len());
                assert_eq!(
                    record["digest"],
                    format!(
                        "{:x}",
                        Sha256::digest(serde_json::to_vec(&entries).unwrap())
                    ),
                    "{}的独立包与预置资源不一致",
                    manifest.id
                );
                assert!(matched.insert(manifest.id.clone()));
            }
            let id = widgets::create_instance(&store, &manifest.id).unwrap();
            assert!(widgets::stage_bytes(&store, preset.archive().unwrap())
                .unwrap()
                .unchanged());
            widgets::remove_instance(&store, &widgets::Sessions::default(), &id).unwrap();
            widgets::unregister_package(&store, &manifest.id).unwrap();
            let snapshot = store.snapshot().unwrap();
            let catalog = catalog(snapshot["packages"].as_array().unwrap()).unwrap();
            assert_eq!(catalog.len(), PRESETS.len());
            assert!(catalog.iter().all(|p| p["installed"] == false));
            let staged = widgets::stage_bytes(&store, preset.archive().unwrap()).unwrap();
            widgets::activate(&store, &staged).unwrap();
            widgets::unregister_package(&store, &manifest.id).unwrap();
        }
        assert_eq!(matched.len(), records.len());
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
        assert_eq!(catalog.len(), PRESETS.len() + 1);
        assert_eq!(catalog[0]["name"], upgraded["name"]);
        assert_eq!(catalog[0]["version"], "9.0.0");
        assert_eq!(catalog[0]["installed"], true);
        assert_eq!(catalog.last().unwrap()["preset"], false);
        assert_eq!(catalog.last().unwrap()["installed"], true);
        assert!(find("local.custom.tool").unwrap().is_none());
    }

    #[test]
    fn corrected_widgetjs_versions_upgrade_legacy_instances_without_losing_saved_state() {
        for (name, previous, next) in [
            ("clock", "0.1.1", "0.1.3"),
            ("flip-clock", "0.1.1", "0.1.3"),
            ("glitch-clock", "0.1.1", "0.1.3"),
            ("mickey-clock", "0.1.1", "0.1.3"),
            ("text-clock", "0.1.1", "0.1.3"),
            ("hotspot-bangumi", "0.1.2", "0.1.4"),
            ("hotspot-bilibili", "0.1.2", "0.1.4"),
            ("hotspot-bilibili-rcmd", "0.1.2", "0.1.4"),
            ("hotspot-cls-telegraph", "0.1.2", "0.1.4"),
            ("hotspot-douyin", "0.1.2", "0.1.4"),
            ("hotspot-tou-tiao", "0.1.2", "0.1.4"),
            ("hotspot-weibo", "0.1.2", "0.1.4"),
            ("hotspot-zhihu", "0.1.2", "0.1.4"),
            ("ai-assistant", "0.1.0", "0.1.2"),
            ("ai-mindmap", "0.1.2", "0.1.4"),
            ("ai-mermaid", "0.1.2", "0.1.4"),
            ("ai-translate", "0.1.2", "0.1.3"),
            ("clipboard", "0.1.0", "0.1.2"),
            ("clipboard-search", "0.1.0", "0.1.2"),
            ("birthday-list", "0.2.0", "0.2.1"),
            ("countdown", "0.2.0", "0.2.1"),
            ("countdown-list", "0.2.0", "0.2.1"),
            ("todo", "0.2.0", "0.2.1"),
            ("calendar-large", "0.2.0", "0.2.3"),
            ("calendar-large", "0.2.1", "0.2.3"),
            ("calendar-large", "0.2.2", "0.2.3"),
            ("calendar", "0.1.1", "0.1.2"),
            ("hotspot-bangumi", "0.1.1", "0.1.4"),
            ("hotspot-bilibili", "0.1.1", "0.1.4"),
            ("hotspot-bilibili-rcmd", "0.1.1", "0.1.4"),
            ("hotspot-cls-telegraph", "0.1.1", "0.1.4"),
            ("hotspot-douyin", "0.1.1", "0.1.4"),
            ("hotspot-tou-tiao", "0.1.1", "0.1.4"),
            ("hotspot-weibo", "0.1.1", "0.1.4"),
            ("hotspot-zhihu", "0.1.1", "0.1.4"),
            ("sticker", "0.1.0", "0.1.2"),
            ("excalidraw", "0.1.0", "0.1.2"),
            ("convert", "0.1.0", "0.1.2"),
            ("mindmap", "0.2.1", "0.2.3"),
            ("forex", "0.1.0", "0.1.2"),
            ("sticker", "0.1.1", "0.1.2"),
            ("metal", "0.1.0", "0.1.1"),
            ("excalidraw", "0.1.1", "0.1.2"),
            ("convert", "0.1.1", "0.1.2"),
            ("gold", "0.1.0", "0.1.1"),
            ("stock-medium", "0.1.0", "0.1.1"),
            ("stock-small", "0.1.0", "0.1.1"),
            ("coin-small", "0.1.0", "0.1.1"),
            ("coin-medium", "0.1.0", "0.1.1"),
            ("mindmap", "0.2.2", "0.2.3"),
            ("server", "0.1.1", "0.1.2"),
            ("stock", "0.1.0", "0.1.1"),
            ("ai-mindmap", "0.1.3", "0.1.4"),
            ("ai-mermaid", "0.1.3", "0.1.4"),
            ("clipboard", "0.1.1", "0.1.2"),
            ("clipboard-search", "0.1.1", "0.1.2"),
            ("labor", "0.1.0", "0.1.1"),
            ("server", "0.1.0", "0.1.2"),
        ] {
            let package = format!("local.souldesk.widgetjs.{name}");
            let preset = find(&package).unwrap().unwrap();
            let current = preset.archive().unwrap();
            let mut input = zip::ZipArchive::new(Cursor::new(current.clone())).unwrap();
            let mut old = zip::ZipWriter::new(Cursor::new(Vec::new()));
            // 只构造旧版本且内容不同的标准包夹具，不冒充历史程序的实机升级。
            for index in 0..input.len() {
                let mut file = input.by_index(index).unwrap();
                let path = file.name().to_owned();
                let mut body = Vec::new();
                file.read_to_end(&mut body).unwrap();
                if path == "manifest.json" {
                    let mut manifest: Value = serde_json::from_slice(&body).unwrap();
                    manifest["version"] = json!(previous);
                    let capabilities = manifest["capabilities"].as_array_mut().unwrap();
                    let count = capabilities.len();
                    capabilities.retain(|value| {
                        !crate::component_registry::is_scoped_content_capability(
                            value.as_str().unwrap(),
                        )
                    });
                    if capabilities.len() != count || name == "coin-medium" {
                        capabilities.push(json!(crate::component_registry::LEGACY_CONTENT_READ));
                    }
                    for (prefix, legacy) in [
                        ("documents.", "documents.user"),
                        ("clipboard.", "clipboard.manage"),
                    ] {
                        let count = capabilities.len();
                        capabilities.retain(|value| {
                            let value = value.as_str().unwrap();
                            !value.starts_with(prefix)
                                || matches!(value, "clipboard.read" | "clipboard.write")
                        });
                        if capabilities.len() != count {
                            capabilities.push(json!(legacy));
                        }
                    }
                    if manifest["capabilities"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v == crate::asset_permissions::SCOPED)
                    {
                        let mut capabilities = manifest["capabilities"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter(|v| {
                                ![
                                    crate::asset_permissions::SCOPED,
                                    crate::asset_permissions::STYLES,
                                    crate::asset_permissions::IMAGES,
                                    crate::asset_permissions::FETCH,
                                    crate::asset_permissions::WASM,
                                    crate::asset_permissions::EVAL,
                                    crate::asset_permissions::WORKER,
                                    crate::asset_permissions::BLOB_WORKER,
                                ]
                                .iter()
                                .any(|name| *v == *name)
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        capabilities.push(json!("assets.local"));
                        manifest["capabilities"] = json!(capabilities);
                    }
                    body = serde_json::to_vec(&manifest).unwrap();
                } else if path == preset.metadata().unwrap()["entry"].as_str().unwrap() {
                    body.extend_from_slice(b"\n<!-- legacy-upgrade-fixture -->\n");
                }
                old.start_file(path, zip::write::SimpleFileOptions::default())
                    .unwrap();
                old.write_all(&body).unwrap();
            }
            let root = std::env::temp_dir().join(format!(
                "souldesk-widgetjs-version-upgrade-{}",
                uuid::Uuid::new_v4()
            ));
            let store = Store::open(root.clone()).unwrap();
            let baseline =
                widgets::stage_bytes(&store, old.finish().unwrap().into_inner()).unwrap();
            widgets::activate(&store, &baseline).unwrap();
            let first = widgets::create_instance(&store, &package).unwrap();
            let second = widgets::create_instance(&store, &package).unwrap();
            // 标准包升级必须原样保留宿主视为不透明的非空实例存储。
            for (id, label) in [(&first, "私有甲"), (&second, "私有乙")] {
                store
                    .db
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE widget_instances SET storage=?1 WHERE id=?2",
                        rusqlite::params![
                            json!({"version":1,"label":label,"items":[{"id":"keep","value":42}]})
                                .to_string(),
                            id
                        ],
                    )
                    .unwrap();
            }
            let sessions = widgets::Sessions::default();
            let label = format!("float-{first}");
            let old_session =
                widgets::open_display_session(&store, &sessions, &label, &first, None).unwrap();
            let before = store.snapshot().unwrap()["instances"].clone();
            let staged = widgets::stage_bytes(&store, current)
                .unwrap_or_else(|error| panic!("{package}: {error}"));
            assert_ne!(baseline.digest, staged.digest);
            assert_eq!(
                widgets::activate_sessions(&store, &sessions, &staged)
                    .unwrap()
                    .version,
                next
            );
            assert_eq!(store.snapshot().unwrap()["instances"], before);
            assert!(sessions.0.lock().unwrap().is_empty());
            assert!(widgets::gateway(
                &store,
                &sessions,
                &label,
                old_session["token"].as_str().unwrap(),
                "storage.get",
                Value::Null,
            )
            .unwrap_err()
            .starts_with("SESSION_EXPIRED:"));
            let snapshots: u32 = store
                .db
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM widget_upgrade_snapshots WHERE package_id=?1",
                    [&package],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(snapshots, 2);
            drop(store);
            let reopened = Store::open(root).unwrap();
            assert_eq!(reopened.snapshot().unwrap()["instances"], before);
        }
    }

    #[test]
    fn photo_presets_resolve_static_module_dependencies() {
        use std::io::{Cursor, Read};
        for id in [
            "local.souldesk.widgetjs.photo",
            "local.souldesk.widgetjs.sticker",
        ] {
            let mut archive =
                zip::ZipArchive::new(Cursor::new(find(id).unwrap().unwrap().archive().unwrap()))
                    .unwrap();
            let names: std::collections::HashSet<_> =
                archive.file_names().map(str::to_owned).collect();
            for name in names.iter().filter(|name| name.ends_with(".js")) {
                let mut source = String::new();
                archive
                    .by_name(name)
                    .unwrap()
                    .read_to_string(&mut source)
                    .unwrap();
                for prefix in ["from './", "from'./", "from \"./", "from\"./"] {
                    for tail in source.split(prefix).skip(1) {
                        let quote = if prefix.contains('"') { '"' } else { '\'' };
                        let relative = tail.split(quote).next().unwrap();
                        let path = format!("{}/{}", name.rsplit_once('/').unwrap().0, relative);
                        assert!(names.contains(&path), "{id}: {name} 引用了缺失资源 {path}");
                    }
                }
            }
        }
    }
    #[test]
    fn mindmap_local_document_uses_formal_storage_and_survives_reopening_without_crossing_instances(
    ) {
        let store = Store::open(
            std::env::temp_dir().join(format!("souldesk-mindmap-{}", uuid::Uuid::new_v4())),
        )
        .unwrap();
        let staged = widgets::stage_bytes(
            &store,
            find("local.souldesk.widgetjs.mindmap")
                .unwrap()
                .unwrap()
                .archive()
                .unwrap(),
        )
        .unwrap();
        assert!(staged
            .manifest
            .capabilities
            .iter()
            .any(|cap| cap == "storage.document"));
        widgets::activate(&store, &staged).unwrap();
        let first = widgets::create_instance(&store, "local.souldesk.widgetjs.mindmap").unwrap();
        let second = widgets::create_instance(&store, "local.souldesk.widgetjs.mindmap").unwrap();
        let sessions = widgets::Sessions::default();
        let label = format!("float-{first}");
        let session =
            widgets::open_display_session(&store, &sessions, &label, &first, None).unwrap();
        let token = session["token"].as_str().unwrap();
        let document = json!({"version":1,"documents":[{"id":"work","topic":"工作计划","group":"项目","emoji":"📚","createdAt":1,"updatedAt":2,"mindmap":{"id":"root","name":"工作计划","children":[{"id":"branch","name":"待办分支","url":"https://example.com","checked":true,"collapsed":true,"children":[]}]}}],"preferences":{"theme":"dark","showGrid":false,"showCheckboxes":true,"sidebarOpen":false}});
        widgets::gateway(
            &store,
            &sessions,
            &label,
            token,
            "storage.set",
            document.clone(),
        )
        .unwrap();
        assert!(widgets::gateway(
            &store,
            &sessions,
            "other-window",
            token,
            "storage.get",
            Value::Null
        )
        .is_err());
        let root = store.root.clone();
        drop(store);
        let reopened = Store::open(root).unwrap();
        let sessions = widgets::Sessions::default();
        for (id, expected) in [(&first, document), (&second, json!({}))] {
            let label = format!("float-{id}");
            let session =
                widgets::open_display_session(&reopened, &sessions, &label, id, None).unwrap();
            assert_eq!(
                widgets::gateway(
                    &reopened,
                    &sessions,
                    &label,
                    session["token"].as_str().unwrap(),
                    "storage.get",
                    Value::Null
                )
                .unwrap(),
                expected
            );
        }
    }
}
