# WidgetJS网页AI助手实施计划

目标仍为61项全部移植。本批处理固定rtugeek/ai提交0160f55feabd19421953c687eba033a707a8c3c2的网页助手；当前52项实现待验/9项批准待实施，59包/62预置。剪切板22931df0固定程序及手动库保留，不操作或启动它。

## 已核实的功能与实现约束

- 原项目是BackgroundWidget，八个网页站点、1/2/4分屏、每格选站、持久浏览资料、左右侧、20/30/40/50%宽度、置顶、Win+Alt+C及托盘切换、500ms滑动与初次说明。默认各格按原AiUtils顺序为ChatGPT/Grok/Gemini/DeepSeek；不是原API生成器。
- 原网页助手不含有效的跨站统一提问。DuelWebview在主模板中注释，不开放任意网页脚本执行桥。登录由用户手动完成，不读取或导出密码/Cookie，不自动代付。
- 已批准后台/置顶显式启用、默认关闭。普通桌面卡片提供状态、设置、打开；专用侧边窗承载原网页助手。背景入口关闭时隐藏会释放本机网页；显式开启后保留隐藏网页及快捷入口。置顶独立开关，默认关闭。关闭主显示不撤销已批准的后台入口。
- 原代理有HTTP/HTTPS/SOCKS4/SOCKS5。固定Tauri proxy_url只支持HTTP/SOCKS5，Windows多协议使用经过严格校验的固定浏览器参数，不接受凭据或额外参数；不降低TLS校验。代理变更关闭全部本实例网页/登录弹窗后重建，保留同一浏览资料路径，不清登录数据；真实代理与登录另待手动。
- 已评估现有Skill：当前先做Rust状态与窗口接口证据，不涉及新视觉设计或GUI自动化；按本仓库检查/最小实现流程推进。后续Vue沿用固定原模板/SCSS，不新增Pinia/VueRouter或全局依赖。

## SDK证据与窗口方案

已查当前官方Tauri文档及本机锁定2.12.5源码。`Window::add_child`需要unstable特性。多视图窗口不再满足`is_webview_window`，`get_webview_window`和CommandArg<WebviewWindow>会拒绝它，不能直接往现有桌面/面板/管理窗口增加子视图。

使用独立原生Window及本机壳视图，另加有限的助手会话/请求/失败命令（参数为Webview），不改现有窗口为多视图。网页只作为该窗子视图，使用独立固定标签、按实例持久目录，无宿主端口/权限、无初始化脚本。壳负责原控件与状态；页面选择器/设置/提示展开时暂时隐藏远程视图，防止原生子视图盖住控件。1/2/4布局只接受本窗口内有限、不重叠的矩形，不允许任意页面或其他窗口。

当前官方接口参考：
- https://docs.rs/tauri/2.12.5/tauri/window/struct.Window.html#method.add_child
- https://docs.rs/tauri/2.12.5/tauri/webview/struct.WebviewBuilder.html
- https://github.com/tauri-apps/tauri/blob/dev/crates/tauri/src/webview/mod.rs

本机实际证据为Cargo缓存tauri-2.12.5的window/mod.rs（add_child/is_webview_window）、lib.rs（get_webview_window）及webview/webview_window.rs（CommandArg）。Windows最后一个网页释放时，固定tauri-runtime-wry-2.12.3会移除对应WebContext；重建必须串行关闭后再创建，不能忽略代理变化。

2026-11-17只读网页核对：Kimi原网址重定向www.kimi.com，元宝/chat重定向根路径；千问、豆包、ChatGPT、Gemini可由检索工具读取，DeepSeek/Grok读取403。保留原固定入口、允许正常HTTPS重定向；检索结果不证明WebView加载或登录可用，不更换工具绕过403。

## 实施顺序

1. [x] 固定来源复核；配置/版本/UUID回执、代理校验及资料目录的隔离库反例，再实现assistant_state。默认关闭，失败不覆盖，旧请求不能覆盖新配置。
2. [x] 有限网页计划/驱动及Fake反例：1/2/4布局、页面切换、先撤销再重建、旧配置/关闭/删除/退出、单实例释放与原生失败。启用锁定Tauri的unstable特性，不升级版本；实际驱动只编译，不创建网页。
3. [x] 固定包/助手角色/专用Window、本机壳IPC、快捷键/共享托盘入口与生命周期。远程页面无IPC能力；失败状态保留且不会冒称已加载。严格代次/配置/会话校验，不持SQLite锁建窗。
4. [x] 派生原Vue/SCSS、八图标、1/2/4、每格选站、代理/位置/宽度/置顶、说明与滑动；客户端保存失败保留草稿。离线SSR/DOM/模型验证，后台初始开关为false。
5. [x] 功能接通后才加标准包/预置并更新目录数。按最终资源→ZIP字节审计→核心预置专项→Release构建交接新隔离入口。真实窗口、登录/代理/弹窗及多屏仍待用户手动，不启动GUI。

## 下一组件

AI直聘固定公开部署58806a7f...：当前静态源码确认BOSS手动登录、关键字/翻页采集和AI分析，不把通用WebviewUtils内未调用的方法当原功能。后续只开放固定采集脚本/解析和明确AI操作，不开放任意JavaScript或自动账号登录；来源见artifacts/widgetjs-ai-job-source-audit.json及widgetjs-ai-job-fixed-script-audit.json。


## 2026-11-17基础阶段结果

Task 1完成。assistant_state配置/版本/UUID回执/事务14项通过；assistant_webviews有限协调及原生驱动已实现、12项Fake/几何/导航检查通过，Task 2还需与专用Window及退出协调接线，不把编译当真实运行。Tauri只给当前锁定2.12.5启用unstable，未升级版本；首次编译成功。

额外反例先复现两处问题：Serde带标签的unit分支忽略额外字段，现严格按mode字段集合反序列化；HTTPS的本机*.localhost最初可导航，现与其他宿主/危险协议一起拒绝。零版本非默认配置和非规范实例/回执标识拒绝读取。代理切换弹窗释放失败原会遗失其阻塞状态，现持有pending_profiles，确认关闭后才可重建同一资料目录，不静默走旧代理。

26项专项、完整核心444通过/2忽略、格式通过。固定89文件1746540字节散列复核一致，上游HEAD匹配、工作树干净。没有新标准包、前端资源或Release，也没有GUI/登录/代理实际测试；原剪切板22931df0程序散列保持，仍是可手动程序，不包含助手。证据artifacts/widgetjs-assistant-foundation-checks-20261117.json。

下一步直接做Task 2/3接线：Native只添加到专用多视图Window，IPC参数用Webview而非WebviewWindow，本机壳4命令要有限/带宿主令牌。原生页面/登录弹窗回调只更新展示账本并异步排队清理，不能在主线程回调中等待子视图协调锁；退出/代理变更/新代次均先撤销旧授权再执行清理。首次加载完成只表示载入结束，不冒称站点/登录成功。

AI直聘还需完整核对JobDetail/JobAnalysis/Knowledge/Landing五个模块的原功能；当前仅静态确认JobView采集流程，未把它视为全部范围。

## 2026-11-17 原生宿主阶段结果

Task 2/3代码已接通并编译：assistant_host绑定固定包、清单能力、实例、会话、展示代次及配置版本；独立Window加本机壳，只有assistant-shell-*有四个专用命令，远程页/登录窗没有宿主令牌或IPC权限。原生导航开始/完成使用WebView2结果和编号，不以框架的Finished回调冒称成功；新视图另有内存UUID，旧回调不能覆盖新页。登录弹窗继承发起页环境，包含嵌套窗在内每实例最多4个；当前聚焦来源才可发起，隐藏、改站/代理、重载、删除/升级和退出均撤销。

后台开关默认关闭；仅显式启用才登记快捷键/提供后台托盘入口。共享底层键登记器，不调用剪切板业务；冲突可见、每配置版本失败只试一次、旧事件重验。主线程回调不等登记锁，顺带修正现有剪切板分流的同类等待。窗口动作在工作线程串行，回调只触及内存；ready不等待建窗协调锁。隐藏默认释放，启用后台才隐藏保留；同格换站/代理按既定规则处理。布局竞争先用反例复现已载入页被销毁，现暂时隐藏并复用已确认页面，新创建未确认页仍关闭。原生释放不确定会保留nativeCleanupPending/nativeError，不宣称已成功清理。

50项助手专项（状态14/网页17/宿主19）、完整核心469通过/2忽略、前端315、类型/格式/diff检查通过。Cargo的568个锁定包版本/来源/校验和均不变，仅将既有webview2-com0.39.1列为直接依赖；89份来源散列和旧手动程序散列未变。所有新原生GUI/登录/代理/快捷键仍未调用，不是实机验收。记录artifacts/widgetjs-assistant-host-checks-20261117.json。

下一步Task 4：派生原AiWidgetView/BaseWebview/AiSelect/SettingHeader/Tip/AiConfigView，不新增Pinia/VueRouter或全局依赖；主卡与专用窗由page=assistant-panel区分。配置用assistant.status/configure，UUID和expectedRevision；保存失败/超时保留草稿，未知结果只重试原UUID。设置/选择器覆盖层先调用assistant.overlay并等surface.nativeSettled再展开，防原生网页盖住菜单。引导进度0/1/2期间默认guide覆盖，完成3才移除。宿主只返回导航phase，不能显示“已登录”。先做客户端与原模板离线检查，再加标准包/预置和构建；当前数量及手动程序不变。

## 2026-11-17 本批交付

Task 4/5代码、标准包和构建已完成，真实网页/桌面验收保持待手动，不代表所有61项完成。51项助手状态/假驱动、完整核心470/2忽略、全前端333、最终资源63预置5专项通过；60包803文件一致。本机壳和远程页均显式使用当前数据根目录，路径补丁后51专项与Release再通过。入口artifacts/verify-widgetjs-assistant.cmd，程序d16a0822，新库只有标识；报告artifacts/widgetjs-assistant-checks-20261117.json。用户暂停引发的首次组件构建中断后已恢复完成，未运行GUI。

下一步为AI直聘固定来源58806a7f...的完整模块审计，含JobView/JobDetail/JobAnalysis/Knowledge/Landing；仍按用户批准的自填模型、手动登录与显式后台政策实施，不开放任意脚本或自动登录。原7项本地版的云/专用能力继续逐项对照，不因共享库已有实现而把原云同步标完成。
