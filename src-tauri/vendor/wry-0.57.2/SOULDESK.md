# SoulDesk 的 Wry 本地补丁

源码来自 crates.io 的 `wry` 0.57.2 发布包，归档 SHA-256 为 `a490119196d02a51d0871df7b9119a1f3e46d75b7e90053a801454d5c6b3491c`。授权见同目录的 MIT 和 Apache-2.0 许可文件。

补丁位于 `src/webview2/mod.rs`。Windows 侧边面板先提交消息桥脚本注册，再注册消息接收和自定义协议处理器，随后等待脚本注册完成。后续初始化脚本及首次导航依赖这个确认。Tauri 运行时传入的宿主窗口标签 `panel` 用于限定补丁范围。

父窗口子类回调在调整大小、焦点或位置时持有独立的 COM 引用，覆盖调用期间重入窗口销毁的情形。只有成功卸载回调的一方才释放注册时的引用。注册失败时立即回收该引用。[微软 COM 引用计数规则](https://learn.microsoft.com/windows/win32/com/rules-for-managing-reference-counts)

升级 Wry 时复核 IPC 脚本注册时序、窗口标签映射和父窗口回调的释放顺序，并运行 `scripts/verify-panel.mjs` 覆盖首次加载、权限和面板生命周期。
