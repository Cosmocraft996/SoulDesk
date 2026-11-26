# DeskBox 桌面格子待补齐清单

基线取自 DeskBox 固定提交 `4cae1d4ebb0e416af7d66a764f52f6ccaafdda3a` 与 SoulDesk 0.5.11 的 DeskBox 设置版。这里只登记原来标为「暂未接入」的内容。需要显示器、选区或目录的命令还受当前上下文影响。

基线共 106 个设置条目。本次为 69 个条目接上控件或已有操作入口，37 个条目仍使用原有禁用提示。以下状态表示代码接入范围，验收证据另列。

## 菜单

| 入口 | 缺项 | DeskBox 源码 | 验收 |
| --- | --- | --- | --- |
| 托盘 | 打开收纳目录、更新入口 | `App.Tray.cs` | 已接入打开整理使用的本地收纳目录。更新仍由 SoulDesk 宿主管理。 |
| 格子标题与空白处 | 边框样式、标题栏样式、文字与可读性、图标大小 | `ContentWidgetWindow.Commands.cs`、`FileSurfaceContent.SelectionAndMenus.cs` | 已接入边框、标题栏、文字模式及颜色、图标大小。覆盖值写入该格子的布局。 |
| 文件右键 | 打开方式、以管理员身份运行、解压、压缩、剪切、复制、创建快捷方式、回收站、重命名、系统菜单 | `Controls/FileItemMenuBuilder.cs` | 已接入打开方式、以管理员身份运行、复制、创建快捷方式、压缩，以及目录映射中的 ZIP 解压。剪切、引用文件解压、回收站、重命名及资源管理器原生右键仍待接入。 |

## 设置基线

### 性能与资源

参考 `Views/SettingsWindow.xaml` 中的 `PerformanceSettings` 分区。

- 性能模式（`Settings.Performance.Mode.Title`） · 待接入
- 可见闲置后台维护（`Settings.Performance.VisibleIdleCleanup.Title`） · 待接入
- 隐藏后缓存回收（`Settings.Performance.HiddenCleanup.Title`） · 待接入
- 隐藏后清理范围（`Settings.Performance.HiddenScope.Title`） · 待接入
- 缓存容量（`Settings.Performance.CacheBudget.Title`） · 待接入
- 空闲时压缩内存占用（`Settings.Performance.IdleWorkingSetTrim.Title`） · 待接入
- 隐藏后立即释放内存(实验性)（`Settings.Performance.ImmediateHiddenWorkingSetTrim.Title`） · 待接入
- 操作结束后压缩内存占用（`Settings.Performance.QuiescenceWorkingSetTrim.Title`） · 待接入

### 外观

参考 `Views/SettingsWindow.xaml` 中的 `Appearance` 分区。

- 应用主题（`Settings.Theme.Title`） · 已接入
- 托盘图标（`Settings.TrayIcon.Title`） · 待接入
- 主题色来源（`Settings.Accent.Source.Title`） · 已接入
- 主题色（`Settings.Accent.Title`） · 已接入

### 胶囊模式

参考 `Views/SettingsSections/CapsuleModeSettingsSection.xaml` 中的 `CapsuleMode` 分区。

- 展开后的宽度（`Settings.Capsule.WidthMode.Title`） · 已接入
- 展开方向（`Settings.Capsule.ExpansionDirection.Title`） · 已接入
- 收起后显示（`Settings.CompactContent.Title`） · 已接入
- 组合排列（`Settings.Capsule.ArrangementDetails.Title`） · 已接入
- 悬停响应（`Settings.Capsule.HoverResponse.Title`） · 已接入
- 悬停展开延迟（`Settings.Capsule.ExpandDelay.Title`） · 已接入
- 移出收起延迟（`Settings.Capsule.CollapseDelay.Title`） · 已接入
- 展开与收起动画（`Settings.Capsule.Animation.Title`） · 已接入
- 过渡时长（`Settings.Capsule.AnimationDuration.Title`） · 已接入
- 隐藏具体内容（`Settings.CompactPrivacy.Title`） · 已接入

### 格子组

参考 `Views/SettingsWindow.xaml` 中的 `WidgetGroups` 分区。

- 标题栏布局（`Settings.WidgetGroupNavigation.Title`） · 已接入
- 名称显示（`Settings.WidgetGroupTitle.Title`） · 已接入
- 切换动画（`Settings.WidgetGroupSwitchAnimation.Title`） · 已接入
- 滚轮切换（`Settings.WidgetGroupWheel.Title`） · 已接入
- 鼠标移入自动切换（`Settings.WidgetGroupHover.Title`） · 已接入

### 文件格子

参考 `Views/SettingsSections/FileWidgetSettingsSection.xaml` 中的 `AppearanceDetail` 分区。

- 文件夹打开方式（`Settings.FileWidget.FolderOpenBehavior.Title`） · 已接入
- 桌面原版右键菜单（`Settings.FileWidget.SystemContextMenu.Title`） · 待接入

### 快捷与交互

参考 `Views/SettingsWindow.xaml` 中的 `Interaction` 分区。

- 格子层级（`Settings.WidgetLayerMode.Title`） · 待接入
- 全局快捷键（`Settings.GlobalHotkey.Title`） · 待接入
- 自定义快捷键（`Settings.GlobalHotkey.CustomTitle`） · 待接入
- 按 Win+D 后（`Settings.ShowDesktopBehavior.Title`） · 待接入
- 打开方式（`Settings.OpenMethod.Title`） · 已接入
- 格子吸附（`Settings.ResizeSnap.Title`） · 已接入
- 格子间距（`Settings.WidgetSnap.Spacing.Title`） · 已接入
- 双击桌面空白区域（`Settings.DesktopDoubleClick.Title`） · 待接入

### 显示器与格子

参考 `Views/SettingsSections/DisplaySettingsSection.xaml` 中的 `Displays` 分区。

- 显示器与格子（`Settings.Displays.Title`） · 已接入管理入口，显示器拓扑图仍待接入
- 全部移到此显示器（`Settings.Displays.MoveAll`） · 已接入
- 新格子的显示器（`Settings.Displays.NewWidgets.Header`） · 已接入
- 显示器断开时（`Settings.Displays.Disconnect.Header`） · 待接入

### 文件显示

参考 `Views/SettingsWindow.xaml` 中的 `FileDisplaySettings` 分区。

- 显示文件后缀名（`Settings.ShowFileExtensions.Title`） · 已接入
- 排除快捷方式 .lnk（`Settings.HideShortcutExtension.Title`） · 已接入
- 图片和视频只显示图标（`Settings.ShowImageFilesAsIcons.Title`） · 已接入
- 隐藏快捷方式小箭头（`Settings.HideShortcutArrow.Title`） · 已接入
- 列表详细信息（`Settings.ListDetails.Title`） · 已接入
- 显示路径提示（`Settings.FilePathTooltips.Title`） · 已接入
- 文件名宽度（`Settings.FileNameWidth.Title`） · 已接入
- 文件名行数（`Settings.FileNameLines.Title`） · 已接入

### 收纳文件夹

参考 `Views/SettingsWindow.xaml` 中的 `ManagedStorage` 分区。

- 已关闭格子的收纳文件夹（`Settings.ManagedStorage.ListTitle`） · 待接入

### 文件叠放

参考 `Views/SettingsWindow.xaml` 中的 `FileStackSettings` 分区。

- 展开方式（`Settings.FileStacks.OpenMode.Title`） · 已接入
- 弹窗布局（`Settings.FileStacks.PopoverLayout.Title`） · 已接入
- 弹窗视觉样式（`Settings.FileStacks.PopoverStyle.Title`） · 已接入
- 自动叠放（`Settings.FileStacks.Auto.Title`） · 已接入
- 自动叠放数量（`Settings.FileStacks.Threshold.Title`） · 已接入
- 分组方式（`Settings.FileStacks.GroupBy.Title`） · 已接入
- 未匹配文件（`Settings.FileStacks.Unmatched.Title`） · 已接入
- 叠放内排序（`Settings.FileStacks.OrderBy.Title`） · 已接入

### 桌面整理

参考 `Views/SettingsSections/DesktopOrganizationSettingsSection.xaml` 中的 `DesktopOrganizationSettings` 分区。

- 撤消（`DesktopOrganization.Notification.Undo`） · 可打开整理历史并选记录撤回
- 设置自动整理（`DesktopOrganization.Auto.SetupTitle`） · 可打开整理页，沿用现有自动整理开关
- 整理时机（`DesktopOrganization.Auto.Interval.Title`） · 已接入
- 整理到此格子（`DesktopOrganization.Rule.Enable.Title`） · 可编辑所选格子规则，沿用现有匹配语法
- 自定义扩展名（`DesktopOrganization.Rule.Extensions`） · 可编辑所选格子规则，沿用现有扩展名匹配语法

### 搜索

参考 `Views/SettingsSections/SearchSettingsSection.xaml` 中的 `SearchSettings` 分区。

- 搜索快捷键（`Settings.Search.Hotkey.Title`） · 待接入
- 自定义搜索快捷键（`Settings.Search.Hotkey.CustomTitle`） · 待接入
- DeskBox 内容（`Settings.Search.Scope.DeskBox`） · 待接入
- 推荐内容（`Settings.Search.Recommendations.Title`） · 待接入
- 默认结果分类（`Settings.Search.DefaultTab.Title`） · 待接入
- 推荐应用图标动效（`Settings.Search.IconAnimation.Title`） · 待接入

### 窗口材质与边框

参考 `Views/SettingsWindow.xaml` 中的 `AppearanceMaterialSettings` 分区。

- 窗口材质与边框（`Settings.Material.Title`） · 已接入
- 背景透明度（`Settings.Opacity.Title`） · 已接入
- 材质浓度（`Settings.MaterialIntensity.Title`） · 已接入
- 格子文字（`Settings.WidgetForeground.Title`） · 已接入
- 文字阴影（`Settings.TextShadow.Title`） · 已接入
- 背景模式（`Settings.WidgetBackground.Mode.Title`） · 已接入
- 背景图片（`Settings.WidgetBackground.Image.Title`） · 已接入
- 契合度（`Settings.WidgetBackground.Fit.Title`） · 已接入
- 背景明暗（`Settings.WidgetBackground.Dim.Title`） · 已接入
- 独立背景（`Settings.WidgetBackground.PerWidget.Title`） · 已接入
- 边框颜色（`Settings.BorderColor.Title`） · 已接入
- 边框样式（`Settings.Border.Title`） · 已接入
- 格子圆角（`Settings.Corner.Title`） · 已接入
- 透明效果(系统)（`Settings.SystemTransparency.Title`） · 待接入
- 窗口阴影(系统)（`Settings.WindowShadow.Title`） · 待接入

### 显示密度

参考 `Views/SettingsWindow.xaml` 中的 `AppearanceDensitySettings` 分区。

- 显示密度（`Settings.Density.Title`） · 已接入
- 整体密度（`Settings.Density.Scale.Title`） · 已接入
- 文字大小（`Settings.TextSize.Title`） · 已接入
- 图标大小（`Settings.IconSize.Title`） · 已接入
- 横向间距（`Settings.HorizontalSpacing.Title`） · 已接入
- 纵向间距（`Settings.VerticalSpacing.Title`） · 已接入

### 格子窗口与标题栏

参考 `Views/SettingsWindow.xaml` 中的 `AppearanceWindowSettings` 分区。

- 格子标题图标（`Settings.WidgetTitleIcon.Title`） · 已接入
- 展示格子标题样式（`Settings.DisplayWidgetChrome.Title`） · 待接入
- 内容格子标题样式（`Settings.InteractiveWidgetChrome.Title`） · 已接入
- 默认宽度（`Settings.DefaultWidth.Title`） · 已接入

### 显示与隐藏动画

参考 `Views/SettingsWindow.xaml` 中的 `AppearanceAnimationSettings` 分区。

- 显示与隐藏动画（`Settings.Group.Animation.Title`） · 待接入
- 显示动画（`Settings.Animation.Effect.Title`） · 待接入
- 动画速度（`Settings.Animation.Speed.Title`） · 待接入
- 滑动方向（`Settings.Animation.Direction.Title`） · 待接入
- 缓动强度（`Settings.Animation.Easing.Title`） · 待接入
- 错峰滑动（`Settings.Animation.Stagger.Title`） · 待接入

### 已单独设置的格子

参考 `Views/SettingsWindow.xaml` 中的 `CapsuleOverridesSettings` 分区。

- 全部恢复跟随全局（`Settings.Capsule.Overrides.ResetAll.Title`） · 已接入

### 设置与内容备份

参考 `Views/SettingsWindow.xaml` 中的 `BackupRestoreSettings` 分区。

- 自动备份（`Settings.DataBackup.AutomaticBackup.Title`） · 待接入
- 备份间隔（`Settings.DataBackup.AutomaticBackupInterval.Title`） · 待接入
- 保留快照份数（`Settings.DataBackup.AutomaticBackupRetention.Title`） · 待接入
- 备份目录（`Settings.DataBackup.AutomaticBackupDirectory.Title`） · 待接入
- 本机快照（`Settings.DataBackup.Snapshots.Title`） · 待接入

## 宿主与其他组件入口

「常规」里的语言、自启动、启动方式、静默启动和新手引导属于整应用。「功能格子」下的随记、待办、音乐、天气、个人看板为独立组件。「诊断与维护」中的云备份、附件巡检及「关于」中的升级与发布说明需要宿主能力。相关入口按原版保留。

## 验收顺序

1. 编译 WinForms 引擎和 WinUI 界面，检查旧布局和新设置的读写。
2. 在隔离桌面打开实际控件，核对已接入、暂未接入及因当前对象不可用三类状态。
3. 在隔离 Windows 的真实 Explorer 核对拖放、菜单配置、停用后原生图标和源文件。
4. 只对通过上述验证的版本生成本地安装包。

## 本轮验证结果

- 格子引擎和 WinUI 编译通过。模型检查覆盖旧布局读写、设置导出、无效选项拦截及原文件引用。隐藏桌面打开了 30 个设置页，并通过实际控件保存了胶囊排列和文件后缀名选项。
- 两轮真实 Explorer 都通过了展开与折叠格子的拖动、快捷方式拖入及退出后恢复原生图标。通过的用例中，测试项目的原路径、属性与内容散列一致。
- 首轮综合用例确认了自定义堆栈、标题图片及背景图、主屏绑定、Ctrl 联动拖动与胶囊贴底。原文件搬移用例未打开搬移预览。
- 第二轮综合用例在 Ctrl 联动通过之后，测试程序发出的收起模式请求与桌面刷新产生版本冲突。验收脚本现可识别该冲突并重取一次布局状态，综合用例尚未复测通过。
- 多显示器拓扑变化、原文件搬移回退以及逐项文件右键命令仍需验收。本轮未生成安装包。

本地验收日志位于 `artifacts/desktop-icons-handoff-probe/vm/ExplorerPreferences-candidate1.log` 和 `artifacts/desktop-icons-handoff-probe/vm/ExplorerPreferences-candidate2.log`。隔离 Windows 已关机并恢复原登录脚本。

## 2026-11-26 已接入功能专项验收

下表记录修复前的专项验收结果。修复后的结论见下节。

| 项目 | 结果 | 证据 |
| --- | --- | --- |
| 设置存储与默认值 | 通过 | 63 个存储键共验证 163 个有效值写入、发布和重新打开。每个键均验证了无效值拦截和恢复默认，另核对新格子显示器与独立外观覆盖。 |
| WinUI 设置与标题菜单 | 通过所测范围 | 隐藏桌面打开 30 个设置页，实际操作了胶囊排列和显示文件后缀名开关。标题菜单构建了 61 项，包含边框、标题样式、文字与可读性、显示器绑定。 |
| 格子在真实 Explorer 中的显示与交互 | 通过所测范围 | 两轮专项均验证自定义堆栈、标题图标、背景图、主屏绑定、Ctrl 联动拖动、胶囊贴底及文件后缀名即时生效。 |
| 原文件搬移与整理历史 | 通过 | 两轮专项均验证测试文件离开原路径、写入收纳目录并产生历史记录。撤回后重新出现在原路径，文件内容一致。 |
| 文件右键复制与创建快捷方式 | 通过 | 首次通过界面自动化调用复制时未读到文件列表。使用真实鼠标复测后，Windows 剪贴板包含样本文件路径，快捷方式成功生成，原文件内容一致。退出后原生图标恢复。 |
| 标签切换 | 未通过 | 切换成功写入布局并更新标题，随后旧窗口句柄失效。窗口清理用窗口和内容的元组匹配，替换内容时误关闭了本应复用的窗口。 |
| 合组和拆分 | 未完成界面验收 | 两轮都定位不到可操作的标题菜单入口，未触发合组和拆分。已有数据模型检查通过。 |
| 整理结束后的退出 | 未通过 | 第一轮在整理历史仍打开时停止宿主，Explorer 已复原，格子引擎记录空引用异常。第二轮整理后旧窗口句柄失效，追加的文件菜单检查在退出步骤前提前结束。 |

压缩、解压、打开方式和管理员启动尚未逐项实测。专项验收结论为部分通过。

验收日志位于 `artifacts/desktop-icons-handoff-probe/vm/` 下的 `ExplorerPreferences-accept1.log`、`ExplorerPreferences-accept2.log`、`ExplorerPreferences-file-menu1.log`、`ExplorerPreferences-file-menu2.log`。

## 2026-11-26 修复后回归

- 标签切换按窗口身份判断是否需要关闭。真实 Explorer 上切换标签后保留原窗口，手动收起后保留当前标签。
- 整理预览和确认时重新读取桌面文件。真实 Explorer 在接管后新建样本文件，预览找到该文件并搬移成功。从历史撤回后，原路径和内容一致。
- 宿主退出时先交还 Explorer，再结束整理历史对话框及其消息循环。真实 Explorer 用例保持历史窗口打开，停止宿主后引擎正常退出，原生桌面图标恢复。
- 合组和拆组均通过实际菜单完成。合组保留目标窗口，切换活动标签继续复用该窗口，拆出当前标签后两个格子仍保留各自的文件引用。

同一构建在隔离 Windows 的真实 Explorer 上通过 6 项回归，覆盖标签切换与手动折叠、展开和折叠状态下的拖动、菜单合组与拆分、整理及退出、快捷方式从胶囊边缘拖入。每项都核对退出后图标可用，以及测试文件的原路径、属性和 SHA-256。模型和隐藏 WinUI 桌面的相关回归也通过。

最终回归日志和二进制散列位于 `artifacts/desktop-icons-handoff-probe/vm/ExplorerGridRepair-final.log`。
