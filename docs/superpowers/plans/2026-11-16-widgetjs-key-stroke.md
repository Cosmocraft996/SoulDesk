# WidgetJS键盘演示移植实施计划

> 按用户已批准的5项专用后台/交互范围继续；默认关闭，用户在组件内显式开启。当前会话按计划和反例实施，不提交、推送、安装或自动运行GUI/全局键盘采集。用户已确认待办共享后台只调度当前源、私有暂停、番茄钟恢复私有保持暂停；原生共享后续独立实施。

## 目标与来源

完整移植widgetjs.cn.utilities.key_stroke为标准预置组件，固定utilities提交3d733215bac3e9c1e58125518f100c1601650828，保留原Vue模板、键帽/动画/渐变、修饰键组合及CapsLock、450×240底部居中无焦点置顶和释放后约4秒隐藏。源实际仅显示修饰键组合及CapsLock，不以README笼统“每个键”扩大为普通文字记录。来源散列见artifacts/widgetjs-key-stroke-source-audit.json。

## 架构与边界

- 标准包key-stroke提供控制卡和原Overlay.vue两种视图。控制卡显式启用/停用；叠加层沿原SDK消息桥，只读keyboard.peek/status和runtime.ready。新keyboard.display固定能力，不接受实例、窗口地址、键盘注入或任意原生命令。
- 每实例enabled独立持久化，默认false；多个启用实例共用一个Windows监听器、一个叠加层，停用或删除其中一份不影响其他启用者。稳定保留当前有效owner；没有启用者、退出或“隐藏全部”时停止监听并清临时键状态，恢复后按已保存enabled重建。仅隐藏控制卡不取消显式启用的后台演示。
- Rust keyboard.rs保留纯状态模型、权限、启用与服务协调；key_capture.rs负责Windows WH_KEYBOARD_LL专用消息循环、Caps初态、键名和清理。回调总是CallNextHookEx，不阻断/修改输入；仅修饰键组合/Caps事件形成快照，普通字母不生成或保留名称。快照只在内存，不写日志、SQLite或网络，不保留输入历史。
- 工作线程使用消息队列和30ms定时消息清理4秒过期状态，PostThreadMessage WM_QUIT停止并Unhook；停止早于就绪也必须终止，不在回调中读GetAsyncKeyState判断当前事件、不执行数据库/窗口/日志。
- 新键盘窗口由宿主创建固定450×240透明/无装饰/穿透/置顶/无焦点WebView，只加载已登记本组件资源。src/main.tsx识别keyboardOverlay并仅挂WidgetFrame，不导入外部组件业务到管理UI。会话新增keyboard-overlay角色，与原primary/details并存；普通控制卡隐藏/移位不能释放叠加层会话。该角色不授予settings/storage或配置写入。
- 原BrowserWindowApi改为宿主固定叠加层协调；原Motion改CSS固定进入/退出，不注入WidgetJS/Electron运行时。不添加依赖。叠加层真实加载/焦点/鼠标/多显示器和Windows事件效果留手动，离线检查不冒充实机。

## 官方依据

https://learn.microsoft.com/en-us/windows/win32/winmsg/lowlevelkeyboardproc ：回调运行在安装线程，需消息循环；回调时异步键状态尚未更新，必须快速返回并传递其他钩子。
https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowshookexw ：WH_KEYBOARD_LL仅监听当前桌面的低级键盘事件。
https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-postthreadmessagew ：先用PeekMessage创建队列，再公布线程ID供停止消息使用。
本项目已装windows-sys 0.61.2对应特性足够，不安装全局工具或增加依赖。

## 步骤一：状态和启用合同

文件src-tauri/src/keyboard.rs及lib.rs模块声明。先写KeyboardState.new/input/expire/snapshot反例：普通输入不记录、修饰组合按下/释放/4秒、左右修饰键合并、Caps不被重复keydown翻转、旧普通键keyup不放开新键。再实现纯模型。持久化enabled仅当前实例，deny_unknown_fields，坏值明确报错；最后消费者消失清状态，重复启用不增加监听器。

命令node scripts/desktop.mjs test --locked --offline keyboard::tests --lib；先未实现失败，再通过。此步骤不得实际启动Win32钩子。

## 步骤二：Windows线程及宿主叠加层

文件key_capture.rs、keyboard.rs、windows.rs、widgets.rs、lib.rs、src/main.tsx、src/style.css、src-tauri/capabilities/keyboard-overlay.json。固定gateway keyboard.status/configure/peek，试加载拒绝；peek只给本组件专用叠加层。会话及窗口创建/显示在状态检查后进行，停用后的旧排队显示不得复活；初始化失败明确记录并等待用户重试，不无限建窗。退出先设置quitting，再停止/回收监听线程。

纯核心检查覆盖缺能力/预览/跨窗、只读角色、overlay与primary并存、普通视图释放不影响overlay、停用者/最后消费者、过期不重新显示。编译Win32 API，只用本地夹具，不自动开启真实采集或界面。

## 步骤三：原视图与标准包

文件components/widgetjs/key-stroke-source、key-stroke，scripts/port-widgetjs-key-stroke.mjs，build-widgetjs.mjs、presets.rs、package.json。固定原文件散列，派生原模板替换WidgetJS钩子/窗口API为只读快照与CSS动画。控制卡状态区分“已请求启用/实际监听/显示初始化失败”，保存失败回滚开关。预览用原😎示例，无原生请求；关闭页面清轮询。

先写客户端和Vue渲染反例再实现；构建标准ZIP、原生预置输入逐字节核对后才将目录项改实现待验。当前50包/53预置按实际交付增加为51/54，不提前计数。

## 步骤四：审查与交付

独立审查权限、普通输入不记录、线程消息/回收、显示代次/焦点/穿透、角色与其他组件兼容。执行相关→全前端/核心→类型格式→资源/主程序构建，提供新的隔离手动入口但不启动。真实键盘、焦点/穿透、4秒隐藏、停用/删除/退出及多显示器分别记录待验，61项全目标不因本项标完成。

## 状态

- [x] 纯键盘状态与启用合同反例/实现。
- [x] 原生线程、权限角色和叠加层。
- [x] 原Vue、标准包及预置。
- [x] 审查、构建和手动交接（真实桌面等待用户手动，未自动启动）。

2026-11-16：原纯状态反例先红；13项键盘专项已纳入全核心245通过/2跳过，249项前端与类型/格式通过。51 ZIP/650文件逐字节一致。独立审查修正旧窗口排队清理及非所有者重试两项P2；重试反例在旧条件下失败，修正后纳入全核心通过。主程序构建/手动交接进行中；未启动真实监听。

构建完成：b773f50a9dcc50b9b2204d93d8b40e760eb0ae03547251f1f6126ea09d0bb038，固定手动入口及新隔离目录已准备，未启动。详见artifacts/widgetjs-keyboard-checks-20261116.json。

补充：漏松键反例先因缺方法失败，再实现30ms回调外核对本轮held键。安全桌面/UIPI无可用状态时保守释放；不在钩子回调检查异步状态，不扫描普通键。最新全核心246通过/2跳过，键盘专项共14。官方依据：https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getasynckeystate 。

最终修正版：0c612ebb248fe1b6547672ce15ecef26f72512fccf3e83d0f9f16ce8f1319985，已完成构建及固定入口散列核对，包含全部51个已映射包ID；未启动。
