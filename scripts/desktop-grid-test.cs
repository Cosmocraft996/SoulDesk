using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.IO.Ports;
using System.Linq;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;
using System.Windows.Automation;
using System.Windows.Forms;
using Microsoft.Win32;
using SoulDesk.DesktopGrid;

internal static partial class GridAcceptance
{
    private static readonly string GuestRoot = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "SoulDeskGridProductLab");
    private static string Clicked { get { return Path.Combine(GuestRoot, "clicked.txt"); } }
    private static readonly string[] Cases = { "normal-exit", "forced-end", "ui-hang", "explorer-restart" };

    [STAThread]
    private static int Main(string[] args)
    {
        try
        {
            if (args.Length == 2 && args[0] == "--model")
            {
                VerifyModel(Path.GetFullPath(args[1]));
                VerifyRendererModel(Path.Combine(Path.GetFullPath(args[1]), "renderer"));
                VerifyDesktopLabel();
                VerifyOrganization(Path.Combine(Path.GetFullPath(args[1]), "organization"));
                Console.WriteLine("PASS model, grouped references, backup, undo, topology, search, renderer ownership");
                return 0;
            }
            if (args.Length == 2 && args[0] == "--surface")
            {
                LaunchPrivateDesktop(Path.GetFullPath(args[1]));
                Console.WriteLine("PASS isolated grid surfaces, link drop, collapse, tabs, mapped folder, search, import");
                return 0;
            }
            if (args.Length == 3 && args[0] == "--package")
            {
                LaunchPrivateDesktop(Path.GetFullPath(args[2]), Path.GetFullPath(args[1]));
                Console.WriteLine("PASS packaged preview launch, sample grids, relaunch, closing and isolation");
                return 0;
            }
            if (args.Length == 3 && args[0] == "--winui-surface")
            {
                LaunchPrivateDesktop(Path.GetFullPath(args[2]), Path.GetFullPath(args[1]), winui: true);
                Console.WriteLine("PASS isolated WinUI window identity, placement, collapse modes and shutdown");
                return 0;
            }
            if (args.Length >= 3 && args[0] == "--surface-child")
            {
                var name = new StringBuilder(128);
                uint required;
                Require(GetUserObjectInformation(GetThreadDesktop(GetCurrentThreadId()), 2, name, 256, out required) &&
                    name.ToString() == args[2] && args[2].StartsWith("SoulDeskGridTest-"), "测试进程的桌面身份错误");
                if (args.Length == 5 && args[3] == "--winui")
                    VerifyWinUISurface(Path.GetFullPath(args[1]), Path.GetFullPath(args[4]));
                else if (args.Length == 4)
                    VerifyPreviewPackage(Path.GetFullPath(args[3]), Path.GetFullPath(args[1]));
                else
                    VerifySurfaces(Path.GetFullPath(args[1]));
                LocalJson.Write(Path.Combine(args[1], "surface-pass.json"), new { passed = true });
                return 0;
            }
            VerifyGuestIdentity();
            Directory.CreateDirectory(GuestRoot);
            if (args.Length == 1 && args[0] == "--clicked")
            {
                File.WriteAllText(Clicked, DateTime.UtcNow.ToString("o"));
                return 0;
            }
            Native.DpiAware();
            if (args.Length == 2 && args[0] == "--search-backends")
                return VerifySearchBackends(Path.GetFullPath(args[1]));
            if (args.Length == 2 && args[0] == "--host-preview")
                return VerifyHostPreview(Path.GetFullPath(args[1]));
            if (args.Length == 2 && args[0] == "--installed-host")
                return VerifyInstalledHost(Path.GetFullPath(args[1]));
            if (args.Length == 3 && args[0] == "--installed-host-mixed")
                return VerifyInstalledMixedDisplayHost(Path.GetFullPath(args[1]), Path.GetFullPath(args[2]));
            if (args.Length == 2 && args[0] == "--display-topology")
                return VerifyDisplayTopology(Path.GetFullPath(args[1]));
            if (args.Length == 2 && args[0] == "--display-existing")
                return VerifyDisplayTopology(null, args[1]);
            string displayDriver = null;
            bool mixedDisplay = args.Length == 2 && args[0] == "--renderer-display-mixed";
            if (args.Length == 2 && (args[0] == "--renderer-display" || mixedDisplay))
            {
                displayDriver = Path.GetFullPath(args[1]);
                args = new[] { "--renderer-recovery", "--case", mixedDisplay ? "renderer-display-mixed" : "renderer-display" };
            }
            Trace("GRID_GUEST_READY");
            string executable = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "desktop-grid.exe");
            Require(File.Exists(executable), "桌面格子产品二进制不存在");
            bool externalPointer = args.Contains("--external-pointer");
            args = args.Where(arg => arg != "--external-pointer").ToArray();
            string renderer = null;
            if (args.Contains("--renderer-recovery"))
            {
                renderer = Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "desktop-grid-winui", "SoulDeskGridWinUI.exe");
                Require(File.Exists(renderer), "缺少已构建的 WinUI 格子");
                args = args.Where(arg => arg != "--renderer-recovery").ToArray();
            }
            string document, shortcut;
            PrepareDesktopFixtures(out document, out shortcut);
            Wait(() => ReadIcons(document, shortcut) != null, 45000, "访客测试项目不可见");
            string fingerprint = Fingerprint(document, shortcut);
            var results = new List<object>();
            bool passed = true;
            string[] available = renderer == null ? Cases : Cases.Concat(new[]
                { "renderer-end", "renderer-hang", "renderer-tabs", "renderer-placement", "renderer-compact-placement",
                    "renderer-group", "renderer-advanced", "renderer-shortcut-edge" }).ToArray();
            if (displayDriver != null)
                available = available.Concat(new[] { mixedDisplay ? "renderer-display-mixed" : "renderer-display" }).ToArray();
            bool casesSpecified = args.Length >= 2 && args[0] == "--case";
            string[] selected = casesSpecified
                ? available.Where(scenario => args.Skip(1).Contains(scenario)).ToArray() : available;
            Require(!casesSpecified || selected.Length == args.Length - 1, "未知或重复的故障用例");
            Require(!externalPointer || renderer != null && selected.SequenceEqual(new[] { "renderer-placement" }),
                "外部指针仅用于隔离格子的单项拖动测试");
            string digest;
            string rendererDigest = null;
            using (var hash = SHA256.Create())
            {
                digest = BitConverter.ToString(hash.ComputeHash(File.ReadAllBytes(executable))).Replace("-", "");
                if (renderer != null)
                    rendererDigest = BitConverter.ToString(hash.ComputeHash(
                        File.ReadAllBytes(Path.ChangeExtension(renderer, ".dll")))).Replace("-", "");
            }
            var serializer = new JavaScriptSerializer();
            foreach (string scenario in selected)
            {
                Trace("GRID_CASE_BEGIN " + scenario);
                try
                {
                    bool casePassed;
                    results.Add(Scenario(scenario, executable, document, shortcut, fingerprint, renderer, externalPointer,
                        displayDriver, out casePassed));
                    if (casePassed)
                        Trace("GRID_CASE_PASS " + scenario);
                    else
                    {
                        passed = false;
                        Trace("GRID_CASE_FAIL " + scenario + " 未分组图标的位置或点击检查失败");
                    }
                }
                catch (Exception error)
                {
                    passed = false;
                    results.Add(new { scenario, passed = false, error = error.ToString() });
                    Trace("GRID_CASE_FAIL " + scenario + " " + error.Message);
                }
                Trace("GRID_CASE_REPORT " + serializer.Serialize(new { productSha256 = digest, rendererSha256 = rendererDigest,
                    result = results.Last() }));
            }
            var report = new { passed, actualExplorer = true, isolatedGuest = true, productSha256 = digest,
                rendererSha256 = rendererDigest, results };
            LocalJson.Write(Path.Combine(GuestRoot, "product-report.json"), report);
            Trace("GRID_REPORT " + serializer.Serialize(report));
            return passed ? 0 : 1;
        }
        catch (Exception error)
        {
            Console.Error.WriteLine(error.ToString());
            if (args.Length >= 3 && args[0] == "--surface-child")
            {
                Directory.CreateDirectory(args[1]);
                File.WriteAllText(Path.Combine(args[1], "surface-error.txt"), error.ToString());
            }
            if (Environment.MachineName == "SD-ICON-VM") Trace("GRID_ERROR " + error);
            return 1;
        }
    }

    private static void VerifyGuestIdentity()
    {
        Require(Environment.MachineName == "SD-ICON-VM" && Environment.UserName == "ExplorerLab" &&
            (string)Registry.GetValue(@"HKEY_LOCAL_MACHINE\HARDWARE\DESCRIPTION\System\BIOS", "SystemProductName", null) ==
            "SoulDesk-Icon-Lab" && File.Exists(@"C:\SoulDeskLab\SOULDESK.LAB") &&
            File.ReadAllText(@"C:\SoulDeskLab\SOULDESK.LAB").Trim() == "SoulDeskExplorerIsolationLab-v1",
            "故障注入仅限之前配置的隔离访客");
    }

    private static int VerifySearchBackends(string tools)
    {
        string directory = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.MyDocuments), "SoulDeskGridSearchQA");
        Directory.CreateDirectory(directory);
        string fixture = Path.Combine(directory, "SoulDeskIndexFixture_检索.txt");
        File.WriteAllText(fixture, "isolated index probe");
        string program = Path.Combine(tools, "Everything.exe");
        string es = Path.Combine(tools, "es.exe");
        string ini = Path.Combine(tools, "Everything.ini");
        File.WriteAllLines(ini, new[] { "[Everything]", "app_data=0", "run_as_admin=0", "run_in_background=1",
            "show_tray_icon=0", "check_for_updates_on_startup=0", "folders=" + directory,
            "auto_include_fixed_volumes=0", "auto_include_removable_volumes=0", "auto_include_remote_volumes=0" },
            Encoding.ASCII);

        bool windows = false, everything = false, fallback = false;
        string windowsNote = null, everythingNote = null;
        string cliDiagnostic = null;
        var watch = Stopwatch.StartNew();
        using (var indexer = Process.Start(new ProcessStartInfo(program, "-startup")
        {
            WorkingDirectory = tools, UseShellExecute = false, CreateNoWindow = true
        }))
        {
            while (!(windows && everything) && watch.ElapsedMilliseconds < 115000)
            {
                if (!windows)
                {
                    var rows = SearchIndex.Find("SoulDeskIndexFixture_", new string[0], false, true, null);
                    windows = rows.Items.Any(item => References.Equal(item.Path, fixture));
                    windowsNote = rows.Note;
                }
                if (!everything)
                {
                    var rows = SearchIndex.Find("检索", new string[0], false, false, es);
                    everything = rows.Items.Any(item => References.Equal(item.Path, fixture));
                    everythingNote = rows.Note;
                }
                if (!(windows && everything)) Thread.Sleep(1400);
            }
            if (!everything)
                cliDiagnostic = ReadEverythingDiagnostic(es, indexer);
            using (var stop = Start(program, "-exit")) stop.WaitForExit(3000);
            var local = SearchIndex.Find("检索", new[] { fixture }, false, false, Path.Combine(tools, "missing-es.exe"));
            fallback = local.Note != null && local.Items.Any(item => References.Equal(item.Path, fixture));
        }
        string service;
        try { using (var search = new System.ServiceProcess.ServiceController("WSearch")) service = search.Status.ToString(); }
        catch (InvalidOperationException) { service = "Unavailable"; }
        var report = new { windowsIndex = windows, everythingCli = everything, localFallback = fallback, windowsNote, everythingNote, cliDiagnostic,
            windowsSearchService = service, elapsedMs = watch.ElapsedMilliseconds };
        LocalJson.Write(Path.Combine(GuestRoot, "search-qa.json"), report);
        Trace("GRID_SEARCH_REPORT " + new JavaScriptSerializer().Serialize(report));
        return windows && everything && fallback ? 0 : 1;
    }

    private static string ReadEverythingDiagnostic(string es, Process indexer)
    {
        string state = indexer.HasExited ? "indexer exit=" + indexer.ExitCode : "indexer alive";
        using (var process = Process.Start(new ProcessStartInfo(es, "-timeout 9000 -n 3 SoulDeskIndexFixture_")
        {
            UseShellExecute = false, CreateNoWindow = true, RedirectStandardOutput = true, RedirectStandardError = true
        }))
        {
            var text = process.StandardOutput.ReadToEndAsync();
            var error = process.StandardError.ReadToEndAsync();
            if (!process.WaitForExit(20000))
            {
                process.Kill();
                return state + "; ES timed out";
            }
            string message = text.Result.Trim() + " " + error.Result.Trim();
            return state + "; ES exit=" + process.ExitCode + " · " + message.Substring(0, Math.Min(240, message.Length));
        }
    }

    private static int VerifyHostPreview(string executable)
    {
        string root = Path.Combine(GuestRoot, "desktop-integration-grid-" + Guid.NewGuid().ToString("N"));
        var paths = Path.Combine(root, "desktop-grid");
        Directory.CreateDirectory(Path.Combine(paths, "fixtures"));
        File.WriteAllText(Path.Combine(root, "isolated-test-data.marker"), "SoulDesk grid preview laboratory");
        File.WriteAllText(Path.Combine(paths, "fixtures", "验收文档.txt"), "isolated test document");
        using (var blocked = Start(executable, ""))
            Require(blocked.WaitForExit(12000) && blocked.ExitCode == 2, "体验版没有拒绝普通启动");

        var shell = ShellDesktop.Find();
        Require(shell != null && Native.IsWindowVisible(shell.View.Hwnd), "隔离访客的原生桌面未就绪");
        string childFingerprint = null;
        bool settings = false, tray = false, closedFromTitle = false;
        using (var preview = Start(executable, "--desktop-grid-preview \"" + root + "\" --show"))
        {
            try
            {
                Trace("GRID_HOST_PREVIEW_BEGIN");
                var window = WaitForWindow(preview.Id, "SoulDesk", 65000);
                var setup = AutomationElement.FromHandle(window);
                ClickNamed(setup, "设置");
                ClickNamed(setup, "打开格子管理");
                int gridOwner;
                IntPtr gridWindow = WaitForGridManager(root, out gridOwner);
                Require(Native.IsWindowVisible(shell.View.Hwnd) && new Recovery(paths).Current() == null,
                    "体验入口发起了真实 Explorer 接管");
                using (var hash = SHA256.Create())
                using (var child = Process.GetProcessById(gridOwner))
                {
                    childFingerprint = BitConverter.ToString(hash.ComputeHash(File.ReadAllBytes(child.MainModule.FileName))).Replace("-", "");
                    settings = true;
                    Trace("GRID_HOST_SETTINGS_PASS");
                    PostMessage(gridWindow, 0x0112, new IntPtr(0xF060), IntPtr.Zero);
                    closedFromTitle = child.WaitForExit(45000);
                }
                Require(closedFromTitle, "格子窗口关闭后未退出");
                Trace("GRID_HOST_CLOSE_PASS");
                ClickTrayItem();
                gridWindow = WaitForGridManager(root, out gridOwner);
                tray = true;
                Trace("GRID_HOST_TRAY_PASS");
                PostMessage(gridWindow, 0x0112, new IntPtr(0xF060), IntPtr.Zero);
            }
            catch (Exception error)
            {
                Trace("GRID_HOST_PREVIEW_FAIL " + error.Message);
            }
            finally
            {
                // 只结束本次由验收程序启动的访客体验进程。
                if (!preview.HasExited) preview.Kill();
            }
        }
        var report = new { settings, tray, closedFromTitle, childFingerprint, isolated = true,
            originalExplorerVisible = Native.IsWindowVisible(shell.View.Hwnd), explorerLeaseCreated = new Recovery(paths).Current() != null };
        LocalJson.Write(Path.Combine(GuestRoot, "host-preview-qa.json"), report);
        Trace("GRID_HOST_REPORT " + new JavaScriptSerializer().Serialize(report));
        return settings && tray ? 0 : 1;
    }

    private static int VerifyInstalledHost(string installer)
    {
        string data = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "local.souldesk.grid.qa");
        string root = Path.Combine(data, "desktop-grid");
        var recovery = new Recovery(root);
        var previous = recovery.Current();
        Require(previous == null || recovery.Signaled("done", previous.Token), "隔离测试盘仍有未结束的格子会话");
        string renderer;
        string executable = InstallGridHost(installer, out renderer);
        string document, shortcut;
        PrepareDesktopFixtures(out document, out shortcut);
        var store = new GridStore(root);
        store.Change(state =>
        {
            var box = state.Boxes.Count == 0 ? state.Add("安装包格子验收", null) : state.Boxes.Single();
            Require(box.Current().Name == "安装包格子验收" || box.Current().Name == "安装包导入验收",
                "验收目录中的格子来源不匹配");
            box.Current().Name = "安装包格子验收";
            box.Collapsed = true;
            state.Release(new[] { document, shortcut });
        });
        string original = Fingerprint(document, shortcut);
        List<DesktopItem> baseline = null;
        Wait(() => (baseline = ReadIcons(document, shortcut)) != null, 20000, "安装后无法读取测试桌面坐标");
        using (var host = Start(executable, "--show"))
        {
            try
            {
                var main = WaitForWindow(host.Id, "SoulDesk · 组件管理", 65000);
                ClickNamed(AutomationElement.FromHandle(main), "设置");
                ClickNamed(AutomationElement.FromHandle(main), "打开格子管理");
                int productPid;
                IntPtr manager = WaitForGridManager(data, out productPid);
                Trace("GRID_INSTALLED_HOST_MANAGER_READY");
                var startup = Stopwatch.StartNew();
                EnableGrid(manager);
                GridLease lease = null;
                Wait(() => (lease = recovery.Current()) != null && lease.Owner.Pid == productPid &&
                    (previous == null || lease.Token != previous.Token) && recovery.Signaled("ready", lease.Token) &&
                    ShellDesktop.Matches(lease.Desktop) && !Native.IsWindowVisible(lease.Desktop.Hwnd), 85000,
                    "安装版未完成桌面图标接管", () => ManagerContents(manager) + " " + Error(root));
                IntPtr grid = VerifyActiveRenderer(root, renderer, lease);
                long startupMs = startup.ElapsedMilliseconds;
                Trace("GRID_INSTALLED_HOST_ENABLED");
                PostMessage(main, 0x10, IntPtr.Zero, IntPtr.Zero);
                Point position = CenterIcon(baseline[1].NativeBounds);
                Wait(() => HitPid(position) == productPid, 8000, "安装版未保留未分组图标的鼠标位置");
                DoubleClickAndCheck(position, "安装版接管后");
                VerifyRendererGestures(root, productPid, lease.Renderer.Pid, grid,
                    document, CenterIcon(baseline[0].NativeBounds), shortcut, position);
                VerifyInstalledGridSearch(productPid, shortcut);
                VerifyInstalledGridBackup(root, manager, document, shortcut);
                var restoring = Stopwatch.StartNew();
                ClickTrayItem("恢复 Windows 桌面图标");
                List<DesktopItem> restored = null;
                Wait(() => (restored = ReadIcons(document, shortcut)) != null && recovery.Signaled("done", lease.Token),
                    15000, "安装版托盘操作后原生桌面未恢复", () => Error(root));
                long recoveryMs = restoring.ElapsedMilliseconds;
                Require(CenterIcon(restored[0].NativeBounds) == CenterIcon(baseline[0].NativeBounds) &&
                    CenterIcon(restored[1].NativeBounds) == position && Fingerprint(document, shortcut) == original,
                    "安装版拖放改变了测试文件的坐标、内容或属性");
                Wait(() => HitPid(position) == lease.Desktop.Pid, 7000, "恢复后实际鼠标未命中 Explorer");
                DoubleClickAndCheck(position, "安装版还原原生图标后");
                ClickTrayItem("退出 SoulDesk");
                Require(host.WaitForExit(25000) && host.ExitCode == 0, "安装版未从托盘正常退出");
                var report = new
                {
                    passed = true, isolatedGuest = true, installed = true, actualExplorer = true,
                    settingsOpenedGrid = true, trayRestoredIcons = true, trayExitedHost = true,
                    mouseCollapseAndFileAndShortcutDrop = true, unchangedPathsAndCoordinates = true,
                    globalShortcutSearchOpenedLink = true, localExportAndImport = true,
                    startupMs, recoveryMs,
                    setupSha256 = Digest(installer), rendererSha256 = Digest(Path.ChangeExtension(renderer, ".dll"))
                };
                Trace("GRID_INSTALLED_HOST_REPORT " + new JavaScriptSerializer().Serialize(report));
                return 0;
            }
            finally
            {
                try { recovery.RequestRestore(); }
                finally { if (!host.HasExited) host.Kill(); }
            }
        }
    }

    private static string InstallGridHost(string installer, out string renderer)
    {
        VerifyGuestIdentity();
        Require(installer.StartsWith(@"C:\SoulDeskLab\GridQAHost\", StringComparison.OrdinalIgnoreCase) &&
            File.Exists(installer), "安装验收仅接受访客的专用测试安装包");
        string directory = Path.Combine(Path.GetDirectoryName(installer), "Installed");
        Trace("GRID_INSTALLED_HOST_BEGIN");
        using (var setup = Start(installer, "/S /D=" + directory))
            Require(setup.WaitForExit(180000) && setup.ExitCode == 0, "测试安装器未完成");
        string executable = Path.Combine(directory, "souldesk.exe");
        renderer = Path.Combine(directory, "desktop-grid-winui", "SoulDeskGridWinUI.exe");
        Require(File.Exists(executable) && File.Exists(renderer) &&
            File.Exists(Path.Combine(directory, "desktop-grid-winui", "source", "LICENSE")) &&
            File.Exists(Path.Combine(directory, "desktop-grid-winui", "source", "deskbox-upstream.zip")) &&
            File.Exists(Path.Combine(directory, "desktop-grid-winui", "source", "deskbox.patch")), "安装包缺少程序或对应来源");
        Trace("GRID_INSTALLED_HOST_RESOURCES_READY");
        return executable;
    }

    private static string Digest(string path)
    {
        using (var hash = SHA256.Create())
        using (var stream = File.OpenRead(path))
            return BitConverter.ToString(hash.ComputeHash(stream)).Replace("-", "");
    }

    private static void PrepareDesktopFixtures(out string document, out string shortcut)
    {
        string desktop = Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory);
        Require(desktop.StartsWith(@"C:\Users\ExplorerLab\", StringComparison.OrdinalIgnoreCase), "需要访客账户的桌面");
        document = Path.Combine(desktop, "SoulDeskTestGridA.txt");
        shortcut = Path.Combine(desktop, "SoulDeskTestGridB.lnk");
        File.WriteAllText(document, "SoulDesk isolated product test");
        dynamic shell = Activator.CreateInstance(Type.GetTypeFromProgID("WScript.Shell", true));
        dynamic link = shell.CreateShortcut(shortcut);
        link.TargetPath = System.Reflection.Assembly.GetExecutingAssembly().Location;
        link.Arguments = "--clicked";
        link.WindowStyle = 7;
        link.Save();
        Marshal.FinalReleaseComObject(link);
        Marshal.FinalReleaseComObject(shell);
        LocalJson.Write(Path.Combine(GuestRoot, "fixture.json"), new { document, shortcut });
    }

    private static void VerifyInstalledGridSearch(int productPid, string shortcut)
    {
        keybd_event(0x11, 0, 0, UIntPtr.Zero);
        keybd_event(0x12, 0, 0, UIntPtr.Zero);
        try { PressVirtualKey(0x46); }
        finally
        {
            keybd_event(0x12, 0, 2, UIntPtr.Zero);
            keybd_event(0x11, 0, 2, UIntPtr.Zero);
        }
        var search = WaitForWindow(productPid, "SoulDesk · 全局搜索", 10000);
        var surface = AutomationElement.FromHandle(search);
        var query = Named(surface, "搜索文件或快捷方式");
        object input = null;
        Require(query != null && query.TryGetCurrentPattern(ValuePattern.Pattern, out input), "全局搜索输入框不可用");
        ((ValuePattern)input).SetValue(Path.GetFileNameWithoutExtension(shortcut));
        var itemType = new PropertyCondition(AutomationElement.ControlTypeProperty, System.Windows.Automation.ControlType.ListItem);
        Wait(() => surface.FindAll(TreeScope.Descendants, itemType).Cast<AutomationElement>()
            .Any(item => item.Current.Name.Contains(Path.GetFileNameWithoutExtension(shortcut))),
            20000, "安装版全局搜索未找到原地分组的快捷方式");
        if (File.Exists(Clicked)) File.Delete(Clicked);
        query.SetFocus();
        PressVirtualKey(0x0D);
        Wait(() => File.Exists(Clicked), 10000, "全局搜索结果未启动原路径快捷方式");
        PostMessage(search, 0x10, IntPtr.Zero, IntPtr.Zero);
        Wait(() => !Native.IsWindowVisible(search), 6000, "全局搜索窗口没有关闭");
        Trace("GRID_INSTALLED_HOST_SEARCH_PASS");
    }

    private static void VerifyInstalledGridBackup(string root, IntPtr manager, string document, string shortcut)
    {
        var surface = AutomationElement.FromHandle(manager);
        var basePath = Path.Combine(GuestRoot, "installed-layout-" + Guid.NewGuid().ToString("N"));
        var export = basePath + ".json";
        TransferInstalledGridLayout(manager, surface, "导出布局", export);
        Wait(() => File.Exists(export), 10000, "安装版没有导出本地 JSON", () => ManagerContents(manager));
        var layout = GridStore.ReadLayout(export);
        var tab = layout.Boxes.Single().Current();
        Require(tab.Items.Contains(document) && tab.Items.Contains(shortcut), "导出的布局缺少原路径引用");
        tab.Name = "安装包导入验收";
        var import = basePath + "-import.json";
        LocalJson.Write(import, layout);
        TransferInstalledGridLayout(manager, surface, "导入布局", import);
        Wait(() => new GridStore(root).State.Boxes.Single().Current().Name == tab.Name,
            10000, "安装版未从本地 JSON 导入布局");
        PostMessage(manager, 0x10, IntPtr.Zero, IntPtr.Zero);
        Trace("GRID_INSTALLED_HOST_BACKUP_PASS");
    }

    private static void TransferInstalledGridLayout(IntPtr manager, AutomationElement surface, string action, string path)
    {
        var button = Named(surface, action);
        Require(button != null, "无法打开格子布局对话框 " + action);
        ClickAt(button, false);
        IntPtr picker = IntPtr.Zero;
        Wait(() =>
        {
            Native.EnumWindows((window, _) =>
            {
                var type = new StringBuilder(40);
                if (Native.GetWindow(window, 4) == manager && Native.IsWindowVisible(window) &&
                    GetClassName(window, type, type.Capacity) != 0 && type.ToString() == "#32770")
                    picker = window;
                return picker == IntPtr.Zero;
            }, IntPtr.Zero);
            return picker != IntPtr.Zero;
        }, 10000, "系统文件对话框未显示 " + action);
        Wait(() => GetForegroundWindow() == picker && Native.Responsive(WindowIdentity.Read(picker)),
            10000, "系统文件对话框未获得焦点");
        Trace("GRID_INSTALLED_FILE_PICKER " + Title(picker, true));
        // 隔离访客使用英文 Windows，Alt+N 聚焦系统对话框的文件名。
        SendKeys.SendWait("%n");
        SendKeys.SendWait("^a");
        SendKeys.SendWait(path);
        var field = AutomationElement.FocusedElement;
        object value = null;
        Require(GetForegroundWindow() == picker && field != null && field.TryGetCurrentPattern(ValuePattern.Pattern, out value) &&
            ((ValuePattern)value).Current.Value == path, "系统文件对话框未收到预期路径");
        PressVirtualKey(0x0D);
        Wait(() => !Native.IsWindowVisible(picker), 7000, "系统文件对话框未确认 " + action);
    }

    private static IntPtr WaitForWindow(int process, string title, int timeout)
    {
        IntPtr found = IntPtr.Zero;
        Wait(() =>
        {
            Native.EnumWindows((window, _) =>
            {
                uint pid;
                Native.GetWindowThreadProcessId(window, out pid);
                if (pid == process && Native.IsWindowVisible(window) && Title(window).StartsWith(title, StringComparison.Ordinal))
                    found = window;
                return found == IntPtr.Zero;
            }, IntPtr.Zero);
            return found != IntPtr.Zero;
        }, timeout, "未看到验收窗口 " + title);
        return found;
    }

    private static IntPtr WaitForGridManager(string root, out int owner)
    {
        string bin = Path.Combine(root, "desktop-grid", "bin") + "\\";
        int found = 0;
        IntPtr manager = IntPtr.Zero;
        Wait(() =>
        {
            Native.EnumWindows((window, _) =>
            {
                if (!Native.IsWindowVisible(window) ||
                    !Title(window).StartsWith("SoulDesk · 桌面格子", StringComparison.Ordinal))
                    return true;
                uint pid;
                Native.GetWindowThreadProcessId(window, out pid);
                try
                {
                    using (var process = Process.GetProcessById((int)pid))
                    {
                        if (process.ProcessName.Length != 64 ||
                            !process.MainModule.FileName.StartsWith(bin, StringComparison.OrdinalIgnoreCase))
                            return true;
                        found = process.Id;
                        manager = window;
                        return false;
                    }
                }
                catch (ArgumentException) { }
                catch (InvalidOperationException) { }
                catch (System.ComponentModel.Win32Exception) { }
                return true;
            }, IntPtr.Zero);
            return manager != IntPtr.Zero;
        }, 52000, "未发现对应格子进程的可见管理窗口",
            () => Error(Path.Combine(root, "desktop-grid")));
        owner = found;
        return manager;
    }

    private static AutomationElement Named(AutomationElement element, string name)
    {
        var exact = element.FindFirst(TreeScope.Descendants, new PropertyCondition(AutomationElement.NameProperty, name));
        if (exact != null || name != "设置")
            return exact;
        return element.FindAll(TreeScope.Descendants,
            new PropertyCondition(AutomationElement.ControlTypeProperty, System.Windows.Automation.ControlType.Button))
            .Cast<AutomationElement>().FirstOrDefault(item => item.Current.Name.Trim().EndsWith(name, StringComparison.Ordinal));
    }

    private static void ClickNamed(AutomationElement root, string text)
    {
        AutomationElement button = null;
        Wait(() => (button = Named(root, text)) != null, 20000, "未看到控件 " + text);
        InvokeOrClick(button);
    }

    private static void ClickAt(AutomationElement element, bool rightButton)
    {
        var bounds = element.Current.BoundingRectangle;
        Require(!bounds.IsEmpty, "控件不可点击");
        SetCursorPos((int)(bounds.Left + bounds.Width / 2), (int)(bounds.Top + bounds.Height / 2));
        mouse_event(rightButton ? 8u : 2u, 0, 0, 0, UIntPtr.Zero);
        mouse_event(rightButton ? 16u : 4u, 0, 0, 0, UIntPtr.Zero);
    }

    private static void ClickTrayItem(string name = "桌面格子")
    {
        var handle = Native.FindWindowEx(IntPtr.Zero, IntPtr.Zero, "Shell_TrayWnd", null);
        Require(handle != IntPtr.Zero, "找不到访客任务栏");
        var taskbar = AutomationElement.FromHandle(handle);
        var icon = Named(taskbar, "SoulDesk");
        if (icon == null || icon.Current.IsOffscreen)
        {
            var overflow = Named(taskbar, "Show hidden icons") ?? Named(taskbar, "显示隐藏的图标");
            if (overflow != null)
                ClickAt(overflow, false);
            else
            {
                // Windows 的 Win+B 会把焦点移到通知区域。
                keybd_event(0x5B, 0, 0, UIntPtr.Zero);
                PressVirtualKey(0x42);
                keybd_event(0x5B, 0, 2, UIntPtr.Zero);
                Thread.Sleep(750);
                PressVirtualKey(0x20);
            }
            var iconButton = new AndCondition(new PropertyCondition(AutomationElement.NameProperty, "SoulDesk"),
                new PropertyCondition(AutomationElement.ControlTypeProperty, System.Windows.Automation.ControlType.Button));
            Wait(() => (icon = FindTrayControl(iconButton)) != null && !icon.Current.IsOffscreen,
                12000, "找不到 SoulDesk 托盘图标");
        }
        ClickAt(icon, true);
        var menuItem = new AndCondition(new PropertyCondition(AutomationElement.NameProperty, name),
            new PropertyCondition(AutomationElement.ControlTypeProperty, System.Windows.Automation.ControlType.MenuItem));
        AutomationElement entry = null;
        Wait(() => (entry = FindTrayControl(menuItem)) != null,
            15000, "未看到托盘菜单项 " + name);
        object invoke;
        if (entry.TryGetCurrentPattern(InvokePattern.Pattern, out invoke))
            ((InvokePattern)invoke).Invoke();
        else ClickAt(entry, false);
    }

    private static AutomationElement FindTrayControl(Condition condition)
    {
        try { return AutomationElement.RootElement.FindFirst(TreeScope.Descendants, condition); }
        catch (ElementNotAvailableException) { return null; }
        catch (COMException error)
        {
            if (error.ErrorCode != unchecked((int)0x8000FFFF)) throw;
            return null;
        }
    }

    private static void PressVirtualKey(byte key)
    {
        keybd_event(key, 0, 0, UIntPtr.Zero);
        keybd_event(key, 0, 2, UIntPtr.Zero);
    }

    private static object Scenario(string scenario, string executable, string document, string shortcut, string fingerprint,
        string renderer, bool externalPointer, string displayDriver, out bool casePassed)
    {
        string root = Path.Combine(GuestRoot, scenario + "-" + Guid.NewGuid().ToString("N"));
        var store = new GridStore(root);
        bool gestures = renderer != null && scenario == "normal-exit";
        bool shortcutEdge = scenario == "renderer-shortcut-edge";
        bool tabs = scenario == "renderer-tabs";
        bool compactPlacement = scenario == "renderer-compact-placement";
        bool placement = scenario == "renderer-placement" || compactPlacement;
        bool grouping = scenario == "renderer-group";
        bool advanced = scenario == "renderer-advanced";
        string moveFixture = advanced ? Path.Combine(Path.GetDirectoryName(document),
            "SoulDeskMove-" + Guid.NewGuid().ToString("N") + ".txt") : null;
        bool mixedDisplay = scenario == "renderer-display-mixed";
        store.Change(state =>
        {
            var box = state.Add("产品接管测试", null);
            box.Collapsed = gestures || compactPlacement || shortcutEdge;
            if ((placement && !compactPlacement) || grouping)
                box.SetCollapseMode(GridBox.Expanded);
            else if (placement || displayDriver != null)
                box.HoverExpand = compactPlacement;
            if (!gestures)
                state.Attach(box.Current().Id, new[] { document });
            if (tabs)
            {
                box.HoverExpand = false;
                box.Tabs.Add(new GridTab { Name = "标签归档" });
            }
            if (grouping)
            {
                var target = state.Add("验收目标格子", null);
                target.SetCollapseMode(GridBox.Expanded);
                state.Place("qa-initial", target.Id, new GridPosition
                {
                    Monitor = Native.MonitorId(Screen.PrimaryScreen), X = .32, Y = .67
                });
            }
            if (advanced)
            {
                box.SetCollapseMode(GridBox.Expanded);
                box.BoundMonitor = "@primary";
                box.Current().Artwork["TitleIconImage"] = "title-icon.png";
                box.Current().Artwork["BackgroundImage"] = "background.png";
                box.Current().Rules = Path.GetFileName(moveFixture);
                string extra = Path.Combine(root, "extra.txt");
                File.WriteAllText(extra, "second stack member");
                state.Attach(box.Current().Id, new[] { document, extra });
                var stackName = "Manual:" + Guid.NewGuid().ToString("N");
                var json = new JavaScriptSerializer();
                box.Current().StackSettings["FileStackMemberOverrides"] = json.Serialize(
                    new Dictionary<string, string[]> { { stackName, new[] { document, extra } } });
                box.Current().StackSettings["FileStackNameOverrides"] = json.Serialize(
                    new Dictionary<string, string> { { stackName, "验收自定义堆栈" } });
                string asset = Path.Combine(root, "artwork", "widget-assets", box.Current().Id);
                Directory.CreateDirectory(asset);
                using (var bitmap = new Bitmap(24, 24))
                using (var graphics = Graphics.FromImage(bitmap))
                {
                    graphics.Clear(Color.DeepSkyBlue);
                    bitmap.Save(Path.Combine(asset, "title-icon.png"), System.Drawing.Imaging.ImageFormat.Png);
                    bitmap.Save(Path.Combine(asset, "background.png"), System.Drawing.Imaging.ImageFormat.Png);
                }
                var target = state.Add("联动验收格子", null);
                target.SetCollapseMode(GridBox.Expanded);
                target.BoundMonitor = "@primary";
                string displayProbe = Path.Combine(root, "display-preference-proof.log");
                File.WriteAllText(displayProbe, "扩展名显示验收");
                state.Attach(target.Current().Id, new[] { displayProbe });
                state.CapsuleBar = "Horizontal";
                state.Place("qa-initial", target.Id, new GridPosition {
                    Monitor = Native.MonitorId(Screen.PrimaryScreen), X = .28, Y = .65
                });
            }
        });
        var shell = ShellDesktop.Find();
        var baseline = ReadIcons(document, shortcut);
        Require(baseline != null, "接管前未捕获测试图标");
        var iconErrors = new List<string>();

        string renderArgument = renderer == null ? "" : " --renderer-exe \"" + renderer + "\"";
        using (var host = Process.Start(new ProcessStartInfo("cmd.exe", "/d /c ping -n 360 127.0.0.1 >nul")
            { UseShellExecute = false, CreateNoWindow = true }))
        using (var product = Start(executable, "--open \"" + root + "\" --parent " + host.Id + " 0" + renderArgument))
        {
            var recovery = new Recovery(root);
            try
            {
                IntPtr managerHandle = IntPtr.Zero;
                Wait(() => (managerHandle = FindProductWindow(product.Id)) != IntPtr.Zero,
                    55000, "产品设置窗口没有出现", () => Error(root));
                var startup = Stopwatch.StartNew();
                EnableGrid(managerHandle);
                GridLease lease = null;
                Wait(() => (lease = recovery.Current()) != null && recovery.Signaled("ready", lease.Token) &&
                    !Native.IsWindowVisible(lease.Desktop.Hwnd), 85000,
                    "产品未接管桌面图标", () => ManagerContents(managerHandle) + " " + Error(root));
                if (!advanced) PostMessage(managerHandle, 0x10, IntPtr.Zero, IntPtr.Zero);
                IntPtr rendererWindow = renderer == null ? IntPtr.Zero :
                    VerifyActiveRenderer(root, renderer, lease, grouping || advanced ? "产品接管测试" : null,
                        grouping || advanced ? 2 : 1);
                long startupMs = startup.ElapsedMilliseconds;

                Point position = CenterIcon(baseline[1].NativeBounds);
                Wait(() => HitPid(position) == product.Id, 7000, "未分组图标原坐标没有命中 SoulDesk");
                DoubleClickAndCheck(position, "首次接管");
                Require(Fingerprint(document, shortcut) == fingerprint, "接管时路径、内容或属性发生变化");
                Trace("GRID_ACTIVE " + scenario);
                if (gestures)
                {
                    VerifyRendererGestures(root, product.Id, lease.Renderer.Pid, rendererWindow,
                        document, CenterIcon(baseline[0].NativeBounds), shortcut, position);
                    Require(Fingerprint(document, shortcut) == fingerprint, "拖放改变了原始文件或快捷方式");
                }
                if (shortcutEdge)
                {
                    WindowRect capsule;
                    Require(GetWindowRect(rendererWindow, out capsule) && capsule.Bottom - capsule.Top < 85,
                        "快捷方式拖放前格子没有收起");
                    var header = new Point((capsule.Left + capsule.Right) / 2, (capsule.Top + capsule.Bottom) / 2);
                    Require(HitPid(header) == lease.Renderer.Pid, "鼠标无法命中当前格子的胶囊");
                    DragToRenderer(root, product.Id, rendererWindow, position, shortcut, header, capsule.Bottom - capsule.Top, true);
                    Trace("GRID_SHORTCUT_EDGE_PASS");
                }
                if (tabs)
                    VerifyRendererTabs(root, lease, rendererWindow, renderer);
                if (placement)
                {
                    var looseIcon = WindowFromPoint(position);
                    VerifyRendererPlacement(root, lease, rendererWindow, renderer, externalPointer, compactPlacement);
                    Require(WindowFromPoint(position) == looseIcon, "拖动格子后重建了未分组图标");
                    if (!compactPlacement)
                        Require(new GridStore(root).State.Boxes.Single().DisplayMode == GridBox.Expanded,
                            "拖动覆盖了始终展开模式");
                }
                if (grouping)
                    VerifyRendererGrouping(root, lease, rendererWindow, renderer, document);
                if (advanced)
                    lease = VerifyAdvancedExplorer(root, product.Id, lease, rendererWindow, renderer, managerHandle, moveFixture);
                if (displayDriver != null)
                    lease = VerifyRendererDisplayLayout(root, lease, rendererWindow, renderer, displayDriver, mixedDisplay,
                        stage =>
                        {
                            Require(Fingerprint(document, shortcut) == fingerprint, stage + "的样本路径、内容或属性发生变化");
                            try { VerifyProjectedShortcut(product.Id, shortcut, position, stage, baseline[1].NativeBounds); }
                            catch (TimeoutException error)
                            {
                                iconErrors.Add(error.Message);
                                Trace("GRID_DISPLAY_ICON_FAIL " + error.Message);
                            }
                            catch (InvalidOperationException error)
                            {
                                iconErrors.Add(error.Message);
                                Trace("GRID_DISPLAY_ICON_FAIL " + error.Message);
                            }
                        });

                var watch = Stopwatch.StartNew();
                bool gracefulExit = scenario == "normal-exit" || tabs || placement || grouping || advanced || shortcutEdge || displayDriver != null;
                if (gracefulExit) host.Kill();
                if (scenario == "forced-end") product.Kill();
                if (scenario == "renderer-end")
                    using (var process = lease.Renderer.OpenProcess())
                    {
                        Require(process != null && References.Equal(process.MainModule.FileName, renderer),
                            "界面故障目标不属于隔离测试包");
                        process.Kill();
                    }
                if (scenario == "ui-hang" || scenario == "renderer-hang")
                {
                    uint pid;
                    bool freezeRenderer = scenario == "renderer-hang";
                    uint thread = Native.GetWindowThreadProcessId(freezeRenderer ? rendererWindow : lease.Owner.Hwnd, out pid);
                    Require(pid == (freezeRenderer ? lease.Renderer.Pid : product.Id), "控制窗口不属于测试程序");
                    var handle = OpenThread(2, false, thread);
                    Require(handle != IntPtr.Zero, "无法冻结测试界面线程");
                    try { Require(SuspendThread(handle) != uint.MaxValue, "未能模拟界面卡死"); }
                    finally { CloseHandle(handle); }
                }
                int explorerPid = lease.Desktop.Pid;
                if (scenario == "explorer-restart")
                {
                    lease = RestartTestExplorer(root, lease, renderer);
                    explorerPid = lease.Desktop.Pid;
                    VerifyProjectedShortcut(product.Id, shortcut, position, "Explorer 重连后");
                    host.Kill();
                }
                List<DesktopItem> restored = null;
                Wait(() => (restored = ReadIcons(document, shortcut)) != null, 22000,
                    "Explorer 图标未恢复可见", () => Error(root));
                long recoveryMs = watch.ElapsedMilliseconds;
                // Explorer 选中长文件名时会展开文字区域，因此验证图标锚点和原坐标上的实际双击。
                Require(CenterIcon(restored[1].NativeBounds) == position, "未分组快捷方式坐标变化 · " +
                    baseline[1].NativeBounds + " → " + restored[1].NativeBounds);
                Require(CenterIcon(restored[0].NativeBounds) == CenterIcon(baseline[0].NativeBounds),
                    "分组文件恢复时原坐标变化 · " + baseline[0].NativeBounds + " → " + restored[0].NativeBounds);
                Wait(() => recovery.Signaled("done", lease.Token), 6500, "恢复进程没有交还图标");
                if (renderer != null)
                    Wait(() =>
                    {
                        using (var process = lease.Renderer.OpenProcess()) return process == null;
                    }, 6500, "Explorer 已恢复，但 WinUI 格子进程仍在运行");
                if ((gracefulExit || scenario == "explorer-restart") &&
                    (!product.WaitForExit(6500) || product.ExitCode != 0))
                    throw new InvalidOperationException("格子引擎未正常退出 · " + Error(root));
                Wait(() => HitPid(position) == explorerPid && Native.Responsive(lease.Desktop),
                    6000, "恢复后原坐标没有命中可响应的 Explorer");
                DoubleClickAndCheck(position, "还原原生桌面后");
                Require(Fingerprint(document, shortcut) == fingerprint, "恢复后路径、SHA256 或属性变化");
                casePassed = iconErrors.Count == 0;
                return new { scenario, passed = casePassed, startupMs, recoveryMs, originalExplorerPid = shell.View.Pid,
                    resultingExplorerPid = explorerPid, clickedWhileManaged = true, clickedAfterRecovery = true,
                    rendererVisibleWhileManaged = renderer != null,
                    mouseCollapseAndFileAndShortcutDrop = gestures,
                    shortcutDropOnWidgetEdge = shortcutEdge,
                    activeTabAndManualCollapseSaved = tabs,
                    mousePlacementSaved = placement,
                    mergedAndDetachedWithoutMovingFiles = grouping,
                    winUiDualDisplayRestored = displayDriver != null,
                    mixedDpiExplorerRestarted = mixedDisplay,
                    unchangedUngroupedCoordinates = casePassed,
                    unchangedGroupedCoordinatesAfterRecovery = true, iconErrors,
                    pathsAttributesAndSha256Preserved = true };
            }
            finally
            {
                try
                {
                    if (!host.HasExited) host.Kill();
                    using (var restore = Start(executable, "--restore \"" + root + "\"")) restore.WaitForExit(5000);
                    if (!product.WaitForExit(6500)) product.Kill();
                }
                catch (InvalidOperationException) { }
            }
        }
    }

    private static GridLease RestartTestExplorer(string root, GridLease previous, string renderer)
    {
        VerifyGuestIdentity();
        string path = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Windows), "explorer.exe");
        using (var explorer = previous.Desktop.OpenProcess())
        {
            Require(explorer != null && References.Equal(explorer.MainModule.FileName, path), "Explorer 身份不匹配");
            explorer.Kill();
            Require(explorer.WaitForExit(10000), "旧 Explorer 未退出");
        }
        var recovery = new Recovery(root);
        var watch = Stopwatch.StartNew();
        GridLease current = null;
        bool launchRequested = false;
        Wait(() =>
        {
            var shell = ShellDesktop.Find();
            if (shell == null)
            {
                if (!launchRequested && watch.ElapsedMilliseconds > 10000 && Native.GetShellWindow() == IntPtr.Zero)
                {
                    launchRequested = true;
                    using (Start(path, "")) { }
                }
                return false;
            }
            current = recovery.Current();
            return current != null && current.Token != previous.Token && current.Desktop.Pid != previous.Desktop.Pid &&
                shell.View.Pid == current.Desktop.Pid && recovery.Signaled("ready", current.Token);
        }, 130000, "Explorer 重启后格子未重新连接", () => Error(root));
        if (renderer != null)
        {
            Require(current.RendererSession != previous.RendererSession, "Explorer 重启后沿用了旧界面会话");
            VerifyActiveRenderer(root, renderer, current);
        }
        Trace("GRID_REJOINED explorer-restart " + current.Desktop.Pid);
        return current;
    }

    private static void VerifyProjectedShortcut(int productPid, string shortcut, Point position, string stage,
        Rectangle? originalBounds = null)
    {
        Wait(() => HitPid(position) == productPid, 7000, stage + "未分组图标没有留在原坐标",
            () => ProjectedShortcutLocation(productPid, shortcut, position));
        var icon = WindowFromPoint(position);
        string actualLabel = Title(icon, true);
        Require(actualLabel == Path.GetFileNameWithoutExtension(shortcut) || actualLabel == Path.GetFileName(shortcut),
            stage + "原坐标命中了其他图标 · " + actualLabel + " · " + position);
        if (originalBounds.HasValue)
        {
            WindowRect window;
            Require(GetWindowRect(icon, out window), stage + "无法读取未分组图标的位置");
            var actual = Rectangle.FromLTRB(window.Left, window.Top, window.Right, window.Bottom);
            Require(actual == originalBounds.Value, stage + "未分组图标的边界发生变化 · " + originalBounds + " → " + actual);
        }
        DoubleClickAndCheck(position, stage);
        Trace("GRID_DISPLAY_ICON_PASS " + stage + " " + position);
    }

    private static string ProjectedShortcutLocation(int productPid, string shortcut, Point position)
    {
        var locations = new List<string>();
        var shell = ShellDesktop.Find();
        if (shell != null)
            EnumChildWindows(shell.Parent, (window, _) =>
            {
                uint pid;
                Native.GetWindowThreadProcessId(window, out pid);
                if (pid == productPid && Native.IsWindowVisible(window))
                {
                    string label = Title(window, true);
                    if (label == Path.GetFileNameWithoutExtension(shortcut) || label == Path.GetFileName(shortcut))
                    {
                        WindowRect bounds;
                        if (GetWindowRect(window, out bounds))
                            locations.Add(Rectangle.FromLTRB(bounds.Left, bounds.Top, bounds.Right, bounds.Bottom).ToString());
                    }
                }
                return true;
            }, IntPtr.Zero);
        var hit = WindowFromPoint(position);
        var type = new StringBuilder(120);
        GetClassName(hit, type, type.Capacity);
        return "原" + position + " PID=" + HitPid(position) + " " + type +
            "，格子引擎 PID=" + productPid + "，投影位置=" + string.Join(", ", locations);
    }

    private static void EnableGrid(IntPtr manager)
    {
        IntPtr button = IntPtr.Zero;
        EnumChildWindows(manager, (window, _) =>
        {
            if (Title(window, true) == "启用格子") button = window;
            return button == IntPtr.Zero;
        }, IntPtr.Zero);
        Require(button != IntPtr.Zero, "启用按钮没有出现 · " + ManagerContents(manager));
        var control = AutomationElement.FromHandle(button);
        object invoke;
        if (control.TryGetCurrentPattern(InvokePattern.Pattern, out invoke))
            ((InvokePattern)invoke).Invoke();
        else
            PostMessage(GetParent(button), 0x111, new IntPtr(GetDlgCtrlID(button)), button);
    }

    private static IntPtr VerifyActiveRenderer(string root, string renderer, GridLease lease,
        string title = null, int count = 1)
    {
        Require(lease.Renderer != null && References.Equal(lease.RendererImage, renderer),
            "接管租约未保护指定的 WinUI 界面");
        IntPtr window = IntPtr.Zero;
        Wait(() =>
        {
            string directory = Path.Combine(root, "deskbox-renderer-" + lease.RendererSession);
            var pulse = LocalJson.TryRead<GridRendererPulse>(Path.Combine(directory, "pulse.json"));
            var published = LocalJson.TryRead<GridRenderState>(Path.Combine(directory, "state.json"));
            if (pulse == null || pulse.Session != lease.RendererSession || pulse.Pid != lease.Renderer.Pid ||
                pulse.Windows == null || pulse.Windows.Length != count || published == null ||
                published.Session != lease.RendererSession || !published.Visible || published.Closing ||
                pulse.Revision != published.Revision)
                return false;
            var current = pulse.Windows.Select(handle => new IntPtr(handle))
                .FirstOrDefault(handle => title == null || Title(handle).StartsWith(title, StringComparison.Ordinal));
            var identity = WindowIdentity.Read(current);
            var shell = ShellDesktop.Find();
            int cloaked;
            WindowRect bounds;
            if (shell == null || identity == null || identity.Pid != lease.Renderer.Pid ||
                Native.GetWindow(current, 4) != shell.Host.Hwnd || !Native.IsWindowVisible(current) ||
                DwmGetWindowAttribute(current, 14, out cloaked, sizeof(int)) != 0 || cloaked != 0 ||
                !GetWindowRect(current, out bounds) || bounds.Right - bounds.Left < 50 || bounds.Bottom - bounds.Top < 20)
                return false;
            window = current;
            return true;
        }, 10000, "WinUI 格子没有在当前 Explorer 桌面显示", () => Error(root));
        Trace("GRID_RENDERER_ACTIVE " + lease.Renderer.Pid + " " + lease.RendererSession);
        return window;
    }

    private static void VerifyRendererGestures(string root, int productPid, int rendererPid, IntPtr window,
        string document, Point documentPosition, string shortcut, Point shortcutPosition)
    {
        WindowRect capsule;
        Require(GetWindowRect(window, out capsule), "无法读取格子标题条坐标");
        int height = capsule.Bottom - capsule.Top;
        var header = new Point(capsule.Left + (capsule.Right - capsule.Left) / 2, capsule.Top + height / 2);
        Require(HitPid(header) == rendererPid, "桌面鼠标无法命中格子标题条");
        SetCursorPos(header.X, header.Y);
        Wait(() =>
        {
            WindowRect bounds;
            return GetWindowRect(window, out bounds) && bounds.Bottom - bounds.Top > height + 36;
        }, 8500, "鼠标悬停后未展开格子");
        SetCursorPos(shortcutPosition.X, shortcutPosition.Y);
        Wait(() =>
        {
            WindowRect bounds;
            return GetWindowRect(window, out bounds) && bounds.Bottom - bounds.Top <= height + 5;
        }, 8500, "鼠标移开后未折叠格子");
        DragToRenderer(root, productPid, window, documentPosition, document, header, height);
        DragToRenderer(root, productPid, window, shortcutPosition, shortcut, header, height, true);
        Trace("GRID_RENDERER_GESTURES_PASS");
    }

    private static GridLease VerifyAdvancedExplorer(string root, int engine, GridLease lease, IntPtr first, string renderer,
        IntPtr manager, string moveFixture)
    {
        var second = VerifyActiveRenderer(root, renderer, lease, "联动验收格子", 2);
        var original = new GridStore(root).State;
        Require(original.Boxes.All(box => box.BoundMonitor == "@primary"), "主屏绑定未恢复");
        string session = Path.Combine(root, "deskbox-renderer-" + lease.RendererSession);
        var ready = new JavaScriptSerializer().DeserializeObject(File.ReadAllText(Path.Combine(session, "preview-ready.json")))
            as Dictionary<string, object>;
        var windows = (System.Collections.IEnumerable)ready["windows"];
        var details = windows.Cast<Dictionary<string, object>>().Single(item => (string)item["name"] == "产品接管测试");
        Trace("GRID_ADVANCED_CONTENT " + new JavaScriptSerializer().Serialize(details));
        Require((bool)details["titleImage"] && (bool)details["backgroundImage"] &&
            ((System.Collections.IEnumerable)details["stacks"]).Cast<string>().Contains("验收自定义堆栈"),
            "真实格子未读取标题图标、背景图或自定义堆栈");
        WindowRect a, b = new WindowRect();
        Require(GetWindowRect(first, out a) && GetWindowRect(second, out b), "联动前窗口无效");
        var primary = Screen.PrimaryScreen.WorkingArea;
        Require(primary.Contains(a.Left, a.Top) && primary.Contains(b.Left, b.Top), "绑定的格子没有定位至主屏");
        var title = AutomationElement.FromHandle(first).FindFirst(TreeScope.Descendants,
            new PropertyCondition(AutomationElement.AutomationIdProperty, "TitleText"));
        Require(title != null, "未找到联动拖动的标题");
        var bounds = title.Current.BoundingRectangle;
        var start = new Point((int)(bounds.Left + bounds.Width / 2), (int)(bounds.Top + bounds.Height / 2));
        keybd_event(0x11, 0, 0, UIntPtr.Zero);
        try { DragWindowTitle(first, a, start, new Point(start.X - 86, start.Y + 40)); }
        finally { keybd_event(0x11, 0, 2, UIntPtr.Zero); }
        Wait(() =>
        {
            var moved = new GridStore(root).State;
            WindowRect left, right;
            return moved.Layouts.Any(profile => profile.Key != "qa-initial" &&
                profile.Value.Keys.Intersect(original.Boxes.Select(box => box.Id)).Count() == 2) &&
                GetWindowRect(first, out left) && GetWindowRect(second, out right) &&
                Math.Abs(left.Left - a.Left) >= 30 && Math.Abs((left.Left - a.Left) - (right.Left - b.Left)) < 5;
        }, 15000, "真实鼠标 Ctrl 拖动未同时保存两个格子");
        Trace("GRID_ADVANCED_MOVE_PASS");
        foreach (var box in original.Boxes)
            SendAdvancedRequest(session, new GridRenderRequest { Action = "collapse-mode", Box = box.Id, CollapseMode = GridBox.Click });
        Wait(() =>
        {
            WindowRect left, right;
            return GetWindowRect(first, out left) && GetWindowRect(second, out right) &&
                left.Bottom - left.Top < 85 && right.Bottom - right.Top < 85 && Math.Abs(left.Top - right.Top) <= 2 &&
                (left.Right <= right.Left || right.Right <= left.Left);
        }, 10000, "真实 Explorer 中的两个胶囊未横向排列");
        Trace("GRID_ADVANCED_CAPSULE_PASS");

        SetForegroundWindow(manager);
        var management = AutomationElement.FromHandle(manager);
        ClickNamed(management, "设置");
        int settingsOwner = 0;
        IntPtr settingsWindow = IntPtr.Zero;
        Wait(() => (settingsWindow = FindPreviewForm("SoulDesk · 设置", out settingsOwner)) != IntPtr.Zero,
            20000, "未打开独立 WinUI 设置窗口");
        string settingsSession = Directory.GetDirectories(root, "deskbox-renderer-*").Single(directory =>
        {
            var state = LocalJson.TryRead<GridRenderState>(Path.Combine(directory, "state.json"));
            return state != null && state.SettingsOnly && !state.Closing;
        });
        SendAdvancedRequest(settingsSession, new GridRenderRequest
            { Action = "settings-edit", Setting = "WidgetCapsuleBarPlacement", Value = "Bottom" });
        Wait(() =>
        {
            WindowRect left, right;
            return GetWindowRect(first, out left) && GetWindowRect(second, out right) &&
                Math.Abs(left.Bottom - primary.Bottom) <= 3 && Math.Abs(right.Bottom - primary.Bottom) <= 3;
        }, 10000, "真实桌面的两个胶囊未应用贴底设置");
        Require(new GridStore(root).State.Preferences["WidgetCapsuleBarPlacement"] == "Bottom",
            "胶囊贴底设置未写入布局");
        SendAdvancedRequest(settingsSession, new GridRenderRequest
            { Action = "settings-edit", Setting = "WidgetCapsuleBarPlacement", Value = "Floating" });
        SendAdvancedRequest(session, new GridRenderRequest
            { Action = "collapse-mode", Box = original.Boxes[1].Id, CollapseMode = GridBox.Expanded });
        SendAdvancedRequest(settingsSession, new GridRenderRequest
            { Action = "settings-edit", Setting = "ShowFileExtensions", Value = "true" });
        Wait(() => Named(AutomationElement.FromHandle(second), "display-preference-proof.log") != null,
            12000, "真实格子没有应用文件扩展名显示设置");
        Trace("GRID_ADVANCED_PREFERENCES_PASS");
        Require(PostMessage(settingsWindow, 0x10, IntPtr.Zero, IntPtr.Zero), "未能关闭 WinUI 设置窗口");
        Wait(() => !Native.IsWindowVisible(settingsWindow), 8000, "WinUI 设置窗口关闭后仍然可见");
        SetForegroundWindow(manager);
        Trace("GRID_ADVANCED_SETTINGS_PASS");
        ClickNamed(AutomationElement.FromHandle(manager), "桌面整理");
        AutomationElement previewButton = null;
        Wait(() => (previewButton = Named(AutomationElement.FromHandle(manager), "预览搬移")) != null &&
            !previewButton.Current.IsOffscreen, 12000, "整理页面没有显示搬移按钮", () => ManagerContents(manager));
        File.WriteAllText(moveFixture, "SoulDesk isolated original file");
        InvokeOrClick(previewButton);
        AutomationElement moveButton = null;
        Wait(() => (moveButton = Named(AutomationElement.RootElement, "搬移勾选的原文件")) != null,
            20000, "没有打开搬移预览", () => ManagerContents(manager) + " " + Error(root));
        InvokeOrClick(moveButton);
        var history = new GridOrganization(root);
        OrganizedItem receipt = null;
        Wait(() => (receipt = history.Read().FirstOrDefault(item => item.Source == moveFixture && item.Status == "moved")) != null &&
            !File.Exists(moveFixture) && File.Exists(receipt.Destination), 20000, "真实桌面搬移未产生完成记录");
        ClickNamed(AutomationElement.FromHandle(manager), "查看历史");
        var name = AutomationElement.RootElement.FindFirst(TreeScope.Descendants,
            new PropertyCondition(AutomationElement.NameProperty, "已搬移"));
        object objectPattern = null;
        Require(name != null && name.TryGetCurrentPattern(SelectionItemPattern.Pattern, out objectPattern),
            "未找到可撤回的搬移记录");
        ((SelectionItemPattern)objectPattern).Select();
        ClickNamed(AutomationElement.RootElement, "恢复到原路径");
        Wait(() => history.Read().Any(item => item.Id == receipt.Id && item.Status == "restored" && item.LayoutSynced) &&
            File.Exists(moveFixture) && !File.Exists(receipt.Destination), 20000, "真实桌面未恢复原文件");
        Require(File.ReadAllText(moveFixture) == "SoulDesk isolated original file", "文件搬移和撤回改变了内容");
        Trace("GRID_ADVANCED_ORGANIZATION_PASS");
        var recovery = new Recovery(root);
        GridLease latest = null;
        Wait(() => (latest = recovery.Current()) != null && recovery.Signaled("ready", latest.Token) &&
            !recovery.Signaled("done", latest.Token) && !Native.IsWindowVisible(latest.Desktop.Hwnd), 75000,
            "整理后 Explorer 接管未恢复");
        Require(latest.Owner.Pid == engine, "整理后换用了其他格子进程");
        int historyOwner;
        FindPreviewForm("整理历史", out historyOwner);
        Require(historyOwner == engine, "停止宿主前整理历史对话框没有保持打开");
        return latest;
    }

    private static void SendAdvancedRequest(string session, GridRenderRequest request)
    {
        string statePath = Path.Combine(session, "state.json");
        // 用于自动验收的请求绕过了界面的互斥提交。若恰逢桌面刷新，只对版本冲突重取一次快照。
        for (int attempt = 0; attempt < 2; attempt++)
        {
            var state = LocalJson.Read<GridRenderState>(statePath);
            request.Version = state.Version;
            request.Session = state.Session;
            request.Revision = state.Revision;
            request.Id = Guid.NewGuid().ToString("N");
            LocalJson.Write(Path.Combine(session, "request.json"), request);
            GridRenderState reply = null;
            Wait(() =>
            {
                reply = LocalJson.TryRead<GridRenderState>(statePath);
                return reply != null && reply.Completed == request.Id;
            }, 7000, "格子引擎未确认验收请求 " + (request.Setting ?? request.Action));
            if (reply.Error == null) return;
            if (attempt == 0 && reply.Error == "格子已更新，请重新操作。") continue;
            throw new InvalidOperationException("格子引擎拒绝验收请求 " +
                (request.Setting ?? request.Action) + " · " + reply.Error);
        }
    }

    private static void VerifyRendererTabs(string root, GridLease lease, IntPtr window, string renderer)
    {
        string path = Path.Combine(root, "layout.json");
        var original = GridStore.ReadLayout(path).Boxes.Single();
        string target = original.Tabs.Single(tab => tab.Name == "标签归档").Id;
        var surface = AutomationElement.FromHandle(window);
        ClickNamed(surface, "标签归档");
        Wait(() => GridStore.ReadLayout(path).Boxes.Single().Active == target &&
            Title(window).StartsWith("标签归档", StringComparison.Ordinal), 10000, "切换的标签没有应用到原窗口或写入布局");
        Require(VerifyActiveRenderer(root, renderer, lease, "标签归档") == window, "切换标签时更换了格子窗口");

        WindowRect expanded;
        Require(GetWindowRect(window, out expanded), "无法验证折叠前的格子坐标");
        SetCursorPos(expanded.Left + 30, expanded.Top + 22);
        var fold = new OrCondition(
            new PropertyCondition(AutomationElement.AutomationIdProperty, "CollapseButton"),
            new PropertyCondition(AutomationElement.NameProperty, "收起"),
            new PropertyCondition(AutomationElement.NameProperty, "折叠"));
        AutomationElement button = null;
        Wait(() => (button = surface.FindFirst(TreeScope.Descendants, fold)) != null, 10000, "找不到格子的折叠按钮");
        object invoke;
        if (button.TryGetCurrentPattern(InvokePattern.Pattern, out invoke))
            ((InvokePattern)invoke).Invoke();
        else
            ClickAt(button, false);
        Wait(() =>
        {
            WindowRect compact;
            return GetWindowRect(window, out compact) && compact.Bottom - compact.Top < expanded.Bottom - expanded.Top - 36 &&
                GridStore.ReadLayout(path).Boxes.Single().Collapsed;
        }, 10000, "手动折叠没有应用到窗口或写入布局");
        Require(new GridStore(root).State.Boxes.Single().Active == target, "折叠时丢失当前标签");
        Trace("GRID_RENDERER_TABS_PASS");
    }

    private static void VerifyRendererGrouping(string root, GridLease lease, IntPtr sourceWindow, string renderer, string document)
    {
        var store = new GridStore(root);
        var source = store.State.Boxes.Single(box => box.Current().Name == "产品接管测试");
        var target = store.State.Boxes.Single(box => box.Current().Name == "验收目标格子");
        string sourceTab = source.Active;
        var targetWindow = VerifyActiveRenderer(root, renderer, lease, "验收目标格子", 2);

        OpenRendererGroupMenu(sourceWindow, lease.Renderer.Pid);
        var join = FindRendererMenuItem(lease.Renderer.Pid, "组合格子…", "Combine with another widget…");
        ClickAt(join, false);
        var targetMenuItem = FindRendererMenuItem(lease.Renderer.Pid, "验收目标格子");
        InvokeOrClick(targetMenuItem);
        Wait(() =>
        {
            var merged = new GridStore(root).State.Boxes;
            return merged.Count == 1 && merged[0].Id == target.Id && merged[0].Active == target.Active &&
                merged[0].Tabs.Select(tab => tab.Id).SequenceEqual(new[] { target.Active, sourceTab }) &&
                merged[0].Tabs[1].Items.Contains(document);
        }, 10000, "格子菜单合组后没有保存正确的标签和文件引用",
            () => GridRendererRequestSummary(root, lease.RendererSession));
        Require(VerifyActiveRenderer(root, renderer, lease) == targetWindow, "合组后没有保留目标格子窗口");
        Trace("GRID_RENDERER_GROUP_MERGE_PASS");

        ClickNamed(AutomationElement.FromHandle(targetWindow), "产品接管测试");
        Wait(() => new GridStore(root).State.Boxes.Single().Active == sourceTab &&
            Title(targetWindow).StartsWith("产品接管测试", StringComparison.Ordinal),
            10000, "合组后的文件标签没有切换到界面");
        Require(VerifyActiveRenderer(root, renderer, lease, "产品接管测试") == targetWindow,
            "合组后切换活动标签时更换了窗口");
        // 合组后标签条取代单格标题，在标签条右侧的标题栏空白处打开窗口菜单。
        WindowRect groupedBounds;
        Require(GetWindowRect(targetWindow, out groupedBounds), "无法读取格子组标题栏坐标");
        var screen = Screen.FromHandle(targetWindow);
        var header = new Point(groupedBounds.Right - Native.Scale(screen, 9), groupedBounds.Top + Native.Scale(screen, 17));
        Require(HitPid(header) == lease.Renderer.Pid, "格子组标题栏被其他窗口遮挡");
        SetCursorPos(header.X, header.Y);
        mouse_event(8, 0, 0, 0, UIntPtr.Zero);
        mouse_event(16, 0, 0, 0, UIntPtr.Zero);
        InvokeOrClick(FindRendererMenuItem(lease.Renderer.Pid, "移出格子组", "Remove from widget group"));
        Wait(() =>
        {
            var detached = new GridStore(root).State.Boxes;
            return detached.Count == 2 && detached.Any(box => box.Id == target.Id && box.Active == target.Active) &&
                detached.Any(box => box.Id != target.Id && box.Active == sourceTab && box.Current().Items.Contains(document));
        }, 10000, "从菜单拆出标签后没有恢复两个独立格子",
            () => GridRendererRequestSummary(root, lease.RendererSession));
        VerifyActiveRenderer(root, renderer, lease, "产品接管测试", 2);
        Trace("GRID_RENDERER_GROUP_PASS");
    }

    private static void OpenRendererGroupMenu(IntPtr window, int rendererPid)
    {
        var surface = AutomationElement.FromHandle(window);
        var titleIds = new OrCondition(new[] { "TitleText", "CompactTitleText", "OverlayTitleText" }
            .Select(id => (Condition)new PropertyCondition(AutomationElement.AutomationIdProperty, id)).ToArray());
        AutomationElement title = null;
        Wait(() =>
        {
            title = surface.FindAll(TreeScope.Descendants, titleIds).Cast<AutomationElement>()
                .FirstOrDefault(element => !element.Current.IsOffscreen && !element.Current.BoundingRectangle.IsEmpty);
            return title != null;
        }, 8000, "找不到可点击的格子标题");
        var bounds = title.Current.BoundingRectangle;
        Require(HitPid(new Point((int)(bounds.Left + bounds.Width / 2), (int)(bounds.Top + bounds.Height / 2))) == rendererPid,
            "格子标题被其他窗口遮挡");
        ClickAt(title, true);
    }

    private static string GridRendererRequestSummary(string root, string session)
    {
        var directory = Path.Combine(root, "deskbox-renderer-" + session);
        var request = LocalJson.TryRead<GridRenderRequest>(Path.Combine(directory, "request.json"));
        var state = LocalJson.TryRead<GridRenderState>(Path.Combine(directory, "state.json"));
        return request == null ? "界面未提交请求" :
            request.Action + " · " + (state == null ? "无状态回执" : state.Error ?? "无拒绝记录");
    }

    private static AutomationElement FindRendererMenuItem(int rendererPid, params string[] names)
    {
        var nameConditions = names.Select(name => (Condition)new PropertyCondition(AutomationElement.NameProperty, name)).ToArray();
        var filter = new AndCondition(new PropertyCondition(AutomationElement.ProcessIdProperty, rendererPid),
            new PropertyCondition(AutomationElement.ControlTypeProperty, System.Windows.Automation.ControlType.MenuItem),
            nameConditions.Length == 1 ? nameConditions[0] : new OrCondition(nameConditions));
        AutomationElement result = null;
        Wait(() => (result = AutomationElement.RootElement.FindFirst(TreeScope.Descendants, filter)) != null &&
            !result.Current.IsOffscreen, 8000, "未找到格子菜单项 " + names[0]);
        return result;
    }

    private static void InvokeOrClick(AutomationElement item)
    {
        object invoke;
        if (item.TryGetCurrentPattern(InvokePattern.Pattern, out invoke))
            ((InvokePattern)invoke).Invoke();
        else
            ClickAt(item, false);
    }

    private static WindowRect VerifyRendererPlacement(string root, GridLease lease, IntPtr window, string renderer,
        bool externalPointer, bool compact, Point? shift = null, string destinationMonitor = null)
    {
        WindowRect initial = new WindowRect();
        int expandedThreshold = Native.Scale(Screen.PrimaryScreen, 84);
        Wait(() => GetWindowRect(window, out initial) &&
            (initial.Bottom - initial.Top < expandedThreshold) == compact, 8000, "格子未进入待验证的展开或收起状态");
        Point start;
        if (compact)
        {
            // 折叠胶囊左侧图标是拖动区，中间标题用于悬停展开。
            start = new Point(initial.Left + Native.Scale(Screen.PrimaryScreen, 27),
                (initial.Top + initial.Bottom) / 2);
        }
        else
        {
            var title = AutomationElement.FromHandle(window).FindFirst(TreeScope.Descendants,
                new PropertyCondition(AutomationElement.AutomationIdProperty, "TitleText"));
            var header = title == null ? System.Windows.Rect.Empty : title.Current.BoundingRectangle;
            Require(!header.IsEmpty && !title.Current.IsOffscreen, "找不到展开格子的可见标题");
            start = new Point((int)(header.Left + header.Width / 2), (int)(header.Top + header.Height / 2));
        }
        Require(HitPid(start) == lease.Renderer.Pid, "无法点击格子标题栏");
        var movement = shift ?? new Point(-180, 125);
        var destination = new Point(start.X + movement.X, start.Y + movement.Y);
        var hit = AutomationElement.FromPoint(new System.Windows.Point(start.X, start.Y));
        var nativeHit = WindowFromPoint(start);
        var nativeClass = new StringBuilder(128);
        GetClassName(nativeHit, nativeClass, nativeClass.Capacity);
        UIntPtr hitCode;
        SendMessageTimeout(nativeHit, 0x84, UIntPtr.Zero,
            new IntPtr((start.Y << 16) | (start.X & 0xffff)), 2, 500, out hitCode);
        Trace("GRID_RENDERER_HIT " + hit.Current.Name + " / " + hit.Current.ClassName + " / " +
            hit.Current.AutomationId + " / " + nativeClass + " / " + unchecked((int)hitCode.ToUInt64()));
        Trace("GRID_RENDERER_MOVE " + start + " -> " + destination);
        if (externalPointer)
        {
            var bounds = Screen.PrimaryScreen.Bounds;
            Trace("GRID_RENDERER_AWAIT_HID " + start.X + " " + start.Y + " " + destination.X + " " + destination.Y +
                " " + bounds.Width + " " + bounds.Height);
        }
        else
            DragWindowTitle(window, initial, start, destination);
        Wait(() =>
        {
            WindowRect current;
            return !Native.IsWindow(window) || GetWindowRect(window, out current) &&
                Math.Abs(initial.Left - current.Left) > 100 && Math.Abs(initial.Top - current.Top) > 80;
        }, externalPointer ? 45000 : 5000, "鼠标未能拖动格子的可见标题栏");
        Wait(() => new GridStore(root).State.Layouts.Values.SelectMany(profile => profile.Values)
            .Any(position => destinationMonitor == null || position.Monitor == destinationMonitor),
                10000, "手动拖动的格子位置没有写入目标显示器布局");
        var placed = VerifyActiveRenderer(root, renderer, lease);
        Require(placed == window, "保存位置后重建了格子窗口");
        WindowRect rect;
        Require(GetWindowRect(placed, out rect), "保存位置后无法读取格子坐标");
        var expected = new Point(initial.Left + movement.X, initial.Top + movement.Y);
        Require(Math.Abs(expected.X - rect.Left) < 25 && Math.Abs(expected.Y - rect.Top) < 25,
            "保存的格子位置偏离拖动终点 · 预期 " + expected + " · 实际 " + new Point(rect.Left, rect.Top));
        Require((rect.Bottom - rect.Top < expandedThreshold) == compact &&
            new GridStore(root).State.Boxes.Single().Collapsed == compact,
            "拖动改变了格子的展开或收起状态");
        Trace("GRID_RENDERER_POSITION_PASS");
        return rect;
    }

    private static void DragWindowTitle(IntPtr window, WindowRect initial, Point start, Point destination)
    {
        SetCursorPos(start.X, start.Y);
        Thread.Sleep(160);
        mouse_event(2, 0, 0, 0, UIntPtr.Zero);
        try
        {
            int maxX = 0, maxY = 0;
            for (int step = 1; step <= 14; step++)
            {
                SetCursorPos(start.X + (destination.X - start.X) * step / 14,
                    start.Y + (destination.Y - start.Y) * step / 14);
                Thread.Sleep(55);
                WindowRect current;
                if (GetWindowRect(window, out current))
                {
                    maxX = Math.Max(maxX, Math.Abs(initial.Left - current.Left));
                    maxY = Math.Max(maxY, Math.Abs(initial.Top - current.Top));
                }
            }
            Trace("GRID_RENDERER_MOVE_OBSERVED " + maxX + "," + maxY);
        }
        finally { mouse_event(4, 0, 0, 0, UIntPtr.Zero); }
    }

    private static void DragToRenderer(string root, int productPid, IntPtr window, Point origin,
        string path, Point header, int collapsedHeight, bool windowEdge = false)
    {
        Wait(() => HitPid(origin) == productPid, 5500, "拖放前原坐标没有命中桌面文件");
        SetCursorPos(origin.X, origin.Y);
        Thread.Sleep(180);
        mouse_event(2, 0, 0, 0, UIntPtr.Zero);
        try
        {
            SetCursorPos(origin.X + 11, origin.Y);
            Thread.Sleep(170);
            for (int step = 1; step <= 18; step++)
            {
                SetCursorPos(origin.X + (header.X - origin.X) * step / 18, origin.Y + (header.Y - origin.Y) * step / 18);
                Thread.Sleep(70);
            }
            WindowRect expanded = new WindowRect();
            Wait(() => GetWindowRect(window, out expanded) && expanded.Bottom - expanded.Top > collapsedHeight + 36,
                6000, "拖放经过标题条时格子未展开");
            var target = new Point(windowEdge ? expanded.Right - 4 : expanded.Left + (expanded.Right - expanded.Left) * 2 / 3,
                expanded.Top + Math.Min(100, (expanded.Bottom - expanded.Top) / 2));
            SetCursorPos(target.X, target.Y);
            Trace("GRID_FILE_DROP_TARGET " + Path.GetExtension(path) + " " + target + " " + HitPid(target));
            Thread.Sleep(450);
        }
        finally { mouse_event(4, 0, 0, 0, UIntPtr.Zero); }
        Wait(() => GridStore.ReadLayout(Path.Combine(root, "layout.json")).Boxes[0].Current().Items.Contains(path),
            8500, "系统鼠标拖放未保存原地引用 · " + Path.GetExtension(path));
    }

    private static void VerifyModel(string root)
    {
        var fixtures = Path.Combine(root, "fixtures");
        Directory.CreateDirectory(fixtures);
        string document = Path.Combine(fixtures, "测试文件.pdf");
        string shortcut = Path.Combine(fixtures, "Shortcut.lnk");
        File.WriteAllText(document, "document fixture");
        File.WriteAllText(shortcut, "shortcut fixture");
        string old = Fingerprint(document, shortcut);
        var store = new GridStore(root);
        string first = null, second = null, tab = null;
        store.Change(state =>
        {
            var a = state.Add("文档", null);
            a.Current().Rules = "*.pdf";
            first = a.Id; tab = a.Current().Id;
            second = state.Add("快捷方式", fixtures).Id;
            state.Attach(tab, new[] { document, shortcut });
            GridPreferences.Set(state, "ShowFileExtensions", "true");
            GridPreferences.Set(state, "WidgetCapsuleBarPlacement", "Bottom");
            state.NewWidgetMonitor = "@cursor";
            state.Place("双屏", first, new GridPosition { Monitor = "副屏", X = .5, Y = .3 });
            state.Place("单屏", first, new GridPosition { Monitor = "主屏", X = .2, Y = .8 });
        });
        Require(GridState.Matches(store.State.Tabs().First(), "测试文件.PDF"), "规则匹配失败");
        Require(!GridState.Matches(store.State.Tabs().First(), "测试文件.doc"), "规则匹配错误");
        string backup = Path.Combine(root, "export.json");
        store.Export(backup);
        var imported = GridStore.ReadLayout(backup);
        Require(imported.Tabs().First().Items.Count == 2, "备份丢失引用");
        Require(imported.Preferences["ShowFileExtensions"] == "true" &&
            imported.Preferences["WidgetCapsuleBarPlacement"] == "Bottom" &&
            imported.NewWidgetMonitor == "@cursor", "备份丢失格子全局设置");
        Require(!GridPreferences.Accepts("WidgetCapsuleBarPlacement", "Invalid") &&
            !GridPreferences.Accepts("WidgetOpacity", "101") &&
            !GridPreferences.Accepts("WidgetBackgroundUnifiedImage", @"..\background.png"), "错误设置值通过了校验");
        Require(imported.Position("双屏", first, new[] { "主屏", "副屏" }).Monitor == "副屏", "双屏布局丢失");
        Require(imported.Position("单屏", first, new[] { "主屏" }).Monitor == "主屏", "单屏布局丢失");
        store.Change(state => state.MoveTab(first, tab, second), true);
        Require(store.State.Boxes.Count == 1 && store.State.Boxes[0].Tabs.Count == 2, "合并标签组失败");
        store.Change(state => state.MoveTab(second, tab, null));
        Require(store.State.Boxes.Count == 2, "拆分标签组失败");
        Require(store.Undo() && store.State.Boxes.Any(box => box.Id == first), "撤销失败");
        store.Change(state => state.Release(new[] { shortcut }));
        Require(new GridStore(root).State.Tabs().First().Items.SequenceEqual(new[] { document }), "重开后分组内容错误");
        var matches = SearchIndex.Find("测试文件", new[] { document, shortcut }, true, false, null);
        Require(matches.Items.Count == 1 && matches.Items[0].Path == document, "检索测试文件失败");
        Require(Fingerprint(document, shortcut) == old, "仅更新分组时文件路径或内容变化");
    }

    private static void VerifyRendererModel(string root)
    {
        Directory.CreateDirectory(root);
        string document = Path.Combine(root, "原路径.txt");
        string shortcut = Path.Combine(root, "原快捷方式.lnk");
        string unrelated = Path.Combine(root, "未授权.txt");
        File.WriteAllText(document, "document");
        File.WriteAllText(shortcut, "shortcut");
        File.WriteAllText(unrelated, "outside current list");
        string originals = Fingerprint(document, shortcut, unrelated);
        var store = new GridStore(root);
        string source = null, destination = null;
        store.Change(state =>
        {
            source = state.Add("原格子", null).Current().Id;
            destination = state.Add("目标格子", null).Current().Id;
            state.Attach(source, new[] { document, shortcut });
        });

        using (var renderer = new GridRendererSession(root))
        {
            Action publish = () => renderer.Publish(store.State.Boxes.Select(box =>
                GridRenderBox.At(box, new Rectangle(40, 50, 450, 375), new Size(300, 250))),
                new[] { document, shortcut }, true);
            publish();
            var session = Directory.GetDirectories(root, "deskbox-renderer-*").Single();
            string stateFile = Path.Combine(session, "state.json");
            string requestFile = Path.Combine(session, "request.json");
            var state = LocalJson.Read<GridRenderState>(stateFile);
            Require(state.Visible && state.Boxes.Length == 2 && state.Boxes[0].Box.Current().Id == source &&
                state.Boxes[0].Width == 450 && state.Boxes[0].LogicalWidth == 300 &&
                state.Boxes[1].Box.Current().Id == destination,
                "外部界面未收到实际格子布局和位置");

            var request = new GridRenderRequest
            {
                Version = state.Version, Session = state.Session, Revision = state.Revision,
                Id = Guid.NewGuid().ToString("N"), Action = "group", Tab = destination,
                Paths = new[] { document, shortcut }
            };
            LocalJson.Write(requestFile, request);
            Require(renderer.ApplyRequest(store), "外部界面拖放未更新宿主布局");
            publish();
            Require(!renderer.ApplyRequest(store), "同一个拖放请求被重复处理");
            var accepted = LocalJson.Read<GridRenderState>(stateFile);
            var saved = new GridStore(root).State;
            Require(accepted.Completed == request.Id && accepted.Accepted.SequenceEqual(request.Paths) &&
                saved.Tabs().Single(tab => tab.Id == destination).Items.SequenceEqual(request.Paths) &&
                saved.Tabs().Single(tab => tab.Id == source).Items.Count == 0,
                "原路径分组没有持久化到同一份宿主布局");

            request.Id = Guid.NewGuid().ToString("N");
            request.Tab = source;
            LocalJson.Write(requestFile, request);
            Require(!renderer.ApplyRequest(store) && LocalJson.Read<GridRenderState>(stateFile).Error != null,
                "过期请求改写了新版布局");
            request.Id = Guid.NewGuid().ToString("N");
            request.Revision = accepted.Revision;
            request.Paths = new[] { unrelated };
            LocalJson.Write(requestFile, request);
            Require(!renderer.ApplyRequest(store) && LocalJson.Read<GridRenderState>(stateFile).Error != null,
                "外部界面加入了没有授权的路径");

            request.Id = Guid.NewGuid().ToString("N");
            request.Tab = destination;
            request.Action = "release";
            request.Paths = new[] { document };
            LocalJson.Write(requestFile, request);
            Require(renderer.ApplyRequest(store), "外部界面移出分组未更新宿主布局");
            publish();
            Require(new GridStore(root).State.Tabs().Single(tab => tab.Id == destination)
                .Items.SequenceEqual(new[] { shortcut }) && Fingerprint(document, shortcut, unrelated) == originals,
                "外部界面移出分组修改了原文件或快捷方式");

            var firstBox = store.State.Boxes.Single(box => box.Tabs.Any(tab => tab.Id == source));
            var secondBox = store.State.Boxes.Single(box => box.Tabs.Any(tab => tab.Id == destination));
            store.Change(next => next.MoveTab(secondBox.Id, destination, firstBox.Id));
            publish();
            request.Revision = LocalJson.Read<GridRenderState>(stateFile).Revision;
            request.Id = Guid.NewGuid().ToString("N");
            request.Action = "activate";
            request.Box = firstBox.Id;
            request.Tab = source;
            request.Paths = null;
            LocalJson.Write(requestFile, request);
            Require(renderer.ApplyRequest(store) && new GridStore(root).State.Boxes.Single().Active == source,
                "界面未保存当前标签");
            publish();
            request.Revision = LocalJson.Read<GridRenderState>(stateFile).Revision;
            request.Id = Guid.NewGuid().ToString("N");
            request.Action = "collapse";
            request.Collapsed = true;
            LocalJson.Write(requestFile, request);
            Require(renderer.ApplyRequest(store) && new GridStore(root).State.Boxes.Single().Collapsed,
                "界面未保存折叠状态");
            publish();
            request.Revision = LocalJson.Read<GridRenderState>(stateFile).Revision;
            request.Id = Guid.NewGuid().ToString("N");
            request.Action = "collapse-mode";
            request.CollapseMode = GridBox.Expanded;
            LocalJson.Write(requestFile, request);
            Require(renderer.ApplyRequest(store) && new GridStore(root).State.Boxes.Single().DisplayMode == GridBox.Expanded &&
                !new GridStore(root).State.Boxes.Single().Collapsed, "界面未保存始终展开模式");
            publish();
            request.Revision = LocalJson.Read<GridRenderState>(stateFile).Revision;
            request.Id = Guid.NewGuid().ToString("N");
            request.Action = "position";
            request.Bounds = new GridRenderBounds { X = -1480, Y = 75, Width = 450, Height = 375 };
            LocalJson.Write(requestFile, request);
            Require(renderer.ApplyRequest(store, "mixed", (box, rect) =>
            {
                Require(box.Id == firstBox.Id && rect == new Rectangle(-1480, 75, 450, 375),
                    "格子没有保留副屏的实际坐标");
                return new GridPosition { Monitor = "second", X = 0.2, Y = 0.1, Width = 300, Height = 250 };
            }), "界面位置请求未生效");
            var placed = new GridStore(root).State.Position("mixed", firstBox.Id, new[] { "second" });
            Require(placed != null && placed.Monitor == "second" && placed.Width == 300 && placed.Height == 250,
                "界面的位置和缩放尺寸没有保存到对应屏幕组合");
            Require(new GridStore(root).State.Boxes.Single().DisplayMode == GridBox.Expanded,
                "保存位置覆盖了展开方式");
            publish();
            request.Revision = LocalJson.Read<GridRenderState>(stateFile).Revision;
            request.Id = Guid.NewGuid().ToString("N");
            request.Action = "detach";
            request.Tab = destination;
            LocalJson.Write(requestFile, request);
            Require(renderer.ApplyRequest(store) && new GridStore(root).State.Boxes.Count == 2 &&
                Fingerprint(document, shortcut, unrelated) == originals, "拆出标签时没有保留路径或分组");
            publish();
            var detached = store.State.Boxes.Single(box => box.Id != firstBox.Id);
            request.Revision = LocalJson.Read<GridRenderState>(stateFile).Revision;
            request.Id = Guid.NewGuid().ToString("N");
            request.Action = "merge";
            request.Box = detached.Id;
            request.Target = firstBox.Id;
            LocalJson.Write(requestFile, request);
            Require(renderer.ApplyRequest(store), "界面合并格子请求未生效");
            var merged = new GridStore(root).State;
            Require(merged.Boxes.Single().Tabs.Select(tab => tab.Id).SequenceEqual(new[] { source, destination }) &&
                merged.Boxes[0].Active == source && merged.Position("mixed", firstBox.Id, new[] { "second" }).Monitor == "second",
                "合组没有保留目标位置、当前标签或原标签顺序");
            publish();
            request.Revision = LocalJson.Read<GridRenderState>(stateFile).Revision;
            request.Id = Guid.NewGuid().ToString("N");
            request.Action = "reorder";
            request.Box = firstBox.Id;
            request.Tab = destination;
            request.Target = source;
            request.ExpectedTabs = new[] { source, destination };
            LocalJson.Write(requestFile, request);
            Require(renderer.ApplyRequest(store) &&
                new GridStore(root).State.Boxes.Single().Tabs.Select(tab => tab.Id).SequenceEqual(new[] { destination, source }) &&
                store.State.Boxes[0].Active == source && Fingerprint(document, shortcut, unrelated) == originals,
                "保存标签顺序时修改了当前标签、文件或快捷方式");
            publish();
            request.Revision = LocalJson.Read<GridRenderState>(stateFile).Revision;
            request.Id = Guid.NewGuid().ToString("N");
            LocalJson.Write(requestFile, request);
            Require(!renderer.ApplyRequest(store) && LocalJson.Read<GridRenderState>(stateFile).Error != null,
                "标签顺序变化后仍接受了旧拖放事件");
            Action<string, string> edit = (setting, value) =>
            {
                request.Revision = LocalJson.Read<GridRenderState>(stateFile).Revision;
                request.Id = Guid.NewGuid().ToString("N");
                request.Action = "edit";
                request.Box = firstBox.Id;
                request.Tab = destination;
                request.Setting = setting;
                request.Value = value;
                LocalJson.Write(requestFile, request);
                Require(renderer.ApplyRequest(store), "格子菜单设置未保存：" + setting);
                publish();
            };
            edit("name", "新标签名称");
            edit("list", "true");
            edit("stack", "true");
            edit("sort", "name");
            edit("sort", "name");
            edit("folder-open", "Embedded");
            edit("position-lock", "true");
            edit("size-lock", "false");
            edit("direction", "Up");
            edit("group-navigation", "Stack");
            edit("group-title", "TextOnly");
            edit("group-wheel", "false");
            edit("group-hover", "true");
            var updatedBox = new GridStore(root).State.Boxes.Single();
            var updatedTab = updatedBox.Tabs.Single(tab => tab.Id == destination);
            Require(updatedTab.Name == "新标签名称" && updatedTab.List && updatedTab.StackByType &&
                updatedTab.Sort == "name" && updatedTab.Descending && updatedTab.FolderOpen == "Embedded" &&
                updatedBox.LocksPosition && !updatedBox.LocksSize && updatedBox.ExpansionDirection == "Up" &&
                updatedBox.GroupNavigation == "Stack" && updatedBox.GroupTitleDisplay == "TextOnly" &&
                updatedBox.GroupWheel == false && updatedBox.GroupHover,
                "格子菜单设置重启后不一致");
            request.Revision = LocalJson.Read<GridRenderState>(stateFile).Revision;
            request.Id = Guid.NewGuid().ToString("N");
            request.Action = "group";
            request.Paths = new[] { document };
            request.DisplayOrder = new[] { shortcut };
            request.Before = shortcut;
            LocalJson.Write(requestFile, request);
            Require(renderer.ApplyRequest(store) && new GridStore(root).State.Tabs()
                .Single(tab => tab.Id == destination).Items.SequenceEqual(new[] { document, shortcut }) &&
                new GridStore(root).State.Tabs().Single(tab => tab.Id == destination).Sort == "manual",
                "从自动排序拖放后没有保存可见插入位置和手动顺序");
            publish();
            request.Revision = LocalJson.Read<GridRenderState>(stateFile).Revision;
            request.Id = Guid.NewGuid().ToString("N");
            request.Action = "position";
            LocalJson.Write(requestFile, request);
            Func<GridBox, Rectangle, GridPosition> locate = (box, rect) =>
                new GridPosition { Monitor = "second", X = 0.2, Y = 0.1, Width = 350, Height = 220 };
            Require(!renderer.ApplyRequest(store, "mixed", locate) && LocalJson.Read<GridRenderState>(stateFile).Error != null,
                "锁定位置后仍能拖动");
            request.Id = Guid.NewGuid().ToString("N");
            request.Action = "resize";
            request.CapsuleWidth = 170;
            LocalJson.Write(requestFile, request);
            Require(renderer.ApplyRequest(store, "mixed", locate) && new GridStore(root).State.Boxes.Single().CapsuleWidth == 170,
                "锁定位置影响了调整大小，或未保存胶囊宽度");
            publish();
            edit("reset-width", null);
            edit("hidden", "true");
            Require(new GridStore(root).State.Boxes.Single().Hidden &&
                !new GridStore(root).State.Boxes.Single().CapsuleWidth.HasValue &&
                Fingerprint(document, shortcut, unrelated) == originals, "隐藏格子、恢复宽度或原文件保护失败");
        }
    }

    private static void LaunchPrivateDesktop(string root, string package = null, bool winui = false)
    {
        string name = "SoulDeskGridTest-" + Guid.NewGuid().ToString("N");
        IntPtr privateDesktop = CreateDesktop(name, IntPtr.Zero, IntPtr.Zero, 0, 0xC3, IntPtr.Zero);
        Require(privateDesktop != IntPtr.Zero, "无法创建隐藏的格子测试桌面 " + Marshal.GetLastWin32Error());
        try
        {
            string executable = System.Reflection.Assembly.GetExecutingAssembly().Location;
            var startup = new StartupInfo { Size = Marshal.SizeOf(typeof(StartupInfo)), Desktop = @"WinSta0\" + name };
            ProcessInfo process;
            string command = "\"" + executable + "\" --surface-child \"" + root + "\" " + name;
            if (winui) command += " --winui";
            if (package != null) command += " \"" + package + "\"";
            Require(CreateProcess(executable, new StringBuilder(command),
                IntPtr.Zero, IntPtr.Zero, false, 0, IntPtr.Zero, Path.GetDirectoryName(executable), ref startup, out process),
                "无法在隐藏桌面启动测试 " + Marshal.GetLastWin32Error());
            try
            {
                uint status;
                string failure = Path.Combine(root, "surface-error.txt");
                Require(WaitForSingleObject(process.Process, winui ? 68000u : 40000u) == 0 &&
                    GetExitCodeProcess(process.Process, out status) && status == 0 && File.Exists(Path.Combine(root, "surface-pass.json")),
                    "隐藏桌面验证失败" + (File.Exists(failure) ? " · " + File.ReadAllText(failure) : ""));
            }
            finally { CloseHandle(process.Thread); CloseHandle(process.Process); }
        }
        finally { CloseDesktop(privateDesktop); }
    }

    private static void VerifyPreviewPackage(string executable, string root)
    {
        Require(File.Exists(Path.Combine(root, "isolated-test-data.marker")), "缺少体验包的隔离标记");
        Require(!File.Exists(Path.Combine(root, "layout.json")), "需要全新解压的体验目录");
        var fixtureFiles = Directory.GetFiles(Path.Combine(root, "fixtures"), "*", SearchOption.AllDirectories);
        string originals = Fingerprint(fixtureFiles);
        using (var refused = Start(executable, ""))
            Require(refused.WaitForExit(5000) && refused.ExitCode == 2, "体验包允许绕过隔离参数");
        string script = Path.Combine(Path.GetDirectoryName(executable), "启动桌面格子体验.cmd");
        string arguments = "/d /c \"\"" + script + "\"\"";
        using (var launcher = Start(Path.Combine(Environment.SystemDirectory, "cmd.exe"), arguments))
            Require(launcher.WaitForExit(5000) && launcher.ExitCode == 0, "体验启动器未成功返回");
        int engineId = 0;
        var project = FindPreviewForm("项目文件 · SoulDesk", out engineId);
        int mappedId;
        var mapped = FindPreviewForm("资料目录 · SoulDesk", out mappedId);
        Require(engineId == mappedId, "两个示例格子未在同一体验进程显示");
        string assemblyPath;
        using (var engine = Process.GetProcessById(engineId))
        {
            Require(engine.Handle != IntPtr.Zero, "无法跟踪体验引擎的真实退出状态");
            assemblyPath = engine.MainModule.FileName;
            try
            {
                using (var hash = SHA256.Create())
                {
                    string sum = BitConverter.ToString(hash.ComputeHash(File.ReadAllBytes(assemblyPath))).Replace("-", "");
                    Require(sum.Equals(Path.GetFileNameWithoutExtension(assemblyPath), StringComparison.OrdinalIgnoreCase),
                        "实际启动的格子引擎与包内指纹不一致");
                }
                var saved = new GridStore(root).State;
                Require(saved.Boxes.Count == 2 && saved.Boxes[0].Current().Items.Count >= 2 && saved.Boxes[1].Collapsed,
                    "体验包没有初始化展开格子和折叠胶囊");
                Require(Fingerprint(fixtureFiles) == originals && new Recovery(root).Current() == null &&
                    !Directory.Exists(Path.Combine(root, "webview")) && !File.Exists(Path.Combine(root, "workbench.sqlite")),
                    "体验启动触碰了样本文件、Explorer 交接或组件宿主");
                SavePreviewImage(root, project, mapped);
                using (var launcher = Start(Path.Combine(Environment.SystemDirectory, "cmd.exe"), arguments))
                    Require(launcher.WaitForExit(5000) && launcher.ExitCode == 0, "再次打开体验版失败");
                int managerId;
                var manager = FindPreviewForm("SoulDesk · 桌面格子", out managerId);
                Require(managerId == engineId, "再次运行体验启动器创建了重复实例");
                Require(PostMessage(manager, 0x0112, new IntPtr(0xF060), IntPtr.Zero) && engine.WaitForExit(6500),
                    "关闭体验管理窗口时引擎未正常退出");
                Require(engine.ExitCode == 0 && !File.Exists(Path.Combine(root, "last-error.json")), "退出体验时记录了异常");
            }
            finally
            {
                if (!engine.HasExited && assemblyPath.StartsWith(Path.Combine(root, "bin") + "\\", StringComparison.OrdinalIgnoreCase))
                    engine.Kill();
            }
        }
    }

    private static IntPtr FindPreviewForm(string title, out int process)
    {
        IntPtr found = IntPtr.Zero;
        uint owner = 0;
        Wait(() =>
        {
            Native.EnumWindows((handle, _) =>
            {
                if (!Native.IsWindowVisible(handle) || Title(handle) != title)
                    return true;
                found = handle;
                Native.GetWindowThreadProcessId(handle, out owner);
                return false;
            }, IntPtr.Zero);
            return found != IntPtr.Zero;
        }, 9000, "体验包没有显示 " + title);
        process = (int)owner;
        return found;
    }

    private static void SavePreviewImage(string root, IntPtr project, IntPtr mapped)
    {
        // 用独立桌面的两个格子窗口渲染预览，截图不读取主桌面。
        using (var image = new Bitmap(390, 310))
        using (var graphics = Graphics.FromImage(image))
        {
            using (var background = new System.Drawing.Drawing2D.LinearGradientBrush(
                new Rectangle(Point.Empty, image.Size), Color.FromArgb(45, 67, 75), Color.FromArgb(16, 30, 38), 70f))
                graphics.FillRectangle(background, 0, 0, image.Width, image.Height);
            DrawPreviewForm(graphics, project, new Point(26, 23), false);
            DrawPreviewForm(graphics, mapped, new Point(26, 242), true);
            image.Save(Path.Combine(root, "grid-preview.png"));
        }
    }

    private static void DrawPreviewForm(Graphics output, IntPtr window, Point at, bool capsule)
    {
        WindowRect bounds;
        if (!GetWindowRect(window, out bounds)) return;
        int width = bounds.Right - bounds.Left, height = bounds.Bottom - bounds.Top;
        if (width < 2 || height < 2) return;
        using (var image = new Bitmap(width, height))
        using (var graphics = Graphics.FromImage(image))
        {
            IntPtr dc = graphics.GetHdc();
            bool rendered;
            try { rendered = PrintWindow(window, dc, 0x2); }
            finally { graphics.ReleaseHdc(dc); }
            Require(rendered, "未能在隔离桌面绘制格子");
            int displayWidth = Math.Min(width, 335);
            var target = new Rectangle(at, new Size(displayWidth, height * displayWidth / width));
            var saved = output.Save();
            using (var mask = GridLook.Rounded(target, capsule ? target.Height / 2 : 14))
                output.SetClip(mask);
            output.DrawImage(image, target);
            output.Restore(saved);
        }
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct WindowRect { internal int Left, Top, Right, Bottom; }

    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr window, out WindowRect bounds);
    [DllImport("user32.dll")] private static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("dwmapi.dll")] private static extern int DwmGetWindowAttribute(IntPtr window, int attribute, out int value, int size);
    [DllImport("user32.dll")] private static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);

    private static void VerifyOrganization(string root)
    {
        string desktop = Path.Combine(root, "desktop");
        Directory.CreateDirectory(desktop);
        string file = Path.Combine(desktop, "同盘测试.txt");
        File.WriteAllText(file, "original");
        string digest = Fingerprint(file);
        var org = new GridOrganization(root);
        var tab = new GridTab { Name = "凭据测试" };
        var change = new KeyValuePair<DesktopItem, GridTab>(DesktopItem.FromPath(file), tab);
        var moved = org.Plan(new[] { change });
        org.Move(moved);
        Require(!File.Exists(file) && File.Exists(moved[0].Destination) && org.Read().Single().Status == "moved",
            "同盘搬移和历史登记失败");
        File.WriteAllText(file, "collision");
        bool blocked = false;
        try { org.Restore(moved[0].Id); }
        catch (InvalidOperationException) { blocked = true; }
        Require(blocked && File.ReadAllText(file) == "collision" && File.ReadAllText(moved[0].Destination) == "original",
            "冲突时恢复操作覆盖了文件");
        File.Delete(file);
        var restored = org.Restore(moved[0].Id);
        Require(restored.Status == "restored" && Fingerprint(file) == digest && !File.Exists(restored.Destination),
            "恢复未保留原内容或留下副本");

        string folder = Path.Combine(desktop, "文件夹");
        Directory.CreateDirectory(folder);
        File.WriteAllText(Path.Combine(folder, "子文件.txt"), "folder file");
        var folderMove = org.Plan(new[] { new KeyValuePair<DesktopItem, GridTab>(DesktopItem.FromPath(folder), tab) });
        org.Move(folderMove);
        Require(!Directory.Exists(folder) && File.Exists(Path.Combine(folderMove[0].Destination, "子文件.txt")),
            "文件夹搬移没有保持目录结构");
        var saved = org.Read();
        saved.Single(item => item.Id == folderMove[0].Id).Status = "moving";
        LocalJson.Write(Path.Combine(root, "organization-history.json"), saved);
        org.Recover();
        Require(org.Read().Single(item => item.Id == folderMove[0].Id).Status == "moved", "重启未核对未完成的搬移");
        Require(org.Restore(folderMove[0].Id).Status == "restored" && Directory.Exists(folder), "文件夹未能恢复");
    }

    private static void VerifyDesktopLabel()
    {
        using (var bitmap = new Bitmap(110, 42))
        using (var graphics = Graphics.FromImage(bitmap))
        using (var font = GridLook.Font(8.5f))
        {
            graphics.Clear(Color.Fuchsia);
            graphics.TextRenderingHint = System.Drawing.Text.TextRenderingHint.ClearTypeGridFit;
            GridFile.DrawDesktopLabel(graphics, "报告 sample.lnk", font, new Rectangle(0, 0, 109, 41));
            bool drewWhite = false;
            for (int y = 0; y < bitmap.Height; y++)
                for (int x = 0; x < bitmap.Width; x++)
                {
                    Color pixel = bitmap.GetPixel(x, y);
                    if (pixel.ToArgb() == Color.Fuchsia.ToArgb()) continue;
                    Require(pixel.R == pixel.G && pixel.G == pixel.B, "桌面文件标签与底色混色");
                    drewWhite |= pixel.ToArgb() == Color.White.ToArgb();
                }
            Require(drewWhite, "未绘制桌面文件标签");
        }
    }

    private static void VerifyWinUISurface(string root, string renderer)
    {
        Native.DpiAware();
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        string mapped = Path.Combine(root, "fixtures", "mapping");
        Directory.CreateDirectory(mapped);
        File.WriteAllText(Path.Combine(mapped, "mapped-grid-probe.txt"), "mapping probe");
        string left = Path.Combine(root, "fixtures", "left-stack.txt");
        string right = Path.Combine(root, "fixtures", "right-stack.txt");
        File.WriteAllText(left, "left stack");
        File.WriteAllText(right, "right stack");
        File.WriteAllText(Path.Combine(root, "isolated-test-data.marker"), "SoulDesk isolated test data");
        File.WriteAllText(Path.Combine(root, "test-reference-menu.request"), "isolated menu");
        var store = new GridStore(root);
        store.Change(state =>
        {
            var box = state.Add("隔离窗口", null);
            box.SetCollapseMode(GridBox.Expanded);
            state.Attach(box.Current().Id, new[] { left, right });
            var json = new JavaScriptSerializer();
            string key = "Manual:" + GridState.NewId();
            box.Current().StackSettings["FileStackMemberOverrides"] = json.Serialize(
                new Dictionary<string, string[]> { { key, new[] { left, right } } });
            box.Current().StackSettings["FileStackNameOverrides"] = json.Serialize(
                new Dictionary<string, string> { { key, "stack-probe" } });
            state.Add("目录映射", mapped).Current().FolderOpen = "Embedded";
        });
        using (var app = new GridApplication(root, true, null, false, renderer))
        {
            try
            {
                string session = Directory.GetDirectories(root, "deskbox-renderer-*").Single();
                string error = Path.Combine(session, "preview-error.txt");
                string ready = Path.Combine(session, "preview-ready.json");
                Wait(() => { Application.DoEvents(); return File.Exists(ready); },
                    43000, "WinUI 格子没有完成首帧", () => File.Exists(error) ? File.ReadAllText(error) : "");
                string launch = File.ReadAllText(ready);
                Require(launch.Contains("mapped-grid-probe.txt"), "WinUI 未加载映射目录内容");
                Require(launch.Contains("stack-probe"), "WinUI 未加载自定义堆栈内容");
                var probe = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(launch);
                var menu = (Dictionary<string, object>)probe["menu"];
                var commands = ((System.Collections.IEnumerable)menu["labels"]).Cast<string>().ToArray();
                Require(commands.Any(label => label.Contains("仅目录映射可用")) &&
                    commands.Any(label => label.Contains("文件格子设置")), "格子菜单缺少原版入口或上下文提示");
                VerifyWinUISettings(app, root);
                int owner;
                var window = FindPreviewForm("隔离窗口 · 桌面格子", out owner);
                WindowRect initial;
                Require(GetWindowRect(window, out initial) && initial.Bottom - initial.Top > 100, "始终展开的格子未展开");
                var moved = new Rectangle(initial.Left - 85, initial.Top + 90, initial.Right - initial.Left, initial.Bottom - initial.Top);
                app.Place(app.Store.State.Boxes[0], moved);
                Wait(() =>
                {
                    Application.DoEvents();
                    WindowRect actual;
                    return GetWindowRect(window, out actual) && Math.Abs(actual.Left - moved.Left) < 20 &&
                        Math.Abs(actual.Top - moved.Top) < 20 && actual.Bottom - actual.Top > 100;
                }, 6500, "位置同步重建了格子，或导致格子收起");
                app.Edit(state => state.Boxes[0].SetCollapseMode(GridBox.Click));
                Wait(() =>
                {
                    Application.DoEvents();
                    WindowRect actual;
                    return GetWindowRect(window, out actual) && actual.Bottom - actual.Top < 90;
                }, 6500, "切换为单击展开后未在原窗口收起");
                app.Edit(state => state.Boxes[0].SetCollapseMode(GridBox.Expanded));
                Wait(() =>
                {
                    Application.DoEvents();
                    WindowRect actual;
                    return GetWindowRect(window, out actual) && actual.Bottom - actual.Top > 100 &&
                        Math.Abs(actual.Left - moved.Left) < 20 && Math.Abs(actual.Top - moved.Top) < 20;
                }, 6500, "切回始终展开后没有回到拖动结束的位置");
                app.Edit(state =>
                {
                    state.Boxes[0].Current().Name = "原窗改名";
                    state.Boxes[0].Current().List = true;
                    state.Boxes[0].Current().StackByType = true;
                    state.Boxes[0].Current().ChangeSort("name");
                    state.Boxes[0].PositionLocked = true;
                    state.Boxes[0].ExpansionDirection = "Down";
                });
                Wait(() =>
                {
                    Application.DoEvents();
                    return Title(window) == "原窗改名 · 桌面格子";
                }, 6500, "格子改名后没有更新原有 WinUI 窗口");
                app.Edit(state =>
                {
                    state.Boxes[0].SetCollapseMode(GridBox.SystemDefault);
                    state.SetDefaultCollapseMode(GridBox.Click);
                });
                Wait(() =>
                {
                    Application.DoEvents();
                    WindowRect actual;
                    return GetWindowRect(window, out actual) && actual.Bottom - actual.Top < 90;
                }, 6500, "跟随默认的格子未应用单击展开模式");
                app.Edit(state => state.SetDefaultCollapseMode(GridBox.Expanded));
                Wait(() =>
                {
                    Application.DoEvents();
                    WindowRect actual;
                    return GetWindowRect(window, out actual) && actual.Bottom - actual.Top > 100;
                }, 6500, "更改默认展开方式后未在原窗口展开");
                app.Edit(state => state.Boxes[0].Hidden = true);
                Wait(() => { Application.DoEvents(); return !Native.IsWindow(window); }, 6500, "隐藏格子后仍显示旧窗口");
                app.Edit(state => state.Boxes[0].Hidden = false);
                int restoredOwner;
                var restored = FindPreviewForm("原窗改名 · 桌面格子", out restoredOwner);
                Require(restored != window && owner == restoredOwner, "重新显示格子启动了新的界面进程");
                Require(new Recovery(root).Current() == null, "隔离 WinUI 会话发起了 Explorer 交接");
            }
            finally { app.Stop(true); }
        }
    }

    private static void VerifyWinUISettings(GridApplication app, string root)
    {
        File.WriteAllText(Path.Combine(root, "test-reference-settings.request"), "isolated settings");
        Require(app.ShowSettings("CapsuleMode"), "设置窗口未启动");
        string session = Directory.GetDirectories(root, "deskbox-renderer-*").Single(directory =>
        {
            var state = LocalJson.TryRead<GridRenderState>(Path.Combine(directory, "state.json"));
            return state != null && state.SettingsOnly;
        });
        string ready = Path.Combine(session, "preview-ready.json");
        string error = Path.Combine(session, "preview-error.txt");
        Wait(() => { Application.DoEvents(); return File.Exists(ready); }, 30000, "设置窗口未完成首帧",
            () => File.Exists(error) ? File.ReadAllText(error) : "");
        var identity = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(File.ReadAllText(ready));
        var window = new IntPtr(Convert.ToInt64(identity["handle"]));
        Require(Native.IsWindowVisible(window), "设置窗口不可见");
        string reportFile = Path.Combine(session, "settings-test.json");
        Wait(() => { Application.DoEvents(); return File.Exists(reportFile); }, 12000, "WinUI 设置页验证未完成",
            () => File.Exists(error) ? File.ReadAllText(error) : "");
        var report = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(File.ReadAllText(reportFile));
        var categories = ((System.Collections.IEnumerable)report["categories"]).Cast<string>().ToArray();
        Require(categories.SequenceEqual(new[] { "常规", "外观", "胶囊模式", "文件格子", "功能格子", "快捷与交互", "诊断与维护", "关于" }),
            "设置分类与 DeskBox 固定版本不一致");
        Require((int)report["pages"] == 30 && (int)report["disabled"] > 0, "设置详情页或未接入提示缺失");
        Require(new GridStore(root).State.CapsuleBar == "Horizontal", "WinUI 胶囊排列开关未通过设置会话保存");
        Require(new GridStore(root).State.Preferences["ShowFileExtensions"] == "true",
            "WinUI 文件后缀名开关未通过设置会话保存");
        Require(app.ShowSettings("About"), "不能复用设置窗口");
        Wait(() => { Application.DoEvents(); return Native.IsWindowVisible(window) &&
            LocalJson.Read<GridRenderState>(Path.Combine(session, "state.json")).SettingsPage == "About"; },
            4000, "设置窗口路由切换失败");
        Require(PostMessage(window, 0x10, IntPtr.Zero, IntPtr.Zero), "无法关闭设置窗口");
        Wait(() => { Application.DoEvents(); return !Native.IsWindowVisible(window); }, 6500, "设置窗口关闭后仍可见");
    }

    private static void VerifySurfaces(string root)
    {
        Native.DpiAware();
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        string fixtures = Path.Combine(root, "fixtures");
        string mapped = Path.Combine(fixtures, "映射资料");
        Directory.CreateDirectory(mapped);
        string document = Path.Combine(fixtures, "隔离报告.txt");
        string dropFile = Path.Combine(fixtures, "拖放样例.txt");
        File.WriteAllText(document, "isolated fixture");
        File.WriteAllText(dropFile, "unchanged drop fixture");
        File.WriteAllText(Path.Combine(mapped, "目录样本.txt"), "mapped fixture");
        string gridId = null, mappedId = null, mappedTab = null;
        using (var app = new GridApplication(root, true, null))
        {
            app.Edit(state =>
            {
                var one = state.Add("项目格子", null);
                gridId = one.Id;
                state.Attach(one.Current().Id, new[] { document });
                var two = state.Add("映射格子", mapped);
                mappedId = two.Id;
                mappedTab = two.Current().Id;
            });
            Application.DoEvents();
            var grid = Application.OpenForms.OfType<GridSurface>().Single(surface => surface.Box.Id == gridId);
            int height = grid.ScreenBounds.Height;
            Require(Descendants(grid).OfType<GridFile>().Any(item => item.Text == "隔离报告.txt"), "格子中文件引用未渲染");
            var dropTarget = Descendants(grid).OfType<FlowLayoutPanel>().Single(panel => panel.AllowDrop);
            var dropData = new DataObject(DataFormats.FileDrop, new[] { dropFile });
            var effects = DragDropEffects.Link | DragDropEffects.Move;
            var drop = new DragEventArgs(dropData, 0, 0, 0, effects, DragDropEffects.None);
            var flags = System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic;
            typeof(Control).GetMethod("OnDragEnter", flags).Invoke(dropTarget, new object[] { drop });
            Require(drop.Effect == DragDropEffects.Link, "桌面文件拖放必须选择链接效果");
            typeof(Control).GetMethod("OnDragDrop", flags).Invoke(dropTarget, new object[] { drop });
            Application.DoEvents();
            grid = Application.OpenForms.OfType<GridSurface>().Single(surface => surface.Box.Id == gridId);
            Require(grid.Box.Current().Items.Contains(dropFile) && File.ReadAllText(dropFile) == "unchanged drop fixture",
                "拖放时未原地加入文件");
            var before = Descendants(grid).OfType<GridFile>().Single(item => item.Text == "隔离报告.txt");
            typeof(Control).GetMethod("OnDragEnter", flags).Invoke(before, new object[] { drop });
            typeof(Control).GetMethod("OnDragDrop", flags).Invoke(before, new object[] { drop });
            Application.DoEvents();
            grid = Application.OpenForms.OfType<GridSurface>().Single(surface => surface.Box.Id == gridId);
            Require(grid.Box.Current().Items.SequenceEqual(new[] { dropFile, document }), "拖到格子图标上没有保存排序");
            var originalBounds = grid.ScreenBounds;
            var menuButton = Descendants(grid).OfType<Button>().Single(button => button.AccessibleName == "格子菜单");
            menuButton.PerformClick();
            var menu = grid.ContextMenuStrip;
            Require(menu.Visible, "格子菜单未打开");
            menu.Close();
            Require(!menu.IsDisposed, "关闭格子菜单后菜单意外释放");
            menuButton.PerformClick();
            Require(grid.ContextMenuStrip == menu && menu.Visible, "格子菜单不能重复打开");
            menu.Items.OfType<ToolStripMenuItem>().Single(item => item.Text == "列表视图").PerformClick();
            Application.DoEvents();
            grid = Application.OpenForms.OfType<GridSurface>().Single(surface => surface.Box.Id == gridId);
            Require(grid.Box.Current().List && menu.IsDisposed, "菜单切换视图后菜单生命周期错误");
            Descendants(grid).OfType<Button>().Single(button => button.AccessibleName == "折叠格子").PerformClick();
            Application.DoEvents();
            grid = Application.OpenForms.OfType<GridSurface>().Single(surface => surface.Box.Id == gridId);
            Require(grid.Box.Collapsed && grid.ScreenBounds.Height < height && grid.ScreenBounds.Width < originalBounds.Width,
                "折叠后的界面没有收成短胶囊");
            Descendants(grid).OfType<Button>().Single(button => button.AccessibleName == "展开格子").PerformClick();
            Application.DoEvents();
            app.Edit(state => state.MoveTab(mappedId, mappedTab, gridId));
            Application.DoEvents();
            grid = Application.OpenForms.OfType<GridSurface>().Single();
            Require(Descendants(grid).OfType<Button>().Any(button => button.Text == "项目格子") &&
                Descendants(grid).OfType<GridFile>().Any(item => item.Text == "目录样本.txt"), "标签或映射内容未渲染");
            Descendants(grid).OfType<Button>().Single(button => button.Text == "项目格子").PerformClick();
            Application.DoEvents();
            grid = Application.OpenForms.OfType<GridSurface>().Single();
            Require(grid.Box.Current().Name == "项目格子" &&
                Descendants(grid).OfType<GridFile>().Any(item => item.Text == "拖放样例.txt"), "标签点击未切换格子内容");
            app.ShowSearch();
            var search = Application.OpenForms.OfType<GridSearch>().Single();
            Descendants(search).OfType<TextBox>().Single().Text = "隔离报告";
            Wait(() =>
            {
                Application.DoEvents();
                return Descendants(search).OfType<ListBox>().Single().Items.Count == 1;
            }, 5000, "全局搜索预览未返回隔离文件");
            search.Dispose();
            string output = Path.Combine(root, "backup.json");
            app.Export(output);
            app.Edit(state => state.RemoveBox(gridId));
            app.Import(output);
            Require(app.Store.State.Boxes.Single().Tabs.Count == 2 && File.ReadAllText(document) == "isolated fixture",
                "导入未恢复标签组或更改了测试项目");
            Require(new Recovery(root).Current() == null, "隔离预览发起了 Explorer 交接");
            var manager = Application.OpenForms.OfType<GridManager>().Single();
            VerifyManagerSettings(app, manager, root);
            using (var loose = new LooseSurface(app))
            {
                var item = DesktopItem.FromPath(document);
                item.ProjectedBounds = new Rectangle(5, 5, 90, 76);
                loose.SetItems(Point.Empty, new List<DesktopItem> { item });
                var icon = loose.Controls.OfType<GridFile>().Single();
                loose.SetItems(Point.Empty, new List<DesktopItem> { item });
                Require(loose.Controls.OfType<GridFile>().Single() == icon, "无关布局更新重建了图标");
                var added = DesktopItem.FromPath(dropFile);
                added.ProjectedBounds = new Rectangle(105, 5, 90, 76);
                loose.SetItems(Point.Empty, new List<DesktopItem> { added, item });
                Require(loose.Controls.Count == 2 && loose.Controls.Contains(icon), "增加图标时重建了已有图标");
            }
            Require(PostMessage(manager.Handle, 0x10, IntPtr.Zero, IntPtr.Zero), "无法发送测试关闭消息");
            Wait(() =>
            {
                Application.DoEvents();
                return !Application.OpenForms.OfType<GridManager>().Any();
            }, 5000, "隔离预览关闭后仍有管理窗口");
        }
    }

    private static void VerifyManagerSettings(GridApplication app, GridManager manager, string root)
    {
        var controls = Descendants(manager).ToArray();
        var buttons = controls.OfType<Button>().ToArray();
        buttons.Single(item => item.Text == "桌面整理").PerformClick();
        Require(buttons.Single(item => item.Text == "预览整理").Visible &&
            !buttons.Single(item => item.Text == "新建格子").Visible, "桌面整理未切换到独立分区");
        buttons.Single(item => item.Text == "设置").PerformClick();
        manager.Size = manager.MinimumSize;
        Application.DoEvents();
        var sections = controls.OfType<RadioButton>().ToDictionary(item => item.Text);
        Require(sections.Values.All(item => item.Visible && item.Parent.ClientRectangle.Contains(item.Bounds) &&
            TextRenderer.MeasureText(item.Text, item.Font, Size.Empty, TextFormatFlags.NoPadding).Width + 6 <= item.ClientSize.Width),
            "最小窗口宽度下设置分区标题被截断");
        var combos = controls.OfType<ComboBox>().ToDictionary(item => item.AccessibleName);
        var switches = controls.OfType<CheckBox>().ToDictionary(item => item.AccessibleName);
        var commit = typeof(ComboBox).GetMethod("OnSelectionChangeCommitted",
            System.Reflection.BindingFlags.Instance | System.Reflection.BindingFlags.NonPublic);
        Action<string, int> choose = (name, index) =>
        {
            var picker = combos[name];
            Require(picker.Visible && picker.Enabled, name + " 无法操作");
            picker.SelectedIndex = index;
            commit.Invoke(picker, new object[] { EventArgs.Empty });
        };

        Require(sections["布局与展开"].Checked && !combos["胶囊间距"].Enabled, "布局默认分区或胶囊间距联动错误");
        choose("默认展开方式", 2);
        choose("胶囊栏布局", 2);
        choose("胶囊间距", 14);
        switches["联动拖动"].Checked = false;
        var persisted = new GridStore(root).State;
        Require(persisted.DefaultCollapseMode == GridBox.Expanded && !persisted.CoordinatedMove &&
            persisted.CapsuleBar == "Vertical" && persisted.CapsuleSpacing == 14, "全局布局设置未正确保存");

        sections["单格设置"].Checked = true;
        choose("单格展开方式", 0);
        choose("显示器绑定", 1);
        switches["锁定位置"].Checked = true;
        switches["锁定大小"].Checked = true;
        var box = new GridStore(root).State.Boxes.Single();
        Require(box.BoundMonitor == "@primary" && box.DisplayMode == GridBox.SystemDefault &&
            box.LocksPosition && box.LocksSize, "单格设置未持久化");

        sections["整理与堆栈"].Checked = true;
        switches["自动归类"].Checked = true;
        switches["包含公共桌面"].Checked = true;
        persisted = new GridStore(root).State;
        Require(persisted.AutoSort && persisted.IncludePublic &&
            buttons.Single(item => item.Text == "编辑规则").Visible, "整理选项或堆栈编辑入口无效");

        sections["全局搜索"].Checked = true;
        Require(switches["Windows 搜索索引"].Visible && !switches["Windows 搜索索引"].Enabled &&
            buttons.Single(item => item.Text == "选择程序").Visible &&
            !buttons.Single(item => item.Text == "选择程序").Enabled, "搜索分区未保留隔离限制");
        sections["布局备份"].Checked = true;
        Require(buttons.Single(item => item.Text == "导出布局").Visible &&
            buttons.Single(item => item.Text == "导入布局").Visible, "设置未显示布局备份入口");
        app.Edit(state => state.SetDefaultCollapseMode(GridBox.Click));
        sections["布局与展开"].Checked = true;
        Require(combos["默认展开方式"].SelectedIndex == 1, "外部布局更新后设置显示旧值");
        app.Edit(state => state.RemoveBox(box.Id));
        sections["单格设置"].Checked = true;
        Require(!combos["当前格子"].Enabled && !combos["显示器绑定"].Enabled &&
            !switches["锁定位置"].Enabled, "无格子时还允许修改单格设置");
        buttons.Single(item => item.Text == "桌面格子").PerformClick();
        Require(combos["展开方式"].Visible && !combos["当前格子"].Visible, "返回格子列表时设置分区未隐藏");
    }

    private static IEnumerable<Control> Descendants(Control parent)
    {
        return parent.Controls.Cast<Control>().SelectMany(control => new[] { control }.Concat(Descendants(control)));
    }

    private static IntPtr FindProductWindow(int id)
    {
        IntPtr result = IntPtr.Zero;
        Native.EnumWindows((window, _) =>
        {
            uint pid;
            Native.GetWindowThreadProcessId(window, out pid);
            if (pid == id && Native.IsWindowVisible(window) && Title(window) == "SoulDesk · 桌面格子")
                result = window;
            return result == IntPtr.Zero;
        }, IntPtr.Zero);
        return result;
    }

    private static string ManagerContents(IntPtr manager)
    {
        var names = new List<string>();
        EnumChildWindows(manager, (window, _) =>
        {
            string name = Title(window, true);
            if (name.Length != 0) names.Add(name);
            return true;
        }, IntPtr.Zero);
        return string.Join(" ", names);
    }

    private static string Title(IntPtr window, bool control = false)
    {
        var buffer = new StringBuilder(256);
        if (control)
        {
            IntPtr result;
            ReadControlText(window, 0xD, new IntPtr(buffer.Capacity), buffer, 2, 350, out result);
        }
        else
            GetWindowText(window, buffer, buffer.Capacity);
        return buffer.ToString();
    }

    private static string Error(string root)
    {
        var files = new[] { "last-error.json", "guard-error.json", "attach-error.json", "lease.json", "ready.json", "done.json" }
            .Select(name => Path.Combine(root, name)).Where(File.Exists).ToArray();
        return files.Length == 0 ? "没有错误报告" : string.Join(" ", files.Select(path =>
            Path.GetFileName(path) + "=" + File.ReadAllText(path)));
    }

    private static List<DesktopItem> ReadIcons(string document, string shortcut)
    {
        var shell = ShellDesktop.Find();
        if (shell == null || !Native.IsWindowVisible(shell.View.Hwnd)) return null;
        try
        {
            var items = shell.Capture();
            var first = items.SingleOrDefault(item => References.Equal(document, item.Id));
            var next = items.SingleOrDefault(item => References.Equal(shortcut, item.Id));
            return first != null && next != null ? new List<DesktopItem> { first, next } : null;
        }
        catch (InvalidOperationException) { return null; }
        catch (ElementNotAvailableException) { return null; }
    }

    private static string Fingerprint(params string[] files)
    {
        using (var hash = SHA256.Create())
            return string.Join("|", files.Select(file => file + ":" + File.GetAttributes(file) + ":" +
                Convert.ToBase64String(hash.ComputeHash(File.ReadAllBytes(file)))));
    }

    private static Process Start(string file, string args)
    {
        return Process.Start(new ProcessStartInfo(file, args) { UseShellExecute = false, CreateNoWindow = true });
    }

    private static Point CenterIcon(Rectangle bounds)
    {
        return new Point(bounds.Left + bounds.Width / 2, bounds.Top + Math.Min(20, bounds.Height / 2));
    }

    private static int HitPid(Point point)
    {
        uint pid;
        Native.GetWindowThreadProcessId(WindowFromPoint(point), out pid);
        return (int)pid;
    }

    private static void DoubleClickAndCheck(Point point, string stage)
    {
        if (File.Exists(Clicked)) File.Delete(Clicked);
        SetCursorPos(point.X, point.Y);
        Thread.Sleep(180);
        for (int i = 0; i < 2; i++)
        {
            mouse_event(2, 0, 0, 0, UIntPtr.Zero);
            Thread.Sleep(45);
            mouse_event(4, 0, 0, 0, UIntPtr.Zero);
            Thread.Sleep(75);
        }
        Wait(() => File.Exists(Clicked), 35000, stage + "实际双击没有运行测试快捷方式 · " + point);
    }

    private static void Trace(string message)
    {
        Console.WriteLine(message);
        if (Directory.Exists(GuestRoot))
            File.AppendAllText(Path.Combine(GuestRoot, "grid-run.log"), message + Environment.NewLine);
        try
        {
            using (var serial = new SerialPort("COM1", 115200))
            {
                serial.Encoding = new UTF8Encoding(false);
                serial.Open();
                serial.WriteLine(message.Replace("\r", " ").Replace("\n", " "));
            }
        }
        catch (IOException) { }
        catch (UnauthorizedAccessException) { }
    }

    private static void Require(bool condition, string message)
    {
        if (!condition) throw new InvalidOperationException(message);
    }

    private static void Wait(Func<bool> condition, int timeout, string message, Func<string> details = null)
    {
        var watch = Stopwatch.StartNew();
        while (watch.ElapsedMilliseconds < timeout)
        {
            if (condition()) return;
            Thread.Sleep(130);
        }
        throw new TimeoutException(message + (details == null ? "" : " · " + details()));
    }

    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    private static extern int GetClassName(IntPtr window, StringBuilder name, int length);
    [DllImport("user32.dll")]
    private static extern IntPtr SendMessageTimeout(IntPtr window, uint message, UIntPtr wParam, IntPtr lParam,
        uint flags, uint timeout, out UIntPtr result);
    [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr window, Native.WindowVisitor visitor, IntPtr state);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetWindowText(IntPtr window, StringBuilder title, int length);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "SendMessageTimeoutW")]
    private static extern IntPtr ReadControlText(IntPtr window, uint message, IntPtr capacity, StringBuilder text, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll")] private static extern IntPtr GetParent(IntPtr window);
    [DllImport("user32.dll")] private static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll")] private static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extra);
    [DllImport("user32.dll")] private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] private static extern void mouse_event(uint flags, uint x, uint y, uint data, UIntPtr extra);
    [DllImport("user32.dll")] private static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("kernel32.dll")] private static extern IntPtr OpenThread(uint mask, bool inherit, uint thread);
    [DllImport("kernel32.dll")] private static extern uint SuspendThread(IntPtr thread);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
    [DllImport("kernel32.dll")] private static extern uint WaitForSingleObject(IntPtr handle, uint milliseconds);
    [DllImport("kernel32.dll")] private static extern bool GetExitCodeProcess(IntPtr process, out uint exit);
    [DllImport("kernel32.dll")] private static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] private static extern IntPtr GetThreadDesktop(uint thread);
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct StartupInfo
    {
        public int Size;
        public string Reserved, Desktop, Title;
        public uint X, Y, Width, Height, Rows, Columns, Fill, Flags;
        public ushort Show, ReservedLength;
        public IntPtr ReservedPointer, Input, Output, Error;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct ProcessInfo { public IntPtr Process, Thread; public int Id, ThreadId; }
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool CreateProcess(
        string application, StringBuilder command, IntPtr processAttributes, IntPtr threadAttributes, bool inherit, uint flags,
        IntPtr environment, string workingDirectory, ref StartupInfo startup, out ProcessInfo process);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern bool GetUserObjectInformation(IntPtr handle,
        int index, StringBuilder name, uint length, out uint required);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern IntPtr CreateDesktop(string name, IntPtr device,
        IntPtr mode, uint flags, uint access, IntPtr attributes);
    [DllImport("user32.dll")] private static extern bool CloseDesktop(IntPtr desktop);
}
