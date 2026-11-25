# SoulDesk 的 Tauri 运行时补丁

源码来自 crates.io 的 `tauri-runtime-wry` 2.12.3 发布归档，SHA-256 为 `e1aa9d8b7a120f879f44fdc1053f7b7dbf416bf0bbc6a72b714dec46b782b52f`。许可见本目录的 Apache-2.0 和 MIT 文件。

回移植 Tauri 上游提交 [84af754a](https://github.com/tauri-apps/tauri/commit/84af754a644920c333c365184d9aa31d688fd055) 中窗口事件循环对象的所有权修复。只有界面线程上的运行时持有强引用。后台复制窗口或应用句柄时只复制原子的弱引用，界面线程负责提升引用和读取原生显示句柄。这样保留了既有异步建窗入口和窗口生命周期。

升级运行时时核对上游版本已包含该提交，并重新检查通知打开 K 线、面板建窗与退出。
