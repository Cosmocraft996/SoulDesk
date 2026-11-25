using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Windows.Forms;
using Timer = System.Windows.Forms.Timer;

namespace SoulDesk.DesktopGrid
{
    internal static class Program
    {
        [STAThread]
        private static int Main(string[] args)
        {
            if (args.Length < 2 || !References.IsLocalPath(args[1]))
                return 2;
            string root = Path.GetFullPath(args[1]);
            bool isolated = args.Contains("--isolated");
            bool preview = args.Contains("--demo");
            string renderer = null;
            int rendererIndex = Array.IndexOf(args, "--renderer-exe");
            if (rendererIndex >= 0)
            {
                if (args[0] != "--open" || isolated && !preview || rendererIndex + 1 >= args.Length ||
                    !References.IsLocalPath(args[rendererIndex + 1]))
                    return 2;
                renderer = Path.GetFullPath(args[rendererIndex + 1]);
                if (!File.Exists(renderer))
                    return 2;
            }
            if (preview && (!isolated || args[0] != "--open" ||
                !Path.GetFileName(root).StartsWith("desktop-integration-", StringComparison.Ordinal) ||
                !File.Exists(Path.Combine(root, "isolated-test-data.marker"))))
                return 2;
            try
            {
                Directory.CreateDirectory(root);
                var recovery = new Recovery(root);
                if (args[0] == "--guard" && args.Length == 3 && !isolated)
                {
                    using (var guard = new Mutex(false, "Local\\SoulDesk.DesktopGrid.Guard"))
                    {
                        if (!Acquire(guard, 3000))
                            return 3;
                        try { recovery.RunGuard(args[2]); }
                        finally { guard.ReleaseMutex(); }
                    }
                    return 0;
                }
                if (args[0] == "--restore")
                {
                    if (!isolated)
                        recovery.RequestRestore();
                    return 0;
                }
                if (args[0] != "--open")
                    return 2;
                using (var mutex = new Mutex(false, MutexName(root, "ui")))
                {
                    if (!Acquire(mutex, 0))
                    {
                        recovery.Signal("open", "");
                        return 0;
                    }
                    try
                    {
                        Native.DpiAware();
                        Application.EnableVisualStyles();
                        Application.SetCompatibleTextRenderingDefault(false);
                        Application.SetUnhandledExceptionMode(UnhandledExceptionMode.ThrowException);
                        int parent;
                        long started;
                        WindowIdentity host = null;
                        int index = Array.IndexOf(args, "--parent");
                        if (index >= 0 && index + 2 < args.Length && int.TryParse(args[index + 1], out parent) &&
                            long.TryParse(args[index + 2], out started))
                        {
                            if (started == 0)
                            {
                                using (var process = Process.GetProcessById(parent))
                                    started = process.StartTime.ToUniversalTime().Ticks;
                            }
                            host = new WindowIdentity { Pid = parent, Started = started,
                                Session = Process.GetCurrentProcess().SessionId };
                        }
                        using (var app = new GridApplication(root, isolated, host, preview, renderer))
                            Application.Run(app);
                    }
                    finally { mutex.ReleaseMutex(); }
                }
                return 0;
            }
            catch (Exception error)
            {
                try { LocalJson.Write(Path.Combine(root, "last-error.json"), new { message = error.Message }); }
                catch (IOException) { }
                if (args[0] == "--open")
                {
                    if (!isolated)
                    {
                        try { new Recovery(root).RequestRestore(); }
                        catch (IOException) { }
                        catch (InvalidOperationException) { }
                    }
                    MessageBox.Show(error.Message, "SoulDesk · 桌面格子", MessageBoxButtons.OK, MessageBoxIcon.Warning);
                }
                return 1;
            }
        }

        internal static Process Start(string mode, string root, string suffix)
        {
            string executable = mode == "--guard" ? Path.ChangeExtension(Application.ExecutablePath, ".guard.exe") :
                Application.ExecutablePath;
            return Process.Start(new ProcessStartInfo(executable,
                mode + " \"" + root.TrimEnd('\\') + "\"" + suffix) { UseShellExecute = false, CreateNoWindow = true });
        }

        private static string MutexName(string root, string purpose)
        {
            using (var hash = SHA256.Create())
                return "Local\\SoulDesk.DesktopGrid." + purpose + "." +
                    Convert.ToBase64String(hash.ComputeHash(Encoding.UTF8.GetBytes(root.ToUpperInvariant()))).Replace('/', '_');
        }

        private static bool Acquire(Mutex mutex, int milliseconds)
        {
            try { return mutex.WaitOne(milliseconds); }
            catch (AbandonedMutexException) { return true; }
        }
    }

    internal sealed class GridApplication : ApplicationContext
    {
        private static readonly TimeSpan FileSyncInterval = TimeSpan.FromSeconds(2);
        private readonly string root;
        private readonly Recovery recovery;
        private readonly WindowIdentity host;
        private readonly GridControlWindow owner;
        private readonly Timer timer;
        private readonly GridManager manager;
        private readonly string rendererExecutable;
        private GridRendererSession renderer;
        private readonly List<GridSurface> surfaces = new List<GridSurface>();
        private readonly Dictionary<string, LooseSurface> loose = new Dictionary<string, LooseSurface>();
        private readonly Dictionary<string, DateTime> observed = new Dictionary<string, DateTime>(StringComparer.OrdinalIgnoreCase);
        private readonly Dictionary<string, FileStamp> candidates = new Dictionary<string, FileStamp>(StringComparer.OrdinalIgnoreCase);
        private Dictionary<string, IconPosition> iconPositions = new Dictionary<string, IconPosition>(StringComparer.OrdinalIgnoreCase);
        private List<DesktopItem> icons = new List<DesktopItem>();
        private ShellDesktop shell;
        private Process guard;
        private GridLease lease;
        private Stopwatch preparing;
        private bool enabled, desired, exiting, waitingForHost, rebuilding, reattachOnDetach, threadExited;
        private bool organizing;
        private DateTime nextAttach, nextSync, reconnectUntil;
        private string topology;
        internal string Status { get; private set; }
        internal bool Isolated { get; private set; }
        internal bool IsPreview { get; private set; }
        internal bool Active { get { return enabled || desired || reattachOnDetach; } }
        internal bool Hotkey { get; private set; }
        internal bool IsOrganizing { get { return organizing; } }
        internal GridStore Store { get; private set; }
        internal GridOrganization Organization { get; private set; }

        internal GridApplication(string root, bool isolated, WindowIdentity host, bool preview = false, string rendererExecutable = null)
        {
            this.root = root;
            this.host = host;
            this.rendererExecutable = rendererExecutable;
            Isolated = isolated;
            IsPreview = preview;
            recovery = new Recovery(root);
            if (!isolated)
                recovery.RecoverAbandoned();
            bool firstPreview = preview && !File.Exists(Path.Combine(root, "layout.json"));
            Store = new GridStore(root);
            Organization = new GridOrganization(root);
            owner = new GridControlWindow(() => ShowSearch());
            Hotkey = !isolated && Native.RegisterHotKey(owner.Handle, 1, 0x4003, (uint)Keys.F);
            manager = new GridManager(this);
            if (isolated)
            {
                var fixtures = Path.Combine(root, "fixtures");
                Directory.CreateDirectory(fixtures);
                icons = Directory.GetFileSystemEntries(fixtures).Select(DesktopItem.FromPath).ToList();
                if (firstPreview)
                    SeedPreview(fixtures);
                enabled = true;
                Status = "隔离预览 · 使用测试目录";
                if (rendererExecutable != null)
                    renderer = new GridRendererSession(root);
                Render();
                if (renderer != null)
                    renderer.Start(rendererExecutable);
            }
            else
                Status = "Windows 桌面图标正在显示";
            recovery.Signal("open", "handled");
            timer = new Timer { Interval = 250 };
            timer.Tick += Tick;
            timer.Start();
            if (File.Exists(Path.Combine(root, "organization-history.json")))
                RecoverOrganization();
            if (!preview || Store.State.Boxes.Count == 0)
                ShowManager();
        }

        private void SeedPreview(string fixtures)
        {
            string mapped = Path.Combine(fixtures, "资料目录");
            var files = Directory.GetFiles(fixtures);
            var screen = Screen.PrimaryScreen;
            string monitorId = Native.MonitorId(screen);
            string profile = Topology();
            Store.Change(state =>
            {
                var first = state.Add("项目文件", null);
                state.Attach(first.Current().Id, files);
                state.Place(profile, first.Id, new GridPosition { Monitor = monitorId, X = .73, Y = .25, Width = 328, Height = 205 });
                if (Directory.Exists(mapped))
                {
                    var second = state.Add("资料目录", mapped);
                    second.Collapsed = true;
                    state.Place(profile, second.Id, new GridPosition { Monitor = monitorId, X = .73, Y = .66, Width = 328, Height = 184 });
                }
            });
        }

        internal void ShowManager()
        {
            manager.RefreshState();
            manager.Show();
            manager.WindowState = FormWindowState.Normal;
            manager.Activate();
        }

        internal void ShowSearch() { new GridSearch(this).Show(); }

        internal void Toggle()
        {
            if (Isolated)
            {
                enabled = !enabled;
                Status = enabled ? "隔离预览 · 使用测试目录" : "隔离预览已暂停";
                Render();
                manager.RefreshState();
                return;
            }
            if (desired || lease != null)
                Stop(false);
            else
            {
                iconPositions.Clear();
                desired = true;
                nextAttach = DateTime.MinValue;
                reconnectUntil = DateTime.MinValue;
                Attach();
            }
        }

        internal void Stop(bool exit)
        {
            exiting |= exit;
            desired = false;
            enabled = false;
            reattachOnDetach = false;
            reconnectUntil = DateTime.MinValue;
            if (lease == null)
            {
                ClearSurfaces();
                if (!Isolated) CloseRenderer();
                preparing = null;
                shell = null;
                Status = "Windows 桌面图标正在显示";
                if (exiting)
                {
                    ExitThread();
                    return;
                }
            }
            else
            {
                recovery.Signal("stop", lease.Token);
                Status = "正在交还 Windows 桌面图标";
            }
            manager.RefreshState();
        }

        private void Attach()
        {
            var candidate = ShellDesktop.Find();
            if (candidate == null || !Native.IsWindowVisible(candidate.View.Hwnd))
            {
                nextAttach = DateTime.UtcNow.AddSeconds(2);
                Status = "等待可见的 Windows 桌面图标";
                manager.RefreshState();
                return;
            }
            try
            {
                var snapshot = candidate.Capture();
                if (snapshot.Count == 0 && ShellDesktop.Files().Any(item => IsOrganizable(item.Id)))
                {
                    nextAttach = DateTime.UtcNow.AddMilliseconds(900);
                    Status = "等待 Explorer 加载桌面项目";
                    manager.RefreshState();
                    return;
                }
                PositionDesktopIcons(snapshot);
                shell = candidate;
                icons = snapshot;
                if (observed.Count == 0)
                    foreach (var item in ShellDesktop.Files())
                        observed[item.Id] = DateTime.MinValue;
                topology = Topology();
                if (rendererExecutable != null)
                    renderer = new GridRendererSession(root, shell.Host);
                Render();
                if (renderer == null)
                    StartGuard();
                else
                {
                    renderer.Start(rendererExecutable);
                    preparing = Stopwatch.StartNew();
                    Status = "正在准备 WinUI 桌面格子";
                    manager.RefreshState();
                }
            }
            catch (Exception error)
            {
                FailAttach(error.Message);
            }
        }

        private void StartGuard()
        {
            var offer = new GridLease
            {
                Token = GridState.NewId(), Owner = WindowIdentity.Read(owner.Handle), Desktop = shell.View
            };
            if (renderer != null)
                renderer.AddToLease(offer);
            LocalJson.Write(recovery.PathFor("offer"), offer);
            guard = Program.Start("--guard", root, " " + offer.Token);
            lease = offer;
            preparing = Stopwatch.StartNew();
            Status = "正在接管 Windows 桌面图标";
            manager.RefreshState();
        }

        private void PositionDesktopIcons(List<DesktopItem> snapshot)
        {
            var displays = Screens().ToDictionary(screen => screen.DeviceName,
                screen => Native.MonitorId(screen) + "|" + screen.Bounds + "|" + Native.Scale(screen, 96));
            var connected = new HashSet<string>(displays.Values, StringComparer.Ordinal);
            var positions = new Dictionary<string, IconPosition>(StringComparer.OrdinalIgnoreCase);
            foreach (var icon in snapshot)
            {
                icon.ProjectedBounds = icon.NativeBounds;
                IconPosition saved;
                if (iconPositions.TryGetValue(icon.Id, out saved))
                {
                    // Explorer 重新排列时，保留仍处于原显示范围和缩放下的坐标。
                    if (connected.Contains(saved.Display))
                        icon.ProjectedBounds = saved.Bounds;
                }
                else
                {
                    string display;
                    if (!displays.TryGetValue(Screen.FromRectangle(icon.NativeBounds).DeviceName, out display))
                        continue;
                    saved = new IconPosition { Display = display, Bounds = icon.NativeBounds };
                }
                positions.Add(icon.Id, saved);
            }
            iconPositions = positions;

            // 新增图标或拔除显示器后，Explorer 的回退位置可能与保留位置冲突。
            if (snapshot.Any(icon => icon.ProjectedBounds != icon.NativeBounds && snapshot.Any(other =>
                other != icon && icon.ProjectedBounds.IntersectsWith(other.ProjectedBounds) &&
                !icon.NativeBounds.IntersectsWith(other.NativeBounds))))
                foreach (var icon in snapshot)
                    icon.ProjectedBounds = icon.NativeBounds;
        }

        private void FailAttach(string message)
        {
            desired = DateTime.UtcNow < reconnectUntil;
            nextAttach = DateTime.UtcNow.AddMilliseconds(1800);
            preparing = null;
            ClearSurfaces();
            CloseRenderer();
            shell = null;
            Status = message;
            LocalJson.Write(recovery.PathFor("attach-error"), new { message });
            manager.RefreshState();
        }

        private void Tick(object sender, EventArgs e)
        {
            if (renderer != null)
            {
                if (Isolated && renderer.Exited)
                {
                    Stop(true);
                    return;
                }
                if (renderer.ApplyRequest(Store, topology, PositionForRenderer,
                    (command, tab) => Post(() => manager.OpenGridAction(command, tab))))
                {
                    Render();
                    manager.RefreshState();
                }
            }
            if (!waitingForHost && host != null)
                using (var process = host.OpenProcess())
                    if (process == null)
                    {
                        waitingForHost = true;
                        Stop(true);
                        return;
                    }
            if (recovery.Signaled("open", ""))
            {
                recovery.Signal("open", "handled");
                ShowManager();
            }
            if (lease != null)
            {
                if ((desired || reattachOnDetach) && recovery.Signaled("restore", lease.Token))
                    Stop(false);
                bool sameDesktop = ShellDesktop.Matches(lease.Desktop);
                bool guardExited = guard.HasExited;
                if (!sameDesktop || guardExited)
                {
                    bool retry = !enabled && desired && DateTime.UtcNow < reconnectUntil;
                    bool shouldResume = reattachOnDetach || (desired && !sameDesktop) || retry;
                    if (sameDesktop)
                    {
                        try { Recovery.Restore(lease); }
                        catch (InvalidOperationException) { return; }
                    }
                    ClearSurfaces();
                    CloseRenderer();
                    preparing = null;
                    lease = null;
                    guard.Dispose();
                    guard = null;
                    shell = null;
                    enabled = false;
                    desired = shouldResume;
                    reattachOnDetach = false;
                    if (shouldResume && !retry)
                        reconnectUntil = DateTime.UtcNow.AddSeconds(90);
                    nextAttach = DateTime.UtcNow.AddMilliseconds(1600);
                    Status = shouldResume ? "正在等待 Explorer 恢复" : "Windows 桌面图标正在显示";
                    manager.RefreshState();
                    if (exiting)
                        ExitThread();
                    return;
                }
                if (desired && !enabled && recovery.Signaled("ready", lease.Token))
                {
                    enabled = true;
                    preparing = null;
                    reconnectUntil = DateTime.MinValue;
                    ShowSurfaces();
                    Status = "桌面格子已启用";
                    manager.RefreshState();
                }
                if (desired && preparing != null && preparing.ElapsedMilliseconds > 55000)
                {
                    if (DateTime.UtcNow < reconnectUntil)
                        Reattach();
                    else
                        Stop(false);
                }
            }
            else if (desired && renderer != null && preparing != null)
            {
                try
                {
                    if (!ShellDesktop.Matches(shell.View) || !Native.IsWindowVisible(shell.View.Hwnd))
                    {
                        reconnectUntil = DateTime.UtcNow.AddSeconds(90);
                        FailAttach("正在等待 Explorer 恢复");
                    }
                    else if (renderer.Exited)
                        FailAttach(renderer.Error ?? "WinUI 桌面格子启动失败");
                    else if (renderer.Ready())
                        StartGuard();
                    else if (preparing.ElapsedMilliseconds > 55000)
                        FailAttach("WinUI 桌面格子未在规定时间内完成首帧");
                }
                catch (Exception error) { FailAttach(error.Message); }
            }
            else if (desired && DateTime.UtcNow >= nextAttach)
                Attach();
            if (!enabled)
                return;
            string current = Topology();
            if (topology != current)
            {
                if (Isolated)
                    Render();
                else
                {
                    // 先让 Explorer 根据新拓扑布置图标，再读取新坐标。
                    Reattach();
                }
                return;
            }
            if (DateTime.UtcNow >= nextSync)
            {
                nextSync = DateTime.UtcNow + FileSyncInterval;
                SyncFiles();
            }
        }

        private void Reattach()
        {
            desired = false;
            reattachOnDetach = true;
            enabled = false;
            recovery.Signal("stop", lease.Token);
        }

        private void SyncFiles()
        {
            if (Isolated)
                return;
            try
            {
                var files = ShellDesktop.Files().ToDictionary(item => item.Id, StringComparer.OrdinalIgnoreCase);
                var added = files.Values.Where(item => !observed.ContainsKey(item.Id)).ToList();
                var gone = observed.Keys.Where(id => !files.ContainsKey(id)).ToList();
                if (added.Count + gone.Count == 0)
                {
                    if (AutoSort(files.Values))
                        Render();
                    foreach (var surface in surfaces)
                        surface.RefreshMappedFolder();
                    return;
                }
                foreach (var item in added)
                    observed[item.Id] = DateTime.UtcNow;
                foreach (string id in gone)
                {
                    observed.Remove(id);
                    candidates.Remove(id);
                }
                if (Store.State.AutoSort)
                    AutoSort(files.Values);
                // Explorer 在隐藏期间仍维护布局，重新交接以读取新增或更名图标的准确位置。
                Reattach();
            }
            catch (IOException) { }
            catch (UnauthorizedAccessException) { }
        }

        private bool AutoSort(IEnumerable<DesktopItem> files)
        {
            if (!Store.State.AutoSort)
                return false;
            var stable = new List<DesktopItem>();
            var now = DateTime.UtcNow;
            foreach (var item in files.Where(entry => observed[entry.Id] != DateTime.MinValue))
            {
                FileStamp current;
                try { current = FileStamp.Read(item.Id); }
                catch (IOException) { continue; }
                catch (UnauthorizedAccessException) { continue; }
                FileStamp former;
                if (!candidates.TryGetValue(item.Id, out former) || former.Changed != current.Changed || former.Size != current.Size)
                {
                    observed[item.Id] = now;
                    candidates[item.Id] = current;
                    continue;
                }
                if (now - observed[item.Id] >= TimeSpan.FromSeconds(4))
                    stable.Add(item);
            }
            var changes = Propose(stable);
            foreach (var item in stable)
            {
                observed[item.Id] = DateTime.MinValue;
                candidates.Remove(item.Id);
            }
            if (changes.Count == 0)
                return false;
            Store.Change(state => Apply(state, changes), true);
            return true;
        }

        internal List<DesktopItem> DesktopItems()
        {
            return Isolated || shell != null ? icons.ToList() : ShellDesktop.Files();
        }

        internal bool TestPath(string path)
        {
            return References.IsLocalPath(path) && (!Isolated || Path.GetFullPath(path)
                .StartsWith(Path.Combine(root, "fixtures") + "\\", StringComparison.OrdinalIgnoreCase));
        }

        internal List<KeyValuePair<DesktopItem, GridTab>> Propose(IEnumerable<DesktopItem> items, bool includeMapped = false)
        {
            var assigned = new HashSet<string>(Store.State.Tabs().SelectMany(tab => tab.Items), StringComparer.OrdinalIgnoreCase);
            var targets = Store.State.Tabs().Where(tab => (includeMapped || tab.Folder == null) &&
                !string.IsNullOrWhiteSpace(tab.Rules)).ToList();
            return items.Where(item => References.IsLocalPath(item.Id) && IsOrganizable(item.Id) && !assigned.Contains(item.Id) &&
                (!item.Shared || Store.State.IncludePublic)).Select(item =>
                    new KeyValuePair<DesktopItem, GridTab>(item, targets.FirstOrDefault(tab => GridState.Matches(tab, Path.GetFileName(item.Id)))))
                .Where(change => change.Value != null).ToList();
        }

        private static bool IsOrganizable(string path)
        {
            try
            {
                return (File.GetAttributes(path) & (FileAttributes.Hidden | FileAttributes.System | FileAttributes.Temporary)) == 0 &&
                    !path.EndsWith(".tmp", StringComparison.OrdinalIgnoreCase) &&
                    !path.EndsWith(".part", StringComparison.OrdinalIgnoreCase) &&
                    !path.EndsWith(".crdownload", StringComparison.OrdinalIgnoreCase);
            }
            catch (IOException) { return false; }
            catch (UnauthorizedAccessException) { return false; }
        }

        internal static void Apply(GridState state, IEnumerable<KeyValuePair<DesktopItem, GridTab>> changes)
        {
            foreach (var group in changes.GroupBy(change => change.Value.Id))
                state.Attach(group.Key, group.Select(change => change.Key.Id));
        }

        internal async void MoveOriginals(List<KeyValuePair<DesktopItem, GridTab>> changes)
        {
            if (organizing || changes.Count == 0) return;
            organizing = true;
            try
            {
                var plan = Organization.Plan(changes.Where(change => TestPath(change.Key.Id) &&
                    (change.Value.Folder == null || TestPath(change.Value.Folder))));
                if (plan.Length == 0) { Report("没有可搬移的本地项目。"); return; }
                await System.Threading.Tasks.Task.Run(() => Organization.Move(plan));
                if (threadExited) return;
                ReconcileOrganization(plan);
                Report("已搬移 " + plan.Count(item => item.Status == "moved") + " 项，失败 " +
                    plan.Count(item => item.Status == "failed") + " 项。详情见整理历史。");
            }
            catch (Exception error) { if (!threadExited) Report(error.Message); }
            finally { organizing = false; }
        }

        internal async System.Threading.Tasks.Task RestoreOrganizedItem(string id)
        {
            if (organizing) return;
            organizing = true;
            try
            {
                var item = await System.Threading.Tasks.Task.Run(() => Organization.Restore(id));
                if (threadExited) return;
                ReconcileOrganization(new[] { item });
                Report(item.Status == "restored" ? "已恢复到原路径。" : item.Error);
            }
            catch (Exception error) { if (!threadExited) Report(error.Message); throw; }
            finally { organizing = false; }
        }

        private async void RecoverOrganization()
        {
            organizing = true;
            try
            {
                var history = await System.Threading.Tasks.Task.Run(() => { Organization.Recover(); return Organization.Read(); });
                if (!threadExited) ReconcileOrganization(history);
            }
            catch (Exception error) { if (!threadExited) Report(error.Message); }
            finally { organizing = false; }
        }

        private void ReconcileOrganization(IEnumerable<OrganizedItem> records)
        {
            var receipts = records.Where(item => !item.LayoutSynced &&
                (item.Status == "moved" || item.Status == "restored")).ToArray();
            if (receipts.Length == 0) return;
            var next = LocalJson.Copy(Store.State);
            var tabs = next.Tabs().ToDictionary(tab => tab.Id);
            var refs = new HashSet<string>(tabs.Values.SelectMany(tab => tab.Items), StringComparer.OrdinalIgnoreCase);
            bool changed = false;
            foreach (var item in receipts)
            {
                if (item.Status == "restored")
                {
                    if (refs.Remove(item.Destination))
                    {
                        next.Release(new[] { item.Destination });
                        changed = true;
                    }
                    continue;
                }
                GridTab tab;
                if (item.Mapped || !tabs.TryGetValue(item.Tab, out tab) || tab.Folder != null ||
                    refs.Contains(item.Destination) || (!File.Exists(item.Destination) && !Directory.Exists(item.Destination)) ||
                    File.Exists(item.Source) || Directory.Exists(item.Source))
                    continue;
                next.Release(new[] { item.Source });
                next.Attach(item.Tab, new[] { item.Destination });
                refs.Add(item.Destination);
                changed = true;
            }
            if (changed) Store.Save(next, false);
            Organization.MarkLayoutSynced(receipts);
            Render();
            manager.RefreshState();
        }

        internal void Post(Action action)
        {
            if (!threadExited)
                manager.BeginInvoke(new Action(() => { if (!threadExited) action(); }));
        }

        internal ToolStripMenuItem MenuItem(string title, Action action)
        {
            return GridLook.Item(title, () => Post(action));
        }

        internal void Group(string tab, IEnumerable<string> paths, string before = null)
        {
            var valid = paths.Where(TestPath).ToArray();
            if (valid.Length != 0)
                Edit(state => state.Attach(tab, valid, before), true);
        }

        internal void Edit(Action<GridState> change, bool undo = false)
        {
            try
            {
                Store.Change(change, undo);
                Render();
                manager.RefreshState();
                if (IsPreview && Store.State.Boxes.Count == 0)
                    ShowManager();
            }
            catch (Exception error) { Report(error.Message); }
        }

        internal void Undo()
        {
            try
            {
                if (Store.Undo())
                {
                    Render();
                    manager.RefreshState();
                }
            }
            catch (Exception error) { Report(error.Message); }
        }

        internal void Report(string message)
        {
            Status = message;
            manager.RefreshState();
        }

        internal void Import(string filename)
        {
            try
            {
                var state = GridStore.ReadLayout(filename);
                state.EverythingCli = Store.State.EverythingCli;
                Store.Save(state, true);
                Render();
                manager.RefreshState();
            }
            catch (Exception error) { Report(error.Message); }
        }

        internal void Export(string filename)
        {
            try { Store.Export(filename); Report("已导出本地格子布局"); }
            catch (Exception error) { Report(error.Message); }
        }

        internal void Open(string path)
        {
            if (!(TestPath(path) || (!Isolated && path.StartsWith("::{") &&
                icons.Any(item => References.Equal(item.Id, path)))))
                return;
            try
            {
                var command = path.StartsWith("::{") ? new ProcessStartInfo("explorer.exe", "shell:" + path) : new ProcessStartInfo(path);
                command.UseShellExecute = true;
                Process.Start(command);
            }
            catch (Exception error) { Report(error.Message); }
        }

        internal void Reveal(string path)
        {
            if (!TestPath(path))
                return;
            try
            {
                Process.Start(new ProcessStartInfo("explorer.exe", "/select,\"" + path + "\"") { UseShellExecute = true });
            }
            catch (Exception error) { Report(error.Message); }
        }

        internal void Place(GridBox box, Rectangle rectangle)
        {
            var position = PositionForBounds(box, rectangle);
            Edit(state => state.Place(topology, box.Id, position));
        }

        private GridPosition PositionForRenderer(GridBox box, Rectangle rectangle)
        {
            if (topology != Topology())
                throw new InvalidOperationException("显示器已变更，请重新调整格子位置。");
            var screens = Screens();
            var screen = BoundScreen(box, screens) ?? Screen.FromPoint(new Point(rectangle.Left + rectangle.Width / 2,
                rectangle.Top + Math.Min(rectangle.Height / 2, 28)));
            var work = screen.WorkingArea;
            int width = Math.Max(144, Native.Unscale(screen, rectangle.Width));
            int height = Math.Max(80, Native.Unscale(screen, rectangle.Height));
            return new GridPosition {
                Monitor = Native.MonitorId(screen), Width = width, Height = height,
                X = Fraction(rectangle.Left - work.Left, work.Width - Native.Scale(screen, width)),
                Y = Fraction(rectangle.Top - work.Top, work.Height - Native.Scale(screen, height))
            };
        }

        private static Screen BoundScreen(GridBox box, Screen[] screens)
        {
            if (box.BoundMonitor == "@primary") return screens.FirstOrDefault(screen => screen.Primary);
            return box.BoundMonitor == null ? null : screens.FirstOrDefault(screen => Native.MonitorId(screen) == box.BoundMonitor);
        }

        internal GridDisplayInfo[] DisplayChoices()
        {
            return Screens().Select((screen, index) => new GridDisplayInfo {
                Id = Native.MonitorId(screen), Primary = screen.Primary,
                Name = "显示器 " + (index + 1) + " · " + screen.Bounds.Width + " × " + screen.Bounds.Height +
                    (screen.Primary ? "（主屏）" : "")
            }).ToArray();
        }

        private GridPosition PositionForBounds(GridBox box, Rectangle rectangle)
        {
            var screen = Screen.FromPoint(new Point(rectangle.Left + rectangle.Width / 2,
                rectangle.Top + Math.Min(rectangle.Height / 2, 28)));
            Rectangle work = screen.WorkingArea;
            int snap = Native.Scale(screen, 10);
            int x = Math.Max(work.Left, Math.Min(rectangle.Left, work.Right - rectangle.Width));
            int y = Math.Max(work.Top, Math.Min(rectangle.Top, work.Bottom - rectangle.Height));
            if (x - work.Left < snap) x = work.Left;
            if (y - work.Top < snap) y = work.Top;
            if (work.Right - x - rectangle.Width < snap) x = Math.Max(work.Left, work.Right - rectangle.Width);
            if (work.Bottom - y - rectangle.Height < snap) y = Math.Max(work.Top, work.Bottom - rectangle.Height);
            foreach (var surface in surfaces.Where(item => item.Box.Id != box.Id))
            {
                var other = surface.ScreenBounds;
                if (Math.Abs(x - other.Right) < snap && y < other.Bottom && y + rectangle.Height > other.Top)
                    x = other.Right;
                if (Math.Abs(y - other.Bottom) < snap && x < other.Right && x + rectangle.Width > other.Left)
                    y = other.Bottom;
            }
            x = Math.Max(work.Left, Math.Min(x, work.Right - rectangle.Width));
            y = Math.Max(work.Top, Math.Min(y, work.Bottom - rectangle.Height));
            var prior = Store.State.Position(topology, box.Id, Screens().Select(Native.MonitorId));
            int height = box.Collapsed ? prior == null ? 255 : prior.Height : Native.Unscale(screen, rectangle.Height);
            return new GridPosition {
                Monitor = Native.MonitorId(screen), X = Fraction(x - work.Left, work.Width - rectangle.Width),
                Y = Fraction(y - work.Top, work.Height - Native.Scale(screen, height)),
                Width = Math.Max(144, Native.Unscale(screen, rectangle.Width)), Height = Math.Max(80, height)
            };
        }

        private static double Fraction(int value, int total)
        {
            return Math.Max(0, Math.Min(1, (double)value / Math.Max(1, total)));
        }

        private static Screen[] Screens() { return Screen.AllScreens; }

        private static string Topology()
        {
            var topology = string.Join("|", Screens().Select(screen => Native.MonitorId(screen) + ":" + screen.Bounds +
                ":" + screen.WorkingArea + ":" + Native.Scale(screen, 100) + ":" + screen.Primary)
                .OrderBy(entry => entry, StringComparer.Ordinal));
            using (var hash = SHA256.Create())
                return Convert.ToBase64String(hash.ComputeHash(Encoding.UTF8.GetBytes(topology)));
        }

        private void Render()
        {
            if (rebuilding)
                return;
            rebuilding = true;
            try
            {
                foreach (var surface in surfaces) surface.Dispose();
                surfaces.Clear();
                topology = Topology();
                if (!enabled && shell == null)
                {
                    ClearSurfaces();
                    if (renderer != null)
                        renderer.Publish(new GridRenderBox[0], new string[0], false);
                    return;
                }
                var screens = Screens();
                IntPtr parent = shell == null ? IntPtr.Zero : shell.Parent;
                if (!Isolated)
                {
                    var grouped = new HashSet<string>(Store.State.Tabs().SelectMany(tab => tab.Items), StringComparer.OrdinalIgnoreCase);
                    foreach (var screen in screens)
                    {
                        var entries = icons.Where(item => !grouped.Contains(item.Id) &&
                            Screen.FromRectangle(item.ProjectedBounds).DeviceName == screen.DeviceName).ToList();
                        LooseSurface surface;
                        if (entries.Count == 0 && loose.TryGetValue(screen.DeviceName, out surface))
                        {
                            surface.Dispose();
                            loose.Remove(screen.DeviceName);
                        }
                        if (entries.Count == 0) continue;
                        if (!loose.TryGetValue(screen.DeviceName, out surface))
                        {
                            surface = new LooseSurface(this);
                            loose.Add(screen.DeviceName, surface);
                            surface.Prepare(parent, screen.Bounds);
                        }
                        surface.SetItems(screen.Bounds.Location, entries);
                    }
                }
                int index = 0;
                var projections = renderer == null ? null : new List<GridRenderBox>();
                foreach (var box in Store.State.Boxes.Where(item => !item.Hidden))
                {
                    var pos = Store.State.Position(topology, box.Id, screens.Select(Native.MonitorId));
                    var screen = BoundScreen(box, screens) ??
                        (pos == null ? null : screens.FirstOrDefault(item => Native.MonitorId(item) == pos.Monitor))
                        ?? Screen.PrimaryScreen;
                    var work = screen.WorkingArea;
                    int width = Math.Min(work.Width, Native.Scale(screen, pos == null ? 324 : pos.Width));
                    int height = Math.Min(work.Height, Native.Scale(screen, pos == null ? 255 : pos.Height));
                    int left = pos == null ? Math.Max(0, work.Width - width - Native.Scale(screen, 22 + 26 * (index % 8))) :
                        (int)Math.Round((work.Width - width) * pos.X);
                    int top = pos == null ? Math.Min(Math.Max(0, work.Height - height), Native.Scale(screen, 28 + 44 * (index % 8))) :
                        (int)Math.Round((work.Height - height) * pos.Y);
                    var bounds = new Rectangle(work.Left + left, work.Top + top, width, height);
                    if (projections != null)
                        projections.Add(GridRenderBox.At(box, bounds,
                            new Size(Native.Unscale(screen, width), Native.Unscale(screen, height))));
                    else
                    {
                        var surface = new GridSurface(this, box);
                        surfaces.Add(surface);
                        surface.PrepareGrid(parent, screen, bounds);
                    }
                    index++;
                }
                if (renderer != null)
                    renderer.Publish(projections, icons.Select(item => item.Id)
                        .Concat(Store.State.Tabs().SelectMany(tab => tab.Items)).Where(TestPath), enabled,
                        Store.State.DefaultCollapseMode, DisplayChoices(), Store.State.CapsuleBar,
                        Store.State.CapsuleSpacing, Store.State.CoordinatedMove, Store.State.StackRules.ToArray());
                if (enabled)
                    ShowSurfaces();
            }
            finally { rebuilding = false; }
        }

        private void ShowSurfaces()
        {
            if (renderer != null) renderer.SetVisible();
            foreach (var item in surfaces) item.ShowSurface();
            foreach (var item in loose.Values) item.ShowSurface();
        }

        private void ClearSurfaces()
        {
            foreach (var surface in surfaces) surface.Dispose();
            foreach (var surface in loose.Values) surface.Dispose();
            surfaces.Clear();
            loose.Clear();
        }

        private void CloseRenderer()
        {
            if (renderer == null)
                return;
            renderer.Dispose();
            renderer = null;
        }

        protected override void ExitThreadCore()
        {
            if (threadExited)
                return;
            threadExited = true;
            timer.Stop();
            ClearSurfaces();
            CloseRenderer();
            if (Hotkey) Native.UnregisterHotKey(owner.Handle, 1);
            owner.DestroyHandle();
            manager.Dispose();
            timer.Dispose();
            base.ExitThreadCore();
        }

        private struct IconPosition
        {
            internal string Display;
            internal Rectangle Bounds;
        }

        private struct FileStamp
        {
            internal DateTime Changed;
            internal long Size;

            internal static FileStamp Read(string path)
            {
                if (Directory.Exists(path))
                    return new FileStamp { Changed = Directory.GetLastWriteTimeUtc(path) };
                var file = new FileInfo(path);
                return new FileStamp { Changed = file.LastWriteTimeUtc, Size = file.Length };
            }
        }
    }

    internal sealed class GridControlWindow : NativeWindow
    {
        private readonly Action search;
        internal GridControlWindow(Action search)
        {
            this.search = search;
            CreateHandle(new CreateParams { Caption = "SoulDesk Desktop Grid Controller" });
        }
        protected override void WndProc(ref Message message)
        {
            if (message.Msg == 0x0312) search();
            base.WndProc(ref message);
        }
    }
}
