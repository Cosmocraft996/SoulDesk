# 共享待办与番茄钟的编辑版本保护

用户已批准可选共享库。本步骤先解决多个视图编辑同一条数据时的丢失更新；不依赖后台数据源切换政策，不改变后台调度。后台暂停/复制政策已另向用户提问，在其答复前不实施受影响的后台切源。

## 方案

复用现有todo/pomodoro expectedRevision与请求UUID重试。模型已有“冲突后读取最新值”用于非编辑动作，保留该约定；编辑表单独立固定打开时revision，在每次save payload覆盖expectedRevision，冲突后仍使用原打开版本，不悄悄借轮询或冲突重读的新版本提交旧草稿。取消/重新打开代表用户重新读取并审阅新数据。存储格式和原接口不变。

待办/大号日历共用todo App：openNew/openEdit记录editRevision；saveTodo携带该版本；定时刷新在任何编辑/确认/主题窗口打开时暂停，窗口关闭后恢复。编辑日期、重复及提醒仍用原转换。番茄钟场景编辑与配置草稿同样记录revision；场景保存携带sceneRevision，进入配置页时记录configRevision，成功保存后才推进该草稿基线。编辑/配置时暂停界面轮询，核心计时/提醒仍继续，原不确定请求只能重试原UUID。

## 文件与验证

- scripts/widgetjs-todo-render.test.mjs：真实Vue打开表单→第一次save收到REVISION_CONFLICT且模型自动读取revision2→第二次save仍带旧revision0；草稿保留，关闭/重新打开后才带新revision2。涵盖共用大日历编辑器，不改大日历原引擎。
- scripts/widgetjs-pomodoro-render.test.mjs：相同场景草稿反例；配置页固定版本并在成功保存后推进。每项先失败再实现。
- components/widgetjs/todo-source/App.vue、pomodoro-source/App.vue：只加上述两个/三个草稿版本字段与轮询窗口检查。

命令node --test scripts/widgetjs-todo-render.test.mjs scripts/widgetjs-pomodoro-render.test.mjs，预期全部通过并自然退出。随后原todo/pomodoro模型检查通过。待后台共享接入批次完成后一起重建/升版和隔离交接，不把源码检查冒充实机双窗口验收，不提交或自动启动GUI。

## 状态

- [x] 写实际Vue版本冲突反例并确认失败。
- [x] 固定编辑版本并检查轮询边界。
- [x] 最小回归与0.1.1独立标准包交付。
- [ ] 随原生共享接入统一重建主程序并进行实机验收。

- 实际反例集中在scripts/widgetjs-shared-edit-render.test.mjs，避免复制原检查器；两项先因版本1→2自动放行失败，固定草稿版本后与原模型/模板合计22项通过。首次通过断言后测试清理调用了番茄钟不存在的dispose，已改用实际store.dispose并自然退出0。尚未重建相关组件包，待后台共享政策及原生接入后一起构建/交接。

- 独立审查补充3项P2：删除确认版本、configure原UUID重试基线、等待期间导航旧配置；均补真实Vue失败反例并修复，26项相关检查通过。全前端245项、5项最终预置检查（覆盖53项核心包管线）、类型/格式通过；50个ZIP共644文件匹配当前输入。todo/calendar-large/pomodoro升0.1.1，旧包保留。现有9cee宿主接口可导入，不为纯前端补丁重建固定主程序；实际多窗口/非空升级仍未验。
