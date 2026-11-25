using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Security.Principal;
using System.Text;
using System.Web.Script.Serialization;
using System.Windows.Automation;
using System.Windows.Forms;
using SoulDesk.DesktopGrid;

internal static partial class GridAcceptance
{
    private static GridLease VerifyRendererDisplayLayout(string root, GridLease lease, IntPtr window,
        string renderer, string driver, bool mixed, Action<string> verifyUnchanged,
        Action<GridLease> beforeDisconnect = null)
    {
        VerifyGuestIdentity();
        using (var user = WindowsIdentity.GetCurrent())
            Require(new WindowsPrincipal(user).IsInRole(WindowsBuiltInRole.Administrator), "访客的虚拟显示驱动测试需要管理员令牌");
        var displays = LiveDisplays();
        Require(displays.Length == 1 && displays[0].Primary, "格子跨屏验收必须从原始单屏开始");
        var primary = displays[0];
        Require(!mixed || Native.Scale(primary, 96) == 96, "混合缩放验收要求主屏保持 100%");
        var single = VerifyRendererPlacement(root, lease, window, renderer, false, false);
        string device = null;
        bool restored = false;
        try
        {
            CreateDisplayDevice(driver, out device);
            int extend = ApplyTopology(0x84);
            Wait(() => (displays = LiveDisplays()).Length == 2,
                45000, "WinUI 验收未打开副屏 · SetDisplayConfig=" + extend);
            var secondary = displays.Single(screen => !screen.Primary);
            lease = WaitForDisplayHandoff(root, lease);
            if (mixed && Native.Scale(secondary, 96) != 144)
            {
                using (Process.Start("ms-settings:display")) { }
                Trace("GRID_DISPLAY_WAIT_SCALE " + secondary.DeviceName + " " + secondary.Bounds);
                Wait(() =>
                {
                    displays = LiveDisplays();
                    return displays.Length == 2 && Native.Scale(displays.Single(screen => screen.Primary), 96) == 96 &&
                        Native.Scale(displays.Single(screen => !screen.Primary), 96) == 144 && !DisplaySettingsForeground();
                }, 150000, "隔离访客未得到主屏 100% 和副屏 150% 的真实布局");
                secondary = displays.Single(screen => !screen.Primary);
                // 设置分辨率和缩放可能连续触发接管，完成标记只记录最近一轮。
                lease = WaitForDisplayHandoff(root, lease, false);
            }
            var targetMonitor = Native.MonitorId(secondary);
            int mainDpi = Native.Scale(primary, 96);
            int secondaryDpi = Native.Scale(secondary, 96);
            Trace("GRID_DISPLAY_DPI " + mainDpi + " " + secondaryDpi);
            if (mixed) verifyUnchanged("设置混合缩放后");
            var current = VerifyActiveRenderer(root, renderer, lease);
            WindowRect bounds;
            Require(GetWindowRect(current, out bounds), "无法读取跨屏移动前的窗口");
            var move = new Point(secondary.WorkingArea.Left + 105 - bounds.Left,
                secondary.WorkingArea.Top + (mixed && secondary.WorkingArea.Height < 850 ? 35 : 305) - bounds.Top);
            var dual = VerifyRendererPlacement(root, lease, current, renderer, false, false, move, targetMonitor);
            Require(secondary.Bounds.Contains((dual.Left + dual.Right) / 2, (dual.Top + dual.Bottom) / 2),
                "拖动后格子窗口没有到达新增显示器");
            Require(!mixed || GetDpiForWindow(VerifyActiveRenderer(root, renderer, lease)) == 144,
                "跨屏后的 WinUI 窗口未使用副屏 150% 缩放");
            Trace("GRID_DISPLAY_PLACEMENT " + RendererGeometry(root, lease, dual));
            if (mixed) verifyUnchanged("跨屏拖动后");

            SetDeviceEnabled(device, false);
            Wait(() => LiveDisplays().Length == 1, 30000, "Windows 未完成副屏断开");
            lease = WaitForDisplayHandoff(root, lease);
            Wait(() => RendererAt(root, lease, single), 10000, "拔除副屏后 WinUI 格子没有恢复单屏位置",
                () => RendererGeometry(root, lease, single));
            Trace("GRID_DISPLAY_DETACHED " + RendererGeometry(root, lease, single));
            if (mixed) verifyUnchanged("拔除副屏后");
            SetDeviceEnabled(device, true);
            ApplyTopology(0x84);
            Wait(() => LiveDisplays().Length == 2, 30000, "Windows 未完成副屏重连");
            lease = WaitForDisplayHandoff(root, lease);
            Wait(() => RendererAt(root, lease, dual), 10000, "重新接入副屏后 WinUI 格子没有恢复副屏位置",
                () => RendererGeometry(root, lease, dual));
            var rejoinedWindow = VerifyActiveRenderer(root, renderer, lease);
            Require(!mixed || GetDpiForWindow(rejoinedWindow) == 144, "重新接入后的格子未使用 150% 缩放");
            Trace("GRID_DISPLAY_REATTACHED " + RendererGeometry(root, lease, dual));
            if (mixed)
            {
                verifyUnchanged("重新接入副屏后");
                lease = RestartTestExplorer(root, lease, renderer);
                Require(LiveDisplays().Length == 2 && GetDpiForWindow(VerifyActiveRenderer(root, renderer, lease)) == 144,
                    "Explorer 重启后格子没有回到 150% 副屏");
                Wait(() => RendererAt(root, lease, dual), 10000, "Explorer 重启后格子没有恢复副屏位置",
                    () => RendererGeometry(root, lease, dual));
                Trace("GRID_DISPLAY_EXPLORER_RESTARTED " + RendererGeometry(root, lease, dual));
                verifyUnchanged("混合缩放下 Explorer 重启后");
            }
            if (beforeDisconnect != null)
                beforeDisconnect(lease);
            restored = true;
            Trace("GRID_WINUI_DISPLAYS_PASS " + mainDpi + " " + secondaryDpi);
        }
        finally
        {
            if (device != null)
            {
                SetDeviceEnabled(device, false);
                Wait(() => LiveDisplays().Length == 1, 30000, "虚拟副屏未关闭");
                if (restored && beforeDisconnect == null) lease = WaitForDisplayHandoff(root, lease);
            }
        }
        return lease;
    }

    private static int VerifyInstalledMixedDisplayHost(string installer, string driver)
    {
        VerifyGuestIdentity();
        var data = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "local.souldesk.desktop");
        var root = Path.Combine(data, "desktop-grid");
        var recovery = new Recovery(root);
        var previous = recovery.Current();
        Require(previous == null || recovery.Signaled("done", previous.Token), "隔离访客仍有未结束的格子会话");
        string renderer;
        string executable = InstallGridHost(installer, out renderer);
        string document, shortcut;
        PrepareDesktopFixtures(out document, out shortcut);
        var store = new GridStore(root);
        store.Change(state =>
        {
            if (state.Boxes.Count == 0)
                state.Add("安装包双屏验收", null);
            var box = state.Boxes.Single();
            Require(box.Current().Name == "安装包双屏验收", "隔离数据中的格子来源不匹配");
            box.Collapsed = false;
            box.HoverExpand = false;
            state.Release(new[] { document, shortcut });
            state.Attach(box.Current().Id, new[] { document });
        });
        string fingerprint = Fingerprint(document, shortcut);
        List<DesktopItem> baseline = null;
        Wait(() => (baseline = ReadIcons(document, shortcut)) != null, 20000, "安装后无法读取测试图标的坐标");
        var originalBounds = baseline[1].NativeBounds;
        var position = CenterIcon(originalBounds);
        DoubleClickAndCheck(position, "安装版接管前的原生快捷方式");
        Trace("GRID_INSTALLED_NATIVE_BASELINE_PASS " + position);
        using (var host = StartInstalledHostFromExplorer(executable))
        {
            try
            {
                var main = WaitForWindow(host.Id, "SoulDesk · 组件管理", 65000);
                ClickNamed(AutomationElement.FromHandle(main), "设置");
                ClickNamed(AutomationElement.FromHandle(main), "打开格子管理");
                int productPid;
                var manager = WaitForGridManager(data, out productPid);
                string engineSha256;
                using (var engine = Process.GetProcessById(productPid))
                    engineSha256 = Digest(engine.MainModule.FileName);
                var lease = EnableInstalledGrid(manager, recovery, productPid, previous);
                VerifyActiveRenderer(root, renderer, lease);
                PostMessage(main, 0x10, IntPtr.Zero, IntPtr.Zero);
                PostMessage(manager, 0x10, IntPtr.Zero, IntPtr.Zero);
                Wait(() => !Native.IsWindowVisible(main) && !Native.IsWindowVisible(manager),
                    10000, "安装版管理窗口尚未收起");
                try { VerifyProjectedShortcut(productPid, shortcut, position, "安装版设置启用后", originalBounds); }
                catch (TimeoutException)
                {
                    Trace("GRID_INSTALLED_SETTINGS_DETAIL " + ManagerContents(manager));
                    throw;
                }
                Trace("GRID_INSTALLED_SETTINGS_PASS");

                ClickTrayItem("恢复 Windows 桌面图标");
                var native = WaitForInstalledDesktop(recovery, lease, document, shortcut);
                Require(CenterIcon(native[1].NativeBounds) == position, "托盘恢复后的原生快捷方式偏离原位");
                DoubleClickAndCheck(position, "安装版托盘恢复后");
                Trace("GRID_INSTALLED_TRAY_RESTORE_PASS");

                ClickTrayItem();
                manager = WaitForGridManager(data, out productPid);
                lease = EnableInstalledGrid(manager, recovery, productPid, lease);
                var grid = VerifyActiveRenderer(root, renderer, lease);
                PostMessage(manager, 0x10, IntPtr.Zero, IntPtr.Zero);
                Wait(() => !Native.IsWindowVisible(manager), 10000, "托盘打开的格子管理窗口尚未收起");
                VerifyProjectedShortcut(productPid, shortcut, position, "安装版托盘启用后", originalBounds);
                Trace("GRID_INSTALLED_TRAY_OPEN_PASS");
                int originalExplorerPid = lease.Desktop.Pid;
                int finalExplorerPid = 0;
                VerifyRendererDisplayLayout(root, lease, grid, renderer, driver, true,
                    stage =>
                    {
                        VerifyProjectedShortcut(productPid, shortcut, position, stage, originalBounds);
                        Require(Fingerprint(document, shortcut) == fingerprint, stage + "样本文件的路径、内容或属性发生变化");
                    },
                    dual =>
                    {
                        finalExplorerPid = dual.Desktop.Pid;
                        Require(finalExplorerPid != originalExplorerPid, "安装版没有连接到重启后的 Explorer");
                        ClickTrayItem("退出 SoulDesk");
                        Require(host.WaitForExit(25000) && host.ExitCode == 0, "安装版未在双屏接管时正常退出");
                        var nativeDual = WaitForInstalledDesktop(recovery, dual, document, shortcut);
                        Require(LiveDisplays().Length == 2, "原生桌面退出验证时副屏已断开");
                        var nativePosition = CenterIcon(nativeDual[1].NativeBounds);
                        Wait(() => HitPid(nativePosition) == finalExplorerPid, 7000, "双屏退出后鼠标没有命中 Explorer");
                        DoubleClickAndCheck(nativePosition, "安装版双屏退出后");
                        Require(Fingerprint(document, shortcut) == fingerprint, "双屏退出后样本文件的路径、内容或属性发生变化");
                        Trace("GRID_INSTALLED_DUAL_EXIT_PASS " + nativePosition);
                    });
                List<DesktopItem> restored = null;
                Wait(() => (restored = ReadIcons(document, shortcut)) != null &&
                    CenterIcon(restored[0].NativeBounds) == CenterIcon(baseline[0].NativeBounds) &&
                    CenterIcon(restored[1].NativeBounds) == position, 15000, "退出并断开副屏后，原生图标未恢复原位");
                DoubleClickAndCheck(position, "安装版退出并回到单屏后");
                Require(Fingerprint(document, shortcut) == fingerprint, "结束后文件或快捷方式已发生变化");
                var report = new
                {
                    passed = true, isolatedGuest = true, installed = true, actualExplorer = true,
                    settingsOpenedGrid = true, trayOpenedGrid = true, trayRestoredIcons = true,
                    trayExitedHostWhileDual = true, dualDisplayLayoutRestored = true,
                    originalExplorerPid, finalExplorerPid,
                    unchangedUngroupedPositionAndClick = true, nativeIconsRestoredAndClickable = true,
                    pathsAttributesAndSha256Preserved = true, mainDpi = 96, secondaryDpi = 144,
                    setupSha256 = Digest(installer), hostSha256 = Digest(executable), engineSha256,
                    rendererSha256 = Digest(Path.ChangeExtension(renderer, ".dll"))
                };
                Trace("GRID_INSTALLED_MIXED_REPORT " + new JavaScriptSerializer().Serialize(report));
                return 0;
            }
            finally
            {
                try { recovery.RequestRestore(); }
                finally { if (!host.HasExited) host.Kill(); }
            }
        }
    }

    private static GridLease EnableInstalledGrid(IntPtr manager, Recovery recovery, int productPid, GridLease previous)
    {
        GridLease lease = null;
        EnableGrid(manager);
        Wait(() => (lease = recovery.Current()) != null && lease.Owner.Pid == productPid &&
            (previous == null || lease.Token != previous.Token) && recovery.Signaled("ready", lease.Token) &&
            ShellDesktop.Matches(lease.Desktop) && !Native.IsWindowVisible(lease.Desktop.Hwnd), 85000,
            "安装版未完成桌面图标接管", () => ManagerContents(manager));
        return lease;
    }

    private static List<DesktopItem> WaitForInstalledDesktop(Recovery recovery, GridLease lease, string document,
        string shortcut)
    {
        List<DesktopItem> restored = null;
        Wait(() => (restored = ReadIcons(document, shortcut)) != null && recovery.Signaled("done", lease.Token),
            22000, "安装版未恢复原生桌面图标");
        Wait(() =>
        {
            using (var renderer = lease.Renderer.OpenProcess()) return renderer == null;
        }, 8000, "原生图标恢复后，格子界面仍在运行");
        return restored;
    }

    private static Process StartInstalledHostFromExplorer(string executable)
    {
        // 验收控制台需要切换虚拟设备，宿主由当前 Explorer 以普通用户权限启动。
        dynamic shell = Activator.CreateInstance(Type.GetTypeFromProgID("Shell.Application", true));
        dynamic windows = shell.Windows();
        object desktop = 0, empty = null;
        int handle;
        dynamic view = windows.FindWindowSW(ref desktop, ref empty, 8, out handle, 1);
        Require(handle != 0 && view != null, "找不到访客的 Explorer 桌面自动化入口");
        view.Document.Application.ShellExecute(executable, "--show", Path.GetDirectoryName(executable), "open", 1);
        int pid = 0;
        Wait(() =>
        {
            foreach (var process in Process.GetProcessesByName("souldesk"))
                using (process)
                    try
                    {
                        if (References.Equal(process.MainModule.FileName, executable))
                            pid = process.Id;
                    }
                    catch (InvalidOperationException) { }
            return pid != 0;
        }, 20000, "Explorer 未启动访客中的安装版 SoulDesk");
        var host = Process.GetProcessById(pid);
        IntPtr token;
        Require(OpenProcessToken(host.Handle, 8, out token), "无法读取安装版的进程权限");
        try
        {
            int elevated, size;
            Require(GetTokenInformation(token, 20, out elevated, sizeof(int), out size) && elevated == 0,
                "安装版应以普通用户权限运行");
        }
        finally { CloseHandle(token); }
        Trace("GRID_INSTALLED_HOST_STANDARD_USER");
        return host;
    }

    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);

    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern bool GetTokenInformation(IntPtr token, int informationClass, out int value, int length, out int size);

    private static bool DisplaySettingsForeground()
    {
        uint pid;
        Native.GetWindowThreadProcessId(GetForegroundWindow(), out pid);
        if (pid == 0) return false;
        try
        {
            using (var process = Process.GetProcessById((int)pid))
                return process.ProcessName == "SystemSettings" || process.ProcessName == "ApplicationFrameHost";
        }
        catch (ArgumentException) { return false; }
    }

    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr window);

    private static GridLease WaitForDisplayHandoff(string root, GridLease previous, bool requirePreviousDone = true)
    {
        var recovery = new Recovery(root);
        GridLease next = null;
        Wait(() =>
        {
            next = recovery.Current();
            return next != null && next.Token != previous.Token && next.RendererSession != previous.RendererSession &&
                (!requirePreviousDone || recovery.Signaled("done", previous.Token)) && recovery.Signaled("ready", next.Token) &&
                !Native.IsWindowVisible(next.Desktop.Hwnd);
        }, 85000, "显示器变更后未完成桌面图标的重新接管", () => Error(root));
        Trace("GRID_DISPLAY_HANDOFF " + next.RendererSession);
        return next;
    }

    private static Screen[] LiveDisplays()
    {
        // 此验收是控制台进程，直接枚举当前拓扑，避免依赖 WinForms 的显示变更消息缓存。
        Application.DoEvents();
        var screens = new List<Screen>();
        Require(EnumDisplayMonitors(IntPtr.Zero, IntPtr.Zero,
            (IntPtr monitor, IntPtr dc, ref WindowRect bounds, IntPtr state) =>
            {
                screens.Add(Screen.FromPoint(new Point((bounds.Left + bounds.Right) / 2, (bounds.Top + bounds.Bottom) / 2)));
                return true;
            }, IntPtr.Zero), "无法枚举 Windows 的实时显示器");
        return screens.ToArray();
    }

    private delegate bool MonitorCallback(IntPtr monitor, IntPtr dc, ref WindowRect bounds, IntPtr state);
    [DllImport("user32.dll")]
    private static extern bool EnumDisplayMonitors(IntPtr dc, IntPtr rectangle, MonitorCallback callback, IntPtr state);

    private static bool TryReadRendererBounds(string root, GridLease lease, out WindowRect bounds)
    {
        bounds = new WindowRect();
        string path = Path.Combine(root, "deskbox-renderer-" + lease.RendererSession, "pulse.json");
        var pulse = LocalJson.TryRead<GridRendererPulse>(path);
        if (pulse == null || pulse.Session != lease.RendererSession || pulse.Pid != lease.Renderer.Pid ||
            pulse.Windows == null || pulse.Windows.Length != 1)
            return false;
        var window = new IntPtr(pulse.Windows[0]);
        return Native.IsWindowVisible(window) && GetWindowRect(window, out bounds);
    }

    private static bool RendererAt(string root, GridLease lease, WindowRect expected)
    {
        WindowRect current;
        return TryReadRendererBounds(root, lease, out current) &&
            Math.Abs(current.Left - expected.Left) <= 2 && Math.Abs(current.Top - expected.Top) <= 2 &&
            current.Right - current.Left == expected.Right - expected.Left &&
            current.Bottom - current.Top == expected.Bottom - expected.Top;
    }

    private static string RendererGeometry(string root, GridLease lease, WindowRect expected)
    {
        var target = Rectangle.FromLTRB(expected.Left, expected.Top, expected.Right, expected.Bottom);
        WindowRect actual;
        string actualBounds = TryReadRendererBounds(root, lease, out actual)
            ? Rectangle.FromLTRB(actual.Left, actual.Top, actual.Right, actual.Bottom).ToString() : "不可见";
        string directory = Path.Combine(root, "deskbox-renderer-" + lease.RendererSession);
        var state = LocalJson.TryRead<GridRenderState>(Path.Combine(directory, "state.json"));
        var projected = state == null ? "无" : string.Join("；", state.Boxes.Select(box =>
            new Rectangle(box.X, box.Y, box.Width, box.Height).ToString()));
        return "预期 " + target + " / 窗口 " + actualBounds + " / 引擎 " + projected;
    }

    private static int VerifyDisplayTopology(string driver, string existingDevice = null)
    {
        VerifyGuestIdentity();
        using (var user = WindowsIdentity.GetCurrent())
            Require(new WindowsPrincipal(user).IsInRole(WindowsBuiltInRole.Administrator), "访客的虚拟显示驱动测试需要管理员令牌");
        string device = null;
        try
        {
            var native = Screen.PrimaryScreen;
            Require(Screen.AllScreens.Length == 1 && native != null, "访客必须以原始单屏拓扑开始");
            string primary = Native.MonitorId(native);
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            string root = Path.Combine(GuestRoot, "display-" + Guid.NewGuid().ToString("N"));
            using (var app = new GridApplication(root, true, null))
            {
                app.Edit(state => state.Add("显示器验收", null));
                Application.DoEvents();
                var first = Application.OpenForms.OfType<GridSurface>().Single();
                var single = new Rectangle(native.WorkingArea.Left + 90, native.WorkingArea.Top + 130, 312, 240);
                app.Place(first.Box, single);
                single = Application.OpenForms.OfType<GridSurface>().Single().ScreenBounds;
                if (existingDevice == null)
                {
                    Trace("GRID_DISPLAY_INSTALL");
                    CreateDisplayDevice(driver, out device);
                }
                else
                {
                    Require(existingDevice.StartsWith(@"ROOT\DISPLAY\", StringComparison.OrdinalIgnoreCase) &&
                        existingDevice.Length == @"ROOT\DISPLAY\0000".Length &&
                        existingDevice.Skip(@"ROOT\DISPLAY\".Length).All(value => value >= '0' && value <= '9'),
                        "已安装的虚拟显示节点标识无效");
                    string installed = RunDisplayTool("/enum-devices /instanceid \"" + existingDevice + "\" /drivers",
                        12000, "确认隔离测试显示驱动");
                    Require(installed.IndexOf(@"Root\MttVDD", StringComparison.OrdinalIgnoreCase) >= 0,
                        "已登记的设备未绑定预期测试驱动");
                    device = existingDevice;
                    SetDeviceEnabled(device, true);
                }
                int extend = ApplyTopology(0x84);
                Wait(() => { Application.DoEvents(); return Screen.AllScreens.Length >= 2; },
                    45000, "Windows 未激活第二块显示器 · SetDisplayConfig=" + extend);
                var second = Screen.AllScreens.First(screen => Native.MonitorId(screen) != primary);
                var target = new Rectangle(second.WorkingArea.Left + 105, second.WorkingArea.Top + 155, 308, 232);
                Wait(() => { Application.DoEvents(); return Application.OpenForms.OfType<GridSurface>().Any(); },
                    6000, "双屏窗口刷新未完成");
                app.Place(Application.OpenForms.OfType<GridSurface>().Single().Box, target);
                var dual = Application.OpenForms.OfType<GridSurface>().Single().ScreenBounds;
                Require(Screen.FromRectangle(dual).DeviceName == second.DeviceName, "未能把格子移到新增的显示器");
                int nativeDpi = Native.Scale(native, 96), secondDpi = Native.Scale(second, 96);
                Trace("GRID_DISPLAY_DUAL " + Native.MonitorId(second) + " " + dual);
                SetDeviceEnabled(device, false);
                Wait(() =>
                {
                    Application.DoEvents();
                    var box = Application.OpenForms.OfType<GridSurface>().SingleOrDefault();
                    return Screen.AllScreens.Length == 1 && box != null && box.ScreenBounds == single;
                }, 45000, "停用副屏后没有回到单屏布局");
                SetDeviceEnabled(device, true);
                ApplyTopology(0x84);
                Wait(() =>
                {
                    Application.DoEvents();
                    var box = Application.OpenForms.OfType<GridSurface>().SingleOrDefault();
                    return Screen.AllScreens.Length >= 2 && box != null && box.ScreenBounds == dual;
                }, 45000, "重连副屏后没有恢复此前位置");
                app.Stop(true);
                var report = new
                {
                    passed = true, actualWindowsTopology = true, virtualSecondDisplay = true,
                    detachedAndRestored = true, mainDpi = nativeDpi, secondaryDpi = secondDpi,
                    mixedDpi = nativeDpi != secondDpi, single = single.ToString(), dual = dual.ToString()
                };
                LocalJson.Write(Path.Combine(GuestRoot, "display-qa.json"), report);
                Trace("GRID_DISPLAY_REPORT " + new JavaScriptSerializer().Serialize(report));
                return 0;
            }
        }
        catch (Exception error)
        {
            Trace("GRID_DISPLAY_REPORT " + new JavaScriptSerializer().Serialize(new { passed = false, error = error.Message }));
            return 1;
        }
        finally
        {
            if (device != null)
                try { SetDeviceEnabled(device, false); }
                catch (Exception error) { Trace("GRID_DISPLAY_CLEANUP " + error.Message); }
        }
    }

    private static void CreateDisplayDevice(string directory, out string device)
    {
        device = null;
        string inf = Path.Combine(directory, "MttVDD.inf");
        Require(File.Exists(inf), "缺少验收用签名显示驱动");
        string data = @"C:\VirtualDisplayDriver";
        Directory.CreateDirectory(data);
        File.Copy(Path.Combine(directory, "vdd_settings.xml"), Path.Combine(data, "vdd_settings.xml"), true);
        var display = new Guid("4d36e968-e325-11ce-bfc1-08002be10318");
        IntPtr list = SetupDiCreateDeviceInfoList(ref display, IntPtr.Zero);
        Require(list != new IntPtr(-1), "无法创建隔离测试设备列表 · " + Marshal.GetLastWin32Error());
        var row = new DeviceInfo { Size = Marshal.SizeOf(typeof(DeviceInfo)) };
        const string fixtureId = @"Root\MttVDD";
        try
        {
            Require(SetupDiCreateDeviceInfo(list, "DISPLAY", ref display, null, IntPtr.Zero, 1, ref row),
                "无法创建虚拟显示节点 · " + Marshal.GetLastWin32Error());
            byte[] ids = Encoding.Unicode.GetBytes(fixtureId + "\0\0");
            Require(SetupDiSetDeviceRegistryProperty(list, ref row, 1, ids, ids.Length),
                "无法写入测试设备硬件标识 · " + Marshal.GetLastWin32Error());
            Require(SetupDiCallClassInstaller(0x19, list, ref row), "无法登记测试设备 · " + Marshal.GetLastWin32Error());
            var identity = new StringBuilder(256);
            int required;
            Require(SetupDiGetDeviceInstanceId(list, ref row, identity, identity.Capacity, out required),
                "无法确定测试设备身份 · " + Marshal.GetLastWin32Error());
            device = identity.ToString();
        }
        finally { SetupDiDestroyDeviceInfoList(list); }
        Trace("GRID_DISPLAY_REGISTERED " + device);
        RunDisplayTool("/add-driver \"" + inf + "\" /install", 90000, "安装虚拟显示驱动");
        RunDisplayTool("/enum-devices /instanceid \"" + device + "\" /drivers", 12000, "查询测试显示设备");
    }

    private static void SetDeviceEnabled(string device, bool enabled)
    {
        Require(device.StartsWith(@"ROOT\DISPLAY\", StringComparison.OrdinalIgnoreCase), "只允许更改刚创建的访客显示节点");
        RunDisplayTool((enabled ? "/enable-device" : "/disable-device") + " \"" + device + "\"", 22000,
            "切换测试显示器");
    }

    private static string RunDisplayTool(string arguments, int timeout, string operation)
    {
        string executable = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "pnputil.exe");
        using (var process = Process.Start(new ProcessStartInfo(executable, arguments)
        {
            UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true
        }))
        {
            var output = process.StandardOutput.ReadToEndAsync();
            var error = process.StandardError.ReadToEndAsync();
            if (!process.WaitForExit(timeout))
            {
                process.Kill();
                throw new TimeoutException(operation + "超时");
            }
            string summary = string.Join(" / ", (output.Result + "\n" + error.Result)
                .Split(new[] { '\r', '\n' }, StringSplitOptions.RemoveEmptyEntries).Select(line => line.Trim()));
            Trace("GRID_DISPLAY_PNP " + process.ExitCode + " " + summary.Substring(0, Math.Min(1200, summary.Length)));
            Require(process.ExitCode == 0, operation + "失败 · " + process.ExitCode);
            return summary;
        }
    }

    private static int ApplyTopology(uint flags)
    {
        return SetDisplayConfig(0, IntPtr.Zero, 0, IntPtr.Zero, flags);
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct DeviceInfo { public int Size; public Guid Class; public uint Number; public IntPtr Reserved; }
    [DllImport("setupapi.dll", SetLastError = true)]
    private static extern IntPtr SetupDiCreateDeviceInfoList(ref Guid deviceClass, IntPtr parent);
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool SetupDiCreateDeviceInfo(IntPtr list, string name, ref Guid deviceClass, string description,
        IntPtr parent, uint flags, ref DeviceInfo info);
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool SetupDiSetDeviceRegistryProperty(IntPtr list, ref DeviceInfo info, uint property, byte[] data, int length);
    [DllImport("setupapi.dll", SetLastError = true)]
    private static extern bool SetupDiCallClassInstaller(uint function, IntPtr list, ref DeviceInfo info);
    [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern bool SetupDiGetDeviceInstanceId(IntPtr list, ref DeviceInfo info, StringBuilder name, int size, out int required);
    [DllImport("setupapi.dll")]
    private static extern bool SetupDiDestroyDeviceInfoList(IntPtr list);
    [DllImport("user32.dll")]
    private static extern int SetDisplayConfig(uint pathCount, IntPtr paths, uint modeCount, IntPtr modes, uint flags);
}
