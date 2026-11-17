# 待办、日历与番茄钟原生共享库实施计划

> 使用 writing-plans / executing-plans / test-driven-development 流程。用户2026-11-16已确认切换政策，继续在当前工作区实施，保留全部已有WIP；不提交、推送、安装、启动GUI或真实提醒。当前51份移植包/54项预置，不增加重复组件条目。

## 已确认行为

- 默认私有。待办与大号日历连接同一种Todo库；番茄钟连接独立Pomodoro库。
- 只调度当前选中的数据源；旧私有数据保留并暂停。隐藏或关闭视图仍调度所选源。
- 断开恢复原私有数据；番茄钟保持暂停，用户显式继续。新库复制场景/记录/设置但不复制正在运行的周期，不自动开始。
- 多实例共享同源只调度一次；最后消费者离开暂停库，其他实例仍连接时不暂停。删除实例只断开它，不删除库。
- 草稿、未确定操作及编辑窗口阻止切源。版本冲突不覆盖；旧会话切源后失效。原云同步继续单列未完成。

## 最小架构

继续使用shared_library元数据/绑定与todo、pomodoro原生文档，不把提醒/计时文档改成普通storage JSON。增加Todo/Pomodoro两种库，原生owner为shared/库UUID；库元数据revision管改名，contentRevision独立记录原生业务revision，状态轮询不加载大文档。私有settings/storage不共享。

Store增加仅用于原生操作与切源的短时串行门闩。生产锁顺序为原生动作门闩→Sessions→SQLite；调度/发送只持门闩，不跨通知调用持SQLite锁。绑定、原生业务操作和删除不能穿过检查→递送间隙。旧通知回调还要核对源仍活动及nonce，暂停更新nonce，避免断开/重连后旧按钮修改恢复的数据。

with_capability在整个原生请求期间保留会话锁，源解析与操作在同一授权生命周期内。各原生reserve和ack重验活动源，调度按owner去重，不依赖窗口。

原生library.create原子复制并连接，返回reloadRequired；客户端识别后直接重载，避免复制与第二次bind间原私有提醒已经发出、复制库却再次发出。普通四类库保持既有两请求兼容。

番茄钟暂停前累计有效已过时间、禁用切源时的自动下一轮；专注暂停复用pause，休息暂停增加rest-pause，恢复必须显式start。复制只保留scenes/history/config/currentSceneId/initialized，phase清空、notice/receipts清空。原周期留在旧源的暂停状态。

## 步骤与检查

1. shared_library.rs先写原生类型/原子新建与私有storage不绕过原生数据的失败反例。再实现类型、native_source/active_native/native_owners、元数据contentRevision与清理。
2. store.rs、widgets.rs添加门闩和with_capability，保持原native接口调用兼容。todo.rs先写双实例共享、旧私有暂停、去重/旧回调、事务回滚检查，再接源解析、copy/pause/remove函数和调度。pomodoro.rs同样接入，覆盖运行/休息暂停和新库不自动计时。
3. 三个标准包升级0.2.0并声明storage.shared。复用原本地库控件，补原子创建响应、native contentRevision、保存队列flush与切换保护。calendar编辑代理必须把beforeLibrarySwitch转接至同一host。补真实Vue渲染和客户端反例。
4. 只构建相关三个资源/ZIP，核对51份包字节一致；原生预置仍54。执行相关→全核心/前端→类型/格式，独立审查后构建新手动程序，不启动。文档明确原云、真实通知/双视图/旧数据升级仍待验。

命令沿用node scripts/desktop.mjs test --locked --offline ... --lib。受限Rust增量复制曾卡住，使用已验证的正常权限离线工具链。备用pnpm入口未就绪时直接执行package.json的同一Node测试与node_modules/typescript/bin/tsc，不安装或改全局配置。

## 进度

- [x] 原生共享类型/元数据/授权与门闩。
- [x] 待办和日历原生源、调度与回调。
- [x] 番茄钟原生源、暂停/复制与恢复。
- [x] 三包控件/版本、审查、构建与手动交接（真实桌面/通知等待手动，未自动启动）。

审查修正：切源前有其他详情会话时拒绝，保护未保存草稿；详情不提供切源按钮。Todo/Pomo删除确认均明确当前共享源影响所有连接实例。两项P2均有先红后绿反例与独立只读复核。全核心258项之后新增待办与大号日历跨包同库、设置隔离及删除保库检查也通过；前端257项、类型/格式、51包650文件字节一致。最终完整核心与主程序构建进行中。

最终全核心259通过/2跳过，前端257通过；主程序构建与固定散列/新隔离入口已完成，84339cedc8b57292232aa565433b3cb877e95cc13a6fc496f2dca3e305622749。原云同步和其他专用能力仍未完成，全目标保持进行中。
