# 宿主运行资源与测试夹具

宿主原生能力直接使用这些文件。组件界面、可安装资源及完整来源记录在[组件广场](https://github.com/Cosmocraft996/SoulDesk-Widgets/tree/main/source/widgetjs-port/components/widgetjs)。

| 文件 | 宿主用途 | 迁移源目录 |
| --- | --- | --- |
| `ai-prompts.json` | AI 生成任务的宿主提示词 | `ai-source/` |
| `coins.json` | 公开行情键与固定币种 | `coin-common/` |
| `job-prompts.json`、`job-capture.js`、`job-login.js` | 固定网页会话和原生 AI 任务 | `job-source/` |
| `pomodoro-ding.wav`、`pomodoro-tray.rgba` | 原生计时声音及托盘图标 | `pomodoro-source/` |
| `photo-fixture.json`、`key-stroke-manifest.json`、`launcher-fixture.png` | 隔离验收及单元测试 | `photo-common/`、`key-stroke/`、`grid/` |
