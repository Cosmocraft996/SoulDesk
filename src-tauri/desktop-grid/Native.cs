using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Windows.Automation;
using System.Windows.Forms;

namespace SoulDesk.DesktopGrid
{
    public sealed class WindowIdentity
    {
        public long Handle, Started;
        public int Pid, Session;
        internal IntPtr Hwnd { get { return new IntPtr(Handle); } }

        internal static WindowIdentity Read(IntPtr window)
        {
            if (window == IntPtr.Zero || !Native.IsWindow(window))
                return null;
            uint pid;
            Native.GetWindowThreadProcessId(window, out pid);
            try
            {
                using (var process = Process.GetProcessById((int)pid))
                    return new WindowIdentity
                    {
                        Handle = window.ToInt64(), Pid = (int)pid, Session = process.SessionId,
                        Started = process.StartTime.ToUniversalTime().Ticks
                    };
            }
            catch (ArgumentException) { return null; }
            catch (InvalidOperationException) { return null; }
            catch (System.ComponentModel.Win32Exception) { return null; }
        }

        internal bool Same(WindowIdentity other)
        {
            return other != null && Handle == other.Handle && Pid == other.Pid &&
                Session == other.Session && Started == other.Started;
        }

        internal Process OpenProcess()
        {
            try
            {
                var process = System.Diagnostics.Process.GetProcessById(Pid);
                if (process.SessionId == Session && process.StartTime.ToUniversalTime().Ticks == Started &&
                    !process.HasExited)
                    return process;
                process.Dispose();
            }
            catch (ArgumentException) { }
            catch (InvalidOperationException) { }
            catch (System.ComponentModel.Win32Exception) { }
            return null;
        }
    }

    internal sealed class ShellDesktop
    {
        internal IntPtr Parent;
        internal WindowIdentity View;
        internal WindowIdentity Host;

        internal static ShellDesktop Find()
        {
            var root = Native.GetShellWindow();
            if (root == IntPtr.Zero)
                return null;
            uint pid;
            Native.GetWindowThreadProcessId(root, out pid);
            ShellDesktop result = FromParent(root, pid);
            if (result == null)
                Native.EnumWindows((window, state) =>
                {
                    result = FromParent(window, pid);
                    return result == null;
                }, IntPtr.Zero);
            return result;
        }

        private static ShellDesktop FromParent(IntPtr parent, uint pid)
        {
            uint owner;
            Native.GetWindowThreadProcessId(parent, out owner);
            if (owner != pid)
                return null;
            IntPtr view = Native.FindWindowEx(parent, IntPtr.Zero, "SHELLDLL_DefView", null);
            if (view == IntPtr.Zero)
                return null;
            IntPtr icons = Native.FindWindowEx(view, IntPtr.Zero, "SysListView32", null);
            var identity = WindowIdentity.Read(icons);
            var layer = WindowIdentity.Read(view);
            if (identity == null || identity.Pid != pid || layer == null || layer.Pid != pid)
                return null;
            var host = WindowIdentity.Read(parent);
            if (host == null || host.Pid != pid)
                return null;
            return new ShellDesktop { Parent = parent, View = identity, Host = host };
        }

        internal static List<DesktopItem> Files()
        {
            var files = new List<DesktopItem>();
            foreach (var folder in new[] {
                Environment.GetFolderPath(Environment.SpecialFolder.DesktopDirectory),
                Environment.GetFolderPath(Environment.SpecialFolder.CommonDesktopDirectory)
            }.Where(value => !string.IsNullOrEmpty(value)).Distinct(StringComparer.OrdinalIgnoreCase))
            {
                if (!Directory.Exists(folder))
                    continue;
                foreach (string item in Directory.EnumerateFileSystemEntries(folder).Take(4097))
                {
                    if (!References.IsLocalPath(item) || Path.GetFileName(item).Equals("desktop.ini", StringComparison.OrdinalIgnoreCase))
                        continue;
                    var file = DesktopItem.FromPath(item);
                    file.Shared = References.Equal(folder, Environment.GetFolderPath(Environment.SpecialFolder.CommonDesktopDirectory));
                    files.Add(file);
                }
            }
            return files;
        }

        private static readonly string[] SystemItems =
        {
            "::{645FF040-5081-101B-9F08-00AA002F954E}", // 回收站
            "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}", // 此电脑
            "::{F02C1A0D-BE21-4350-88B0-7367FC96EF3C}", // 网络
            "::{59031A47-3F72-44A7-89C5-5595FE6B30EE}", // 用户文件
            "::{5399E694-6CE5-4D6C-8FCE-1D8870FDCBA0}", // 控制面板
            "::{031E4825-7B94-4DC3-B131-E946B44C8DD5}"  // 库
        };

        internal List<DesktopItem> Capture()
        {
            if (!Native.IsWindowVisible(View.Hwnd) || !Matches(View))
                throw new InvalidOperationException("请在 Windows 桌面图标可见时启用格子。");
            var files = Files();
            var items = new Dictionary<string, List<DesktopItem>>(StringComparer.OrdinalIgnoreCase);
            foreach (var file in files)
            {
                foreach (string label in new[] { file.Name, Path.GetFileNameWithoutExtension(file.Name),
                    Native.DisplayName(file.Id) }.Where(value => !string.IsNullOrEmpty(value)).Distinct(StringComparer.OrdinalIgnoreCase))
                    Index(items, label, file);
            }
            foreach (string id in SystemItems)
            {
                string label = Native.DisplayName(id);
                if (!string.IsNullOrEmpty(label))
                    Index(items, label, new DesktopItem { Id = id, Name = label });
            }

            var root = AutomationElement.FromHandle(View.Hwnd);
            var icons = root.FindAll(TreeScope.Children,
                new PropertyCondition(AutomationElement.ControlTypeProperty,
                    System.Windows.Automation.ControlType.ListItem));
            var result = new List<DesktopItem>();
            var used = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (AutomationElement item in icons)
            {
                var current = item.Current;
                var bounds = current.BoundingRectangle;
                if (current.IsOffscreen || bounds.IsEmpty || bounds.Width < 5 || bounds.Height < 5)
                    throw new InvalidOperationException("存在无法读取位置的 Windows 桌面图标。");
                List<DesktopItem> choices;
                if (!items.TryGetValue(current.Name, out choices))
                    throw new InvalidOperationException("无法识别 Windows 桌面图标「" + current.Name + "」。");
                var available = choices.Where(candidate => !used.Contains(candidate.Id)).ToList();
                var exact = available.Where(candidate => References.Equal(candidate.Name, current.Name)).ToList();
                if (exact.Count == 1)
                    available = exact;
                if (available.Count != 1)
                    throw new InvalidOperationException("Windows 桌面存在同名图标「" + current.Name + "」，暂不能接管。");
                var entry = available[0];
                used.Add(entry.Id);
                result.Add(new DesktopItem
                {
                    Id = entry.Id, Name = current.Name, Shared = entry.Shared,
                    NativeBounds = new Rectangle((int)bounds.X, (int)bounds.Y, (int)bounds.Width, (int)bounds.Height)
                });
            }
            if (!Matches(View) || !Native.IsWindowVisible(View.Hwnd))
                throw new InvalidOperationException("Explorer 桌面已重建，请重试。");
            return result;
        }

        private static void Index(Dictionary<string, List<DesktopItem>> items, string key, DesktopItem item)
        {
            List<DesktopItem> rows;
            if (!items.TryGetValue(key, out rows))
                items[key] = rows = new List<DesktopItem>();
            rows.Add(item);
        }

        internal static bool Matches(WindowIdentity expected)
        {
            var desktop = Find();
            return expected != null && desktop != null && expected.Same(desktop.View);
        }
    }

    internal static class Native
    {
        private const int Style = -16;
        private const int ExStyle = -20;
        private const int Child = 0x40000000;
        private const int Popup = unchecked((int)0x80000000);
        private const int ToolWindow = 0x80;
        private const int NoActivate = 0x08000000;
        private static readonly IntPtr Top = IntPtr.Zero;

        internal static void DpiAware()
        {
            try { SetProcessDpiAwarenessContext(new IntPtr(-4)); }
            catch (EntryPointNotFoundException) { SetProcessDpiAwareness(2); }
        }

        internal static IntPtr MatchWindowDpi(IntPtr parent)
        {
            if (parent == IntPtr.Zero)
                return IntPtr.Zero;
            try { return SetThreadDpiAwarenessContext(GetWindowDpiAwarenessContext(parent)); }
            catch (EntryPointNotFoundException) { return IntPtr.Zero; }
        }

        internal static void RestoreDpi(IntPtr context)
        {
            if (context != IntPtr.Zero)
                SetThreadDpiAwarenessContext(context);
        }

        internal static void Attach(IntPtr window, IntPtr parent, Rectangle screen)
        {
            if (GetParent(window) != parent)
            {
                SetWindowLong(window, Style, (GetWindowLong(window, Style) & ~Popup) | Child);
                SetWindowLong(window, ExStyle, GetWindowLong(window, ExStyle) | ToolWindow | NoActivate);
                SetParent(window, parent);
                if (GetParent(window) != parent)
                    throw new InvalidOperationException("Windows 桌面图层未能挂接。");
            }
            Place(window, parent, screen);
        }

        internal static void Place(IntPtr window, IntPtr parent, Rectangle screen, bool raise = false)
        {
            var origin = screen.Location;
            if (!ScreenToClient(parent, ref origin) ||
                !SetWindowPos(window, Top, origin.X, origin.Y, screen.Width, screen.Height, raise ? 0x0210u : 0x0214u))
                throw new InvalidOperationException("无法定位桌面格子。");
        }

        internal static void Backdrop(IntPtr window)
        {
            var corners = 2;
            DwmSetWindowAttribute(window, 33, ref corners, sizeof(int));
        }

        internal static void SetLayerOpacity(IntPtr window, int opacity)
        {
            if (opacity == 100)
                return;
            SetWindowLong(window, ExStyle, GetWindowLong(window, ExStyle) | 0x80000);
            SetLayeredWindowAttributes(window, 0, (byte)(opacity * 255 / 100), 2);
        }

        internal static void Visible(WindowIdentity window, bool visible, Func<bool> alive)
        {
            if (!window.Same(WindowIdentity.Read(window.Hwnd)) || !alive())
                throw new InvalidOperationException("Explorer 桌面窗口已失效。");
            if (IsWindowVisible(window.Hwnd) != visible &&
                !ShowWindowAsync(window.Hwnd, visible ? 4 : 0))
                throw new InvalidOperationException("Windows 桌面显隐请求失败。");
            var clock = Stopwatch.StartNew();
            while (IsWindowVisible(window.Hwnd) != visible)
            {
                if (!alive() || clock.ElapsedMilliseconds > 3500)
                    throw new InvalidOperationException("Windows 桌面没有完成显隐交接。");
                Thread.Sleep(25);
            }
            if (!alive())
                throw new InvalidOperationException("Explorer 桌面已重建。");
        }

        internal static bool Responsive(WindowIdentity window)
        {
            UIntPtr result;
            return window.Same(WindowIdentity.Read(window.Hwnd)) &&
                SendMessageTimeout(window.Hwnd, 0, UIntPtr.Zero, IntPtr.Zero, 2, 350, out result) != IntPtr.Zero;
        }

        internal static string MonitorId(Screen screen)
        {
            var info = new DisplayDevice { Size = Marshal.SizeOf(typeof(DisplayDevice)) };
            if (EnumDisplayDevices(screen.DeviceName, 0, ref info, 1) && !string.IsNullOrEmpty(info.Id))
                return info.Id.Length <= 256 ? info.Id : screen.DeviceName;
            return screen.DeviceName;
        }

        internal static int Scale(Screen screen, int units)
        {
            Point point = new Point(screen.Bounds.Left + 1, screen.Bounds.Top + 1);
            IntPtr monitor = MonitorFromPoint(point, 2);
            uint x, y;
            if (monitor != IntPtr.Zero && GetDpiForMonitor(monitor, 0, out x, out y) == 0)
                return (int)Math.Round(units * x / 96.0);
            return units;
        }

        internal static int Unscale(Screen screen, int pixels)
        {
            return (int)Math.Round(pixels * 96.0 / Scale(screen, 96));
        }

        internal static string DisplayName(string id)
        {
            FileInfo info;
            return FileInformation(id, 0x200, out info) ? info.Display : null;
        }

        internal static Bitmap ItemIcon(string id)
        {
            FileInfo info;
            if (FileInformation(id, 0x100, out info) && info.Icon != IntPtr.Zero)
            {
                try
                {
                    using (var icon = Icon.FromHandle(info.Icon))
                        return icon.ToBitmap();
                }
                finally { DestroyIcon(info.Icon); }
            }
            return SystemIcons.Application.ToBitmap();
        }

        private static bool FileInformation(string id, uint flags, out FileInfo info)
        {
            if (References.IsLocalPath(id))
                return SHGetFileInfo(id, 0, out info, (uint)Marshal.SizeOf(typeof(FileInfo)), flags) != UIntPtr.Zero;
            IntPtr pidl;
            uint attributes;
            info = default(FileInfo);
            if (SHParseDisplayName(id, IntPtr.Zero, out pidl, 0, out attributes) < 0)
                return false;
            try
            {
                return SHGetPidlInfo(pidl, 0, out info, (uint)Marshal.SizeOf(typeof(FileInfo)), flags | 8) != UIntPtr.Zero;
            }
            finally { Marshal.FreeCoTaskMem(pidl); }
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct FileInfo
        {
            public IntPtr Icon;
            public int Index;
            public uint Attributes;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string Display;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 80)] public string Type;
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct DisplayDevice
        {
            public int Size;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string Name;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string Description;
            public uint Flags;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string Id;
            [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 128)] public string Key;
        }

        internal delegate bool WindowVisitor(IntPtr window, IntPtr state);
        [DllImport("user32.dll")] internal static extern IntPtr GetShellWindow();
        [DllImport("user32.dll")] internal static extern bool EnumWindows(WindowVisitor callback, IntPtr state);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] internal static extern IntPtr FindWindowEx(IntPtr parent, IntPtr previous, string type, string title);
        [DllImport("user32.dll")] internal static extern bool IsWindow(IntPtr window);
        [DllImport("user32.dll")] internal static extern bool IsWindowVisible(IntPtr window);
        [DllImport("user32.dll")] internal static extern IntPtr GetWindow(IntPtr window, uint relation);
        [DllImport("user32.dll")] private static extern bool SetLayeredWindowAttributes(IntPtr window, uint key, byte alpha, uint flags);
        [DllImport("user32.dll")] internal static extern uint GetWindowThreadProcessId(IntPtr window, out uint owner);
        [DllImport("user32.dll")] private static extern bool ShowWindowAsync(IntPtr window, int how);
        [DllImport("user32.dll")] private static extern IntPtr SendMessageTimeout(IntPtr window, uint msg, UIntPtr w, IntPtr l, uint flags, uint wait, out UIntPtr response);
        [DllImport("user32.dll")] private static extern bool SetProcessDpiAwarenessContext(IntPtr context);
        [DllImport("user32.dll")] private static extern IntPtr SetThreadDpiAwarenessContext(IntPtr context);
        [DllImport("user32.dll")] private static extern IntPtr GetWindowDpiAwarenessContext(IntPtr window);
        [DllImport("shcore.dll")] private static extern int SetProcessDpiAwareness(int context);
        [DllImport("user32.dll")] private static extern bool ScreenToClient(IntPtr window, ref Point value);
        [DllImport("user32.dll")] private static extern IntPtr GetParent(IntPtr window);
        [DllImport("user32.dll")] private static extern IntPtr SetParent(IntPtr window, IntPtr parent);
        [DllImport("user32.dll", EntryPoint = "GetWindowLongW")] private static extern int GetWindowLong(IntPtr window, int slot);
        [DllImport("user32.dll", EntryPoint = "SetWindowLongW")] private static extern int SetWindowLong(IntPtr window, int slot, int value);
        [DllImport("user32.dll")] private static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int w, int h, uint flags);
        [DllImport("user32.dll")] private static extern IntPtr MonitorFromPoint(Point location, uint flags);
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern bool EnumDisplayDevices(string device, uint index, ref DisplayDevice result, uint flags);
        [DllImport("shcore.dll")] private static extern int GetDpiForMonitor(IntPtr monitor, int mode, out uint x, out uint y);
        [DllImport("dwmapi.dll")] private static extern int DwmSetWindowAttribute(IntPtr window, int setting, ref int value, int size);
        [DllImport("shell32.dll", CharSet = CharSet.Unicode)] private static extern UIntPtr SHGetFileInfo(string id, uint attributes, out FileInfo value, uint size, uint flags);
        [DllImport("shell32.dll", EntryPoint = "SHGetFileInfoW")] private static extern UIntPtr SHGetPidlInfo(IntPtr id, uint attributes, out FileInfo value, uint size, uint flags);
        [DllImport("shell32.dll", CharSet = CharSet.Unicode)] private static extern int SHParseDisplayName(string text, IntPtr bindContext, out IntPtr pidl, uint mask, out uint attrs);
        [DllImport("user32.dll")] private static extern bool DestroyIcon(IntPtr icon);
        [DllImport("user32.dll")] internal static extern bool RegisterHotKey(IntPtr hwnd, int id, uint modifiers, uint key);
        [DllImport("user32.dll")] internal static extern bool UnregisterHotKey(IntPtr hwnd, int id);
    }
}
