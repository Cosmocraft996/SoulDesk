using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Linq;

namespace SoulDesk.DesktopGrid
{
    public sealed class GridRenderBox
    {
        public GridBox Box;
        public int X, Y, Width, Height, LogicalWidth, LogicalHeight;

        internal static GridRenderBox At(GridBox box, Rectangle bounds, Size? logicalSize = null)
        {
            return new GridRenderBox
            {
                Box = box, X = bounds.X, Y = bounds.Y, Width = bounds.Width, Height = bounds.Height,
                LogicalWidth = (logicalSize ?? bounds.Size).Width, LogicalHeight = (logicalSize ?? bounds.Size).Height
            };
        }
    }

    public sealed class GridRenderState
    {
        public int Version = 1;
        public int MaximumTabs = GridState.MaximumTabsPerBox;
        public int OwnerPid;
        public string OwnerStarted;
        public string Session;
        public long Revision;
        public bool Visible, Closing;
        public string DefaultCollapseMode = GridBox.Smart;
        public GridDisplayInfo[] Displays = new GridDisplayInfo[0];
        public string CapsuleBar;
        public int CapsuleSpacing = 8;
        public bool CoordinatedMove = true;
        public GridStackRule[] StackRules = new GridStackRule[0];
        public WindowIdentity DesktopLayer;
        public string DesktopStarted;
        public GridRenderBox[] Boxes = new GridRenderBox[0];
        public string[] Files = new string[0];
        public string Completed;
        public string[] Accepted = new string[0];
        public string Error;
    }

    public sealed class GridDisplayInfo
    {
        public string Id, Name;
        public bool Primary;
    }

    public sealed class GridRendererPulse
    {
        public string Session;
        public int Pid;
        public long Counter, Revision;
        public long[] Windows;
    }

    public sealed class GridRenderRequest
    {
        public int Version;
        public string Session;
        public long Revision;
        public string Id;
        public string Action;
        public string Box;
        public string Tab;
        public string Before;
        public string Target;
        public string[] ExpectedTabs;
        public string[] Paths;
        public string[] DisplayOrder;
        public bool? Collapsed;
        public string CollapseMode;
        public string Setting;
        public string Value;
        public double? CapsuleWidth;
        public Dictionary<string, string> Metadata;
        public GridMoveRequest[] Moves;
        public GridRenderBounds Bounds;
    }

    public sealed class GridRenderBounds
    {
        public int X, Y, Width, Height;
    }

    public sealed class GridMoveRequest
    {
        public string Box;
        public GridRenderBounds Bounds;
    }

    internal sealed class GridRendererSession : IDisposable
    {
        private readonly string directory;
        private readonly string stateFile;
        private readonly string requestFile;
        private readonly GridRenderState state = new GridRenderState();
        private Process renderer;

        internal GridRendererSession(string root, WindowIdentity desktopLayer = null)
        {
            using (var process = Process.GetCurrentProcess())
            {
                state.OwnerPid = process.Id;
                state.OwnerStarted = process.StartTime.ToUniversalTime().Ticks.ToString(
                    System.Globalization.CultureInfo.InvariantCulture);
            }
            state.Session = GridState.NewId();
            state.DesktopLayer = desktopLayer;
            if (desktopLayer != null)
                state.DesktopStarted = desktopLayer.Started.ToString(System.Globalization.CultureInfo.InvariantCulture);
            directory = Path.Combine(root, "deskbox-renderer-" + state.Session);
            Directory.CreateDirectory(directory);
            stateFile = Path.Combine(directory, "state.json");
            requestFile = Path.Combine(directory, "request.json");
        }

        internal void Start(string executable)
        {
            if (renderer != null)
                throw new InvalidOperationException("格子界面已经启动。");
            string mode = state.DesktopLayer == null ? "--souldesk-grid-renderer" : "--souldesk-grid-desktop";
            renderer = Process.Start(new ProcessStartInfo(executable, mode + " \"" + directory + "\"")
                {
                    UseShellExecute = false,
                    WorkingDirectory = Path.GetDirectoryName(executable),
                    CreateNoWindow = true
                });
        }

        internal bool Exited { get { return renderer != null && renderer.HasExited; } }
        internal string Error
        {
            get
            {
                string path = Path.Combine(directory, "preview-error.txt");
                return File.Exists(path) ? File.ReadAllText(path).Split('\n')[0].Trim() : null;
            }
        }

        internal bool Ready()
        {
            if (renderer == null || renderer.HasExited || state.DesktopLayer == null)
                return false;
            var pulse = LocalJson.TryRead<GridRendererPulse>(Path.Combine(directory, "pulse.json"));
            return pulse != null && pulse.Session == state.Session && pulse.Pid == renderer.Id &&
                pulse.Revision == state.Revision && pulse.Windows != null && pulse.Windows.Length == state.Boxes.Length &&
                pulse.Windows.Distinct().Count() == pulse.Windows.Length &&
                pulse.Windows.All(handle =>
                {
                    var window = new IntPtr(handle);
                    var identity = WindowIdentity.Read(window);
                    return identity != null && identity.Pid == renderer.Id &&
                        Native.GetWindow(window, 4) == state.DesktopLayer.Hwnd;
                });
        }

        internal void AddToLease(GridLease lease)
        {
            lease.Renderer = new WindowIdentity
            {
                Pid = renderer.Id, Session = renderer.SessionId, Started = renderer.StartTime.ToUniversalTime().Ticks
            };
            lease.RendererSession = state.Session;
            lease.RendererImage = renderer.MainModule.FileName;
        }

        internal void SetVisible()
        {
            if (state.Visible)
                return;
            state.Visible = true;
            state.Revision++;
            LocalJson.Write(stateFile, state);
        }

        internal void Publish(IEnumerable<GridRenderBox> boxes, IEnumerable<string> files, bool visible,
            string defaultCollapseMode = GridBox.Smart, GridDisplayInfo[] displays = null,
            string capsuleBar = null, int capsuleSpacing = 8, bool coordinatedMove = true, GridStackRule[] stackRules = null)
        {
            state.Boxes = boxes.ToArray();
            state.Files = files.Where(References.IsLocalPath).Distinct(StringComparer.OrdinalIgnoreCase).ToArray();
            state.Visible = visible;
            state.DefaultCollapseMode = defaultCollapseMode;
            state.Displays = displays ?? new GridDisplayInfo[0];
            state.CapsuleBar = capsuleBar;
            state.CapsuleSpacing = capsuleSpacing;
            state.CoordinatedMove = coordinatedMove;
            state.StackRules = stackRules ?? new GridStackRule[0];
            state.Revision++;
            LocalJson.Write(stateFile, state);
        }

        internal bool ApplyRequest(GridStore store, string topology = null,
            Func<GridBox, Rectangle, GridPosition> locate = null, Action<string, string> show = null)
        {
            var request = LocalJson.TryRead<GridRenderRequest>(requestFile);
            Guid requestId;
            if (request == null || request.Version != state.Version || request.Session != state.Session ||
                !Guid.TryParseExact(request.Id, "N", out requestId) || request.Id == state.Completed)
                return false;

            state.Completed = request.Id;
            state.Accepted = new string[0];
            state.Error = null;
            try
            {
                if (Apply(store, request, topology, locate, show))
                    return true;
            }
            catch (IOException error) { state.Error = error.Message; }
            catch (UnauthorizedAccessException error) { state.Error = error.Message; }
            catch (InvalidOperationException error) { state.Error = error.Message; }
            LocalJson.Write(stateFile, state);
            return false;
        }

        private bool Apply(GridStore store, GridRenderRequest request, string topology,
            Func<GridBox, Rectangle, GridPosition> locate, Action<string, string> show)
        {
            if (!state.Visible || request.Revision != state.Revision)
                throw new InvalidOperationException("格子已更新，请重新操作。");
            if (request.Action == "activate" || request.Action == "collapse" || request.Action == "detach" ||
                request.Action == "merge" || request.Action == "reorder" || request.Action == "collapse-mode")
                return ApplyBox(store, request);
            if (request.Action == "position" || request.Action == "resize")
                return ApplyPosition(store, request, topology, locate);
            if (request.Action == "move-many")
                return ApplyMoveMany(store, request.Moves, topology, locate);
            if (request.Action == "edit")
                return ApplySetting(store, request);
            if (request.Action == "show" && show != null)
            {
                bool known = request.Setting == "manager" || request.Setting == "search" ||
                    request.Setting == "new" || request.Setting == "map" ||
                    request.Setting == "rules" || request.Setting == "pick" || request.Setting == "appearance" ||
                    request.Setting == "organization" || request.Setting == "stack-rules";
                if (!known || !state.Boxes.Any(box => box.Box.Id == request.Box &&
                    box.Box.Tabs.Any(tab => tab.Id == request.Tab)))
                    throw new InvalidOperationException("格子菜单入口已失效。");
                show(request.Setting, request.Tab);
                return false;
            }
            state.Accepted = ApplyMembers(store, request, state.Files);
            return state.Accepted.Length > 0;
        }

        private static bool ApplyPosition(GridStore store, GridRenderRequest request, string topology,
            Func<GridBox, Rectangle, GridPosition> locate)
        {
            var box = store.State.Boxes.SingleOrDefault(item => item.Id == request.Box);
            var bounds = request.Bounds;
            if (box == null || (request.Action == "resize" ? box.LocksSize : box.LocksPosition) ||
                !ValidBounds(bounds) || topology == null || locate == null || !GridBox.IsCapsuleWidth(request.CapsuleWidth))
                throw new InvalidOperationException("格子已锁定或位置无效。");
            var position = locate(box, new Rectangle(bounds.X, bounds.Y, bounds.Width, bounds.Height));
            store.Change(next =>
            {
                next.Place(topology, box.Id, position);
                if (request.Action == "resize" && request.CapsuleWidth.HasValue)
                    next.Boxes.Single(item => item.Id == box.Id).CapsuleWidth = request.CapsuleWidth;
            });
            return true;
        }

        private static bool ValidBounds(GridRenderBounds bounds)
        {
            return bounds != null && bounds.Width > 0 && bounds.Height > 0 && bounds.Width <= 32000 && bounds.Height <= 32000 &&
                bounds.X >= -1000000 && bounds.Y >= -1000000 && bounds.X <= 1000000 && bounds.Y <= 1000000;
        }

        private bool ApplySetting(GridStore store, GridRenderRequest request)
        {
            var box = store.State.Boxes.SingleOrDefault(item => item.Id == request.Box && !item.Hidden);
            if (box == null || !box.Tabs.Any(tab => tab.Id == request.Tab))
                throw new InvalidOperationException("格子或标签已不存在。");
            store.Change(next =>
            {
                var current = next.Boxes.Single(item => item.Id == request.Box);
                var tab = current.Tabs.Single(item => item.Id == request.Tab);
                switch (request.Setting)
                {
                    case "name":
                        string name = (request.Value ?? "").Trim();
                        if (name.Length == 0 || name.Length > 64)
                            throw new InvalidOperationException("格子名称必须为 1～64 个字符。");
                        tab.Name = name;
                        break;
                    case "list": tab.List = ReadSwitch(request.Value); break;
                    case "stack": tab.StackByType = ReadSwitch(request.Value); break;
                    case "sort": tab.ChangeSort(request.Value); break;
                    case "folder-open":
                        if (request.Value != null && request.Value != "Explorer" && request.Value != "Embedded")
                            throw new InvalidOperationException("文件夹打开方式无效。");
                        tab.FolderOpen = request.Value;
                        break;
                    case "position-lock": current.PositionLocked = ReadSwitch(request.Value); break;
                    case "size-lock": current.SizeLocked = ReadSwitch(request.Value); break;
                    case "hidden": current.Hidden = ReadSwitch(request.Value); break;
                    case "direction":
                        if (!GridBox.IsExpansionDirection(request.Value))
                            throw new InvalidOperationException("胶囊展开方向无效。");
                        current.ExpansionDirection = request.Value;
                        break;
                    case "remove": next.RemoveTab(current.Id, tab.Id); break;
                    case "reset-width": current.CapsuleWidth = null; break;
                    case "group-navigation": current.GroupNavigation = request.Value; break;
                    case "group-title": current.GroupTitleDisplay = request.Value; break;
                    case "group-wheel": current.GroupWheel = ReadSwitch(request.Value); break;
                    case "group-hover": current.GroupHover = ReadSwitch(request.Value); break;
                    case "bind":
                        if (request.Value != null && request.Value != "@primary" &&
                            !state.Displays.Any(display => display.Id == request.Value))
                            throw new InvalidOperationException("指定的显示器已断开。");
                        current.BoundMonitor = request.Value;
                        break;
                    case "stack-settings":
                        tab.StackSettings = request.Metadata ?? new Dictionary<string, string>();
                        break;
                    case "artwork":
                        tab.Artwork = request.Metadata ?? new Dictionary<string, string>();
                        break;
                    default: throw new InvalidOperationException("格子设置无效。");
                }
            }, true);
            return true;
        }

        private static bool ReadSwitch(string value)
        {
            bool enabled;
            if (!bool.TryParse(value, out enabled))
                throw new InvalidOperationException("格子开关值无效。");
            return enabled;
        }

        private static bool ApplyMoveMany(GridStore store, GridMoveRequest[] moves, string topology,
            Func<GridBox, Rectangle, GridPosition> locate)
        {
            if (!store.State.CoordinatedMove || moves == null || moves.Length < 2 || moves.Length > 64 ||
                locate == null || topology == null)
                throw new InvalidOperationException("联动移动已停用，或提交的格子集合无效。");
            var positions = new Dictionary<string, GridPosition>(StringComparer.Ordinal);
            foreach (var move in moves)
            {
                var box = move == null ? null : store.State.Boxes.SingleOrDefault(item => item.Id == move.Box);
                var bounds = move == null ? null : move.Bounds;
                if (box == null || box.Hidden || box.LocksPosition || !ValidBounds(bounds) || positions.ContainsKey(box.Id))
                    throw new InvalidOperationException("参与联动的格子已变更，请重新拖动。");
                positions.Add(box.Id, locate(box, new Rectangle(bounds.X, bounds.Y, bounds.Width, bounds.Height)));
            }
            store.Change(state =>
            {
                foreach (var position in positions) state.Place(topology, position.Key, position.Value);
            }, true);
            return true;
        }

        private static bool ApplyBox(GridStore store, GridRenderRequest request)
        {
            var box = store.State.Boxes.SingleOrDefault(item => item.Id == request.Box);
            if (box == null)
                throw new InvalidOperationException("格子已不存在。");
            if (request.Action == "merge")
            {
                store.Change(next => next.MergeBoxes(box.Id, request.Target), true);
                return true;
            }
            if (request.Action == "reorder")
            {
                store.Change(next => next.ReorderTab(box.Id, request.Tab, request.Target, request.ExpectedTabs), true);
                return true;
            }
            if (request.Action == "collapse")
            {
                if (!request.Collapsed.HasValue)
                    throw new InvalidOperationException("没有提供折叠状态。");
                if (box.Collapsed == request.Collapsed.Value)
                    return false;
                store.Change(next => next.Boxes.Single(item => item.Id == request.Box)
                    .Collapsed = request.Collapsed.Value);
                return true;
            }
            if (request.Action == "collapse-mode")
            {
                if (request.CollapseMode == box.DisplayMode)
                    return false;
                store.Change(next => next.Boxes.Single(item => item.Id == box.Id)
                    .SetCollapseMode(request.CollapseMode));
                return true;
            }
            if (!box.Tabs.Any(tab => tab.Id == request.Tab))
                throw new InvalidOperationException("标签已不属于当前格子。");
            if (request.Action == "detach")
            {
                if (box.Tabs.Count < 2)
                    throw new InvalidOperationException("独立格子无需拆分。");
                store.Change(next => next.MoveTab(box.Id, request.Tab, null), true);
                return true;
            }
            if (box.Active == request.Tab)
                return false;
            store.Change(next => next.Boxes.Single(item => item.Id == request.Box).Active = request.Tab);
            return true;
        }

        private static string[] ApplyMembers(GridStore store, GridRenderRequest request, string[] allowedFiles)
        {
            var tab = store.State.Tabs().SingleOrDefault(item => item.Id == request.Tab);
            if (tab == null || tab.Folder != null || request.Paths == null ||
                request.Paths.Length == 0 || request.Paths.Length > 4096)
                throw new InvalidOperationException("格子或项目无效。");

            var allowed = new HashSet<string>(request.Action == "release" ? tab.Items : allowedFiles.AsEnumerable(),
                StringComparer.OrdinalIgnoreCase);
            var items = request.Paths.Distinct(StringComparer.OrdinalIgnoreCase).ToArray();
            if (items.Any(item => !allowed.Contains(item)))
                throw new InvalidOperationException("项目已不在当前格子或桌面列表。");
            if (request.Action == "group")
            {
                if (items.Any(item => !File.Exists(item) && !Directory.Exists(item)) ||
                    (request.Before != null && !tab.Items.Contains(request.Before, StringComparer.OrdinalIgnoreCase)))
                    throw new InvalidOperationException("原文件或放置位置已经改变。");
                var order = request.DisplayOrder;
                if (order != null)
                {
                    var existing = new HashSet<string>(tab.Items, StringComparer.OrdinalIgnoreCase);
                    if (order.Length > existing.Count || order.Distinct(StringComparer.OrdinalIgnoreCase).Count() != order.Length ||
                        order.Any(path => !existing.Contains(path)))
                        throw new InvalidOperationException("格子显示顺序已变更，请重新拖动。");
                }
                if (items.Contains(request.Before, StringComparer.OrdinalIgnoreCase))
                    return items;
                store.Change(next =>
                {
                    if (order != null)
                    {
                        var target = next.Tabs().Single(entry => entry.Id == request.Tab);
                        target.Items = order.Concat(target.Items).Distinct(StringComparer.OrdinalIgnoreCase).ToList();
                        target.ChangeSort("manual");
                    }
                    next.Attach(request.Tab, items, request.Before);
                }, true);
            }
            else if (request.Action == "release")
                store.Change(next => next.Release(items), true);
            else
                throw new InvalidOperationException("格子操作无效。");
            return items;
        }

        public void Dispose()
        {
            if (renderer == null)
                return;
            try
            {
                if (!renderer.HasExited)
                {
                    try
                    {
                        state.Closing = true;
                        state.Revision++;
                        LocalJson.Write(stateFile, state);
                    }
                    catch (IOException) { }
                    catch (UnauthorizedAccessException) { }
                    if (!renderer.WaitForExit(1200))
                        renderer.Kill();
                }
            }
            catch (InvalidOperationException) { }
            catch (System.ComponentModel.Win32Exception) { }
            finally
            {
                renderer.Dispose();
                renderer = null;
            }
        }
    }
}
