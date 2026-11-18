# SoulDesk 的 Wry 本地补丁

源码来自 crates.io 的 `wry` 0.57.2 发布包，归档 SHA-256 为 `a490119196d02a51d0871df7b9119a1f3e46d75b7e90053a801454d5c6b3491c`。授权见同目录的 MIT 和 Apache-2.0 许可文件。

补丁位于 `src/webview2/mod.rs`。Windows 侧边面板先提交消息桥脚本注册，再注册消息接收和自定义协议处理器，随后等待脚本注册完成。后续初始化脚本及首次导航依赖这个确认。Tauri 运行时传入的宿主窗口标签 `panel` 用于限定补丁范围。

升级 Wry 时复核 IPC 脚本注册时序和窗口标签映射，并运行 `scripts/verify-panel.mjs` 覆盖首次加载、权限和面板生命周期。
