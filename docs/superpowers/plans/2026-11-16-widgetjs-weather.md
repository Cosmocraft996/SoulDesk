# WidgetJS三款和风天气实施计划

> 执行方式：当前会话按 `superpowers:executing-plans` 分任务实施，完成后独立源码审查；用户已批准“四类建议，分批继续”，不重复询问执行方式。不提交、推送、安装或自动运行界面。

**目标：** 将固定提交的三款天气作为SoulDesk标准预置包，用户配置自己的和风天气服务后可查询城市、显示实时天气及大号天气的三日预报/生活指数。

**架构：** 保留原视图、原图标及布局，替换WidgetJS桥和前端HTTP；Rust只访问用户专属和风域名和固定路径。每实例凭据使用Windows DPAPI加密，页面仅获得配置状态，不通过settings/storage回读密钥。默认关闭网络，启用须用户保存配置。

**技术栈：** 已有Rust/Tauri/Vue/Vite；Windows系统凭据保护；Gzip复用锁文件已有flate2版本。无全局安装。

**依据：** docs/13-WidgetJS组件移植.md；artifacts/widgetjs-decision-scope-audit-20261116.json；上游rtugeek/weather固定提交1d5b717b085c82a5b81bfc2f0969bb8518123570。

## 约束和边界

- 用户2026-11-16明确批准9项账号/凭据、7项可选本地共享、5项显式后台能力及1项手工持仓，原云同步仍单列未完成。
- 本计划只做天气3项，不自动启用其他后台能力，不申请账户、不代付、不读取真实密钥或正常用户库。
- v7城市实时与三日预报目前仍有官方文档，但已标记即将弃用；保留来源口径并记录后续v1适配，不以其他免费源替代。
- 密钥放X-QW-Api-Key请求头，专属域名仅允许*.qweatherapi.com的HTTPS默认端口；不跟随重定向，不接受组件传入任意路径或请求头。
- 缺失温度、失败、旧数据及局部预报/指数失败分别显示，不以0或当前读取时间冒充来源观测值。
- 组件包普通配置不包含密钥，密文不返回页面；隐藏试加载不得配置或访问凭据、不得联网。
- 不自动运行界面，最终实机及用户自有服务连通性留给手动验收。

## 任务一：凭据和固定天气协议

文件：src-tauri/src/weather.rs；修改Cargo.toml/Cargo.lock、lib.rs、widgets.rs、windows.rs、sdk/README.md。

接口：新增weather.read能力；weather.status返回{configured,enabled,apiHost}；weather.configure接收{apiHost,apiKey?,enabled}，空密钥保留已有值，首次启用必须提供有效密钥；weather.get接收{kind:now|daily|indices|lookup,location?|query?}；weather.clear清除本实例凭据。

- [x] 先写域名/URL固定路径、禁用/缺凭据、数据解析/来源时间、Gzip限额、错误脱敏和DPAPI往返检查，确认失败。
- [x] 实现每实例DPAPI密文保存、固定HTTP及正式会话校验；清除/删除实例时移除对应密文。
- [x] 运行weather专项和权限回归；回环夹具只用占位密钥，不能据此宣称用户服务连通。

## 任务二：三款原视图与设置

文件：components/widgetjs/weather-source及weather-small/weather-medium/weather-large三个清单；scripts/port-widgetjs-weather.mjs；修改scripts/build-widgetjs.mjs、src-tauri/src/presets.rs、package.json。

接口：页面仅调用任务一接口，普通settings保存城市及非敏感主题/单位；配置页用密码框单次提交密钥，成功后清空输入。原图标本地打包，城市搜索保留id/经纬度/时区身份。

- [x] 固定源码/原资源散列，按现有移植方式替换WidgetJS、图标及Element包装，不引入整套旧宿主依赖。
- [x] 先写缺值/摄氏华氏、失效响应竞争、预览无请求和保存失败保留检查，再实现数据桥。
- [x] 复用现有Vue渲染检查验证三个分支、设置及局部失败；生成三个标准ZIP并登记预置。

## 任务三：集成、审查和交接

- [x] 检查天气能力的预览拒绝、实例隔离及原包升级行为；执行相关前端/核心、类型/格式和构建。
- [x] 核对独立ZIP与预置输入；范围从46份移植包/49预置按实际产物更新，不提前增加数量。
- [x] 独立审查凭据泄露、跳转、压缩限额、错误/旧值与实例生命周期。
- [x] 更新README、13及目录，提供独立手动入口。没有自有凭据时仅标明离线/回环证据，真实和风服务与桌面待验。

## 审查重点

1. 通用snapshot/导出/错误或请求URL不得包含原始密钥。
2. 错误域名、重定向和压缩炸弹不得绕过固定请求与限额。
3. 切城市/配置后的晚到响应不得覆盖新城市；保存失败保留输入和原配置。
4. 未配置/关闭/预览不得发请求，也不得以缺失温度显示0。
5. 上游未实际使用的空气接口不能误称已经提供空气质量功能；v7弃用风险需保留来源说明。

## 执行记录

- 2026-11-16：已添加weather.read网关和固定和风域名/路径，按实例DPAPI保护凭据。先行合同检查因未实现失败，随后6项专项通过；包含Gzip压缩与解压双限额、来源时间/缺值、城市请求身份、密文不回传、配置失败保留和清除回滚。日志artifacts/widgetjs-weather-core-checks.log。未配置真实凭据、未联网。
- 裁定：复用已锁定flate2 1.1.11作为直接依赖解码和风默认Gzip；仅启用既有windows-sys 0.61.2的Cryptography特性，不新增运行时或全局工具。
- 任务一尚待预览/正式能力回归和回环请求证据；三份组件包尚未生成，数量仍为46/49，不能提前计入完成。

- 2026-11-16最终：天气前端10项、核心8项、全前端220项、全核心223项（另2项跳过）、类型/格式和三款资源/主程序构建通过；49个ZIP共638文件逐字节匹配输入，52项预置通过核心包管线。独立审查两项P2（海外ID、午夜日期）均修复，空指数项安全降级。正式包校验曾拒绝不支持的pattern，已删该额外清单约束并保留业务校验；历史失败日志保留。
- 固定程序e2a26e895576c7e326a16ddb2e1f45b40715a97e372da3e6d07f025eb61e3f5a，64527360字节；artifacts/verify-widgetjs-weather.cmd及widgetjs-weather-manual-handoff.md只供用户手动启动，代理未运行。真实自有和风服务/桌面/正常安装仍未验，不算61项全目标完成。
