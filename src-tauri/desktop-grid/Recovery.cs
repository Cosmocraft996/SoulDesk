using System;
using System.Diagnostics;
using System.IO;
using System.Threading;

namespace SoulDesk.DesktopGrid
{
    public sealed class GridLease
    {
        public string Token;
        public WindowIdentity Owner, Desktop;
        public WindowIdentity Renderer;
        public string RendererSession, RendererImage;
    }

    internal sealed class Recovery
    {
        private readonly string root;
        internal Recovery(string root) { this.root = root; }
        internal string PathFor(string name) { return Path.Combine(root, name + ".json"); }

        internal void Signal(string name, string token)
        {
            LocalJson.Write(PathFor(name), new[] { token });
        }

        internal bool Signaled(string name, string token)
        {
            var message = LocalJson.TryRead<string[]>(PathFor(name));
            return message != null && message.Length == 1 && message[0] == token;
        }

        internal GridLease Current() { return LocalJson.TryRead<GridLease>(PathFor("lease")); }

        internal void RequestRestore()
        {
            var lease = Current();
            Signal("restore", lease == null ? "" : lease.Token);
            if (Valid(lease) && !Signaled("done", lease.Token))
                Restore(lease);
        }

        internal void RecoverAbandoned()
        {
            var lease = Current();
            if (Valid(lease) && !Signaled("done", lease.Token) &&
                !lease.Owner.Same(WindowIdentity.Read(lease.Owner.Hwnd)))
            {
                EndRenderer(lease);
                Restore(lease);
                Signal("done", lease.Token);
            }
        }

        internal void RunGuard(string token)
        {
            var lease = LocalJson.TryRead<GridLease>(PathFor("offer"));
            if (!Valid(lease) || token != lease.Token)
            {
                LocalJson.Write(PathFor("guard-error"), new { token, message = "交接记录无效" });
                return;
            }
            var waiting = Stopwatch.StartNew();
            while (!Native.Responsive(lease.Owner) && waiting.ElapsedMilliseconds < 8000 &&
                lease.Owner.Same(WindowIdentity.Read(lease.Owner.Hwnd)) && !Requested(lease))
                Thread.Sleep(100);
            if (Requested(lease))
                return;
            if (!lease.Owner.Same(WindowIdentity.Read(lease.Owner.Hwnd)) || !SameExecutable(lease.Owner))
            {
                LocalJson.Write(PathFor("guard-error"), new { token, message = "格子进程身份不一致" });
                return;
            }
            if (!ShellDesktop.Matches(lease.Desktop) || !Native.IsWindowVisible(lease.Desktop.Hwnd))
            {
                LocalJson.Write(PathFor("guard-error"), new { token, message = "Explorer 图标视图已变更或不可见" });
                return;
            }
            if (!Native.Responsive(lease.Owner))
            {
                LocalJson.Write(PathFor("guard-error"), new { token, message = "格子窗口尚未就绪" });
                return;
            }
            var pulse = ReadPulse(lease);
            if (lease.Renderer != null && (pulse == null || !SameRenderer(lease)))
            {
                LocalJson.Write(PathFor("guard-error"), new { token, message = "WinUI 桌面格子未就绪" });
                return;
            }

            bool armed = false;
            bool stalled = false;
            bool rendererStalled = false;
            try
            {
                LocalJson.Write(PathFor("lease"), lease);
                armed = true;
                if (Requested(lease))
                    return;
                Native.Visible(lease.Desktop, false, () => ShellDesktop.Matches(lease.Desktop));
                Signal("ready", token);
                var lastResponse = Stopwatch.StartNew();
                var lastFrame = Stopwatch.StartNew();
                long previousPulse = pulse == null ? 0 : pulse.Counter;
                while (ShellDesktop.Matches(lease.Desktop) && !Requested(lease))
                {
                    if (!lease.Owner.Same(WindowIdentity.Read(lease.Owner.Hwnd)))
                        break;
                    if (Native.IsWindowVisible(lease.Desktop.Hwnd))
                        break;
                    if (lease.Renderer != null)
                    {
                        using (var process = lease.Renderer.OpenProcess())
                            if (process == null)
                                break;
                        pulse = ReadPulse(lease);
                        if (pulse != null && pulse.Counter > previousPulse)
                        {
                            previousPulse = pulse.Counter;
                            lastFrame.Restart();
                        }
                        else if (lastFrame.ElapsedMilliseconds >= 4200)
                        {
                            rendererStalled = true;
                            break;
                        }
                    }
                    if (Native.Responsive(lease.Owner))
                        lastResponse.Restart();
                    else if (lastResponse.ElapsedMilliseconds >= 4200)
                    {
                        stalled = true;
                        break;
                    }
                    Thread.Sleep(175);
                }
            }
            finally
            {
                if (armed)
                {
                    // 卡死的 WinUI 窗口可能连带阻塞桌面显隐。先按租约验证并结束它，再交还图标。
                    if (rendererStalled)
                        EndRenderer(lease);
                    while (ShellDesktop.Matches(lease.Desktop))
                    {
                        try { Restore(lease); break; }
                        catch (InvalidOperationException)
                        {
                            // 手动恢复可能早于心跳超时。首次交还失败时先释放仍存在的格子窗口。
                            EndRenderer(lease);
                            if (stalled) EndStalledOwner(lease);
                            Thread.Sleep(400);
                        }
                    }
                    if (stalled) EndStalledOwner(lease);
                    if (stalled || !lease.Owner.Same(WindowIdentity.Read(lease.Owner.Hwnd)))
                        EndRenderer(lease);
                    Signal("done", token);
                }
            }
        }

        private bool Requested(GridLease lease)
        {
            return Signaled("stop", lease.Token) || Signaled("restore", lease.Token);
        }

        internal static bool Valid(GridLease lease)
        {
            Guid session;
            return lease != null && lease.Owner != null && lease.Desktop != null && lease.Token != null &&
                lease.Owner.Session == lease.Desktop.Session &&
                lease.Owner.Session == Process.GetCurrentProcess().SessionId &&
                (lease.Renderer == null || lease.Renderer.Session == lease.Owner.Session &&
                    Guid.TryParseExact(lease.RendererSession, "N", out session) &&
                    References.IsLocalPath(lease.RendererImage));
        }

        internal static void Restore(GridLease lease)
        {
            if (Valid(lease) && ShellDesktop.Matches(lease.Desktop))
                Native.Visible(lease.Desktop, true, () => ShellDesktop.Matches(lease.Desktop));
        }

        private static void EndStalledOwner(GridLease lease)
        {
            if (!SameExecutable(lease.Owner) || Native.Responsive(lease.Owner))
                return;
            using (var owner = lease.Owner.OpenProcess())
                if (owner != null)
                    try { owner.Kill(); }
                    catch (InvalidOperationException) { }
        }

        private GridRendererPulse ReadPulse(GridLease lease)
        {
            if (lease.Renderer == null)
                return null;
            var pulse = LocalJson.TryRead<GridRendererPulse>(
                Path.Combine(root, "deskbox-renderer-" + lease.RendererSession, "pulse.json"));
            return pulse != null && pulse.Session == lease.RendererSession && pulse.Pid == lease.Renderer.Pid
                ? pulse : null;
        }

        private static bool SameRenderer(GridLease lease)
        {
            try
            {
                using (var process = lease.Renderer.OpenProcess())
                    return process != null && References.Equal(process.MainModule.FileName, lease.RendererImage);
            }
            catch (InvalidOperationException) { return false; }
            catch (System.ComponentModel.Win32Exception) { return false; }
        }

        private static void EndRenderer(GridLease lease)
        {
            if (lease.Renderer == null || !SameRenderer(lease))
                return;
            using (var process = lease.Renderer.OpenProcess())
                if (process != null)
                    try { process.Kill(); }
                    catch (InvalidOperationException) { }
                    catch (System.ComponentModel.Win32Exception) { }
        }

        private static bool SameExecutable(WindowIdentity owner)
        {
            try
            {
                using (var candidate = owner.OpenProcess())
                    return candidate != null && References.Equal(
                        Path.ChangeExtension(candidate.MainModule.FileName, ".guard.exe"),
                        System.Windows.Forms.Application.ExecutablePath);
            }
            catch (InvalidOperationException) { return false; }
            catch (System.ComponentModel.Win32Exception) { return false; }
        }
    }
}
