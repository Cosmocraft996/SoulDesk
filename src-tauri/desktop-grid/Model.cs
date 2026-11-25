using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Web.Script.Serialization;

namespace SoulDesk.DesktopGrid
{
    internal static class LocalJson
    {
        private const int Limit = 4 * 1024 * 1024;
        private static JavaScriptSerializer Serializer()
        {
            return new JavaScriptSerializer { MaxJsonLength = Limit, RecursionLimit = 28 };
        }

        internal static T Read<T>(string path)
        {
            using (var file = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete))
            using (var reader = new StreamReader(file, Encoding.UTF8))
            {
                if (file.Length > Limit)
                    throw new InvalidDataException("格子数据超过 4 MiB。");
                return Serializer().Deserialize<T>(reader.ReadToEnd());
            }
        }

        internal static T TryRead<T>(string path) where T : class
        {
            try { return Read<T>(path); }
            catch (IOException) { return null; }
            catch (UnauthorizedAccessException) { return null; }
            catch (ArgumentException) { return null; }
            catch (InvalidOperationException) { return null; }
        }

        internal static T Copy<T>(T value)
        {
            return Serializer().Deserialize<T>(Serializer().Serialize(value));
        }

        internal static void Write<T>(string path, T value, string backup = null)
        {
            string pending = path + "." + Guid.NewGuid().ToString("N") + ".tmp";
            try
            {
                byte[] data = Encoding.UTF8.GetBytes(Serializer().Serialize(value));
                if (data.Length > Limit)
                    throw new InvalidDataException("格子数据超过 4 MiB。");
                using (var file = new FileStream(pending, FileMode.CreateNew, FileAccess.Write, FileShare.None))
                {
                    file.Write(data, 0, data.Length);
                    file.Flush(true);
                }
                if (File.Exists(path))
                    File.Replace(pending, path, backup, true);
                else
                    File.Move(pending, path);
            }
            finally
            {
                if (File.Exists(pending))
                    File.Delete(pending);
            }
        }
    }

    public sealed class GridTab
    {
        public string Id = GridState.NewId();
        public string Name = "新格子";
        public string Folder;
        public string Rules;
        public string Sort = "manual";
        public bool? SortDescending;
        public bool List;
        public bool StackByType;
        public string FolderOpen;
        public Dictionary<string, string> StackSettings = new Dictionary<string, string>();
        public Dictionary<string, string> Artwork = new Dictionary<string, string>();
        public List<string> Items = new List<string>();

        internal static readonly string[] SortModes = { "manual", "name", "type", "date", "size" };
        internal bool Descending { get { return SortDescending ?? (Sort == "date" || Sort == "size"); } }

        internal void ChangeSort(string mode)
        {
            if (!SortModes.Contains(mode))
                throw new InvalidOperationException("排序方式无效。");
            SortDescending = mode != "manual" && Sort == mode && !Descending;
            Sort = mode;
        }
    }

    public sealed class GridBox
    {
        public string Id = GridState.NewId();
        public bool Collapsed;
        public bool Locked;
        public bool? PositionLocked;
        public bool? SizeLocked;
        public bool Hidden;
        public string ExpansionDirection;
        public double? CapsuleWidth;
        public string GroupNavigation;
        public string GroupTitleDisplay;
        public bool? GroupWheel;
        public bool GroupHover;
        public string BoundMonitor;
        public bool HoverExpand = true;
        public string CollapseMode;
        public string Active;
        public string Color = "#283635";
        public int Opacity = 90;
        public List<GridTab> Tabs = new List<GridTab>();

        internal const string Smart = "Smart", Click = "Click", Expanded = "Expanded", SystemDefault = "System";
        internal static readonly string[] CollapseModes = { SystemDefault, Smart, Click, Expanded };
        internal static readonly string[] CollapseLabels = { "跟随默认", "悬停自动展开", "单击展开", "始终展开" };

        internal string DisplayMode { get { return CollapseMode ?? (HoverExpand ? Smart : Click); } }
        internal bool LocksPosition { get { return PositionLocked ?? Locked; } }
        internal bool LocksSize { get { return SizeLocked ?? Locked; } }
        internal static bool IsExpansionDirection(string value)
        {
            return value == null || value == "Auto" || value == "Down" || value == "Up";
        }
        internal static bool IsCapsuleWidth(double? width)
        {
            return !width.HasValue || (width.Value >= 1 && width.Value <= 4096);
        }
        internal void SetCollapseMode(string mode)
        {
            if (!CollapseModes.Contains(mode))
                throw new InvalidOperationException("格子展开方式无效。");
            if (mode == DisplayMode)
                return;
            CollapseMode = mode;
            HoverExpand = mode == Smart || mode == SystemDefault;
            Collapsed = mode != Expanded;
        }

        internal GridTab Current()
        {
            return Tabs.FirstOrDefault(tab => tab.Id == Active) ?? Tabs[0];
        }
    }

    public sealed class GridPosition
    {
        public string Monitor;
        public double X;
        public double Y;
        public int Width = 324;
        public int Height = 255;
    }

    public sealed class GridStackRule
    {
        public string Id = GridState.NewId();
        public string Name;
        public List<string> Extensions = new List<string>();
        public override string ToString() { return Name + "  ·  " + string.Join(", ", Extensions); }
    }

    public sealed class GridState
    {
        internal const int MaximumTabsPerBox = 24;
        public int Schema = 1;
        public bool AutoSort;
        public bool IncludePublic;
        public bool SearchIndexed = true;
        public string DefaultCollapseMode = GridBox.Smart;
        public string CapsuleBar;
        public int CapsuleSpacing = 8;
        public bool CoordinatedMove = true;
        public List<GridStackRule> StackRules = new List<GridStackRule>();
        public string EverythingCli;
        public List<GridBox> Boxes = new List<GridBox>();
        public Dictionary<string, Dictionary<string, GridPosition>> Layouts =
            new Dictionary<string, Dictionary<string, GridPosition>>();

        internal static string NewId() { return Guid.NewGuid().ToString("N"); }

        internal static GridState Empty() { return new GridState(); }

        internal IEnumerable<GridTab> Tabs() { return Boxes.SelectMany(box => box.Tabs); }

        internal void SetDefaultCollapseMode(string mode)
        {
            if (mode == DefaultCollapseMode) return;
            DefaultCollapseMode = mode;
            if (mode == GridBox.Click)
                foreach (var box in Boxes.Where(item => item.DisplayMode == GridBox.SystemDefault))
                    box.Collapsed = true;
        }

        internal void Validate()
        {
            if (Schema != 1 || Boxes == null || Boxes.Count > 64 || Layouts == null || Layouts.Count > 64 ||
                (DefaultCollapseMode != GridBox.Smart && DefaultCollapseMode != GridBox.Click && DefaultCollapseMode != GridBox.Expanded) ||
                (CapsuleBar != null && CapsuleBar != "Horizontal" && CapsuleBar != "Vertical") ||
                CapsuleSpacing < 0 || CapsuleSpacing > 32)
                throw new InvalidDataException("格子备份的版本或结构无效。");
            if (StackRules == null || StackRules.Count > 64 ||
                StackRules.Any(rule => rule == null || !GoodId(rule.Id) || string.IsNullOrWhiteSpace(rule.Name) ||
                    rule.Name.Length > 64 || rule.Extensions == null || rule.Extensions.Count == 0 || rule.Extensions.Count > 64 ||
                    rule.Extensions.Any(ext => string.IsNullOrEmpty(ext) || !Regex.IsMatch(ext, "^[a-z0-9_+-]{1,24}$"))) ||
                StackRules.Select(rule => rule.Id).Distinct().Count() != StackRules.Count)
                throw new InvalidDataException("自定义堆栈规则无效。");
            var groups = new HashSet<string>(StringComparer.Ordinal);
            var tabs = new HashSet<string>(StringComparer.Ordinal);
            var items = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
            foreach (var box in Boxes)
            {
                if (box == null || !GoodId(box.Id) || !groups.Add(box.Id) || box.Tabs == null ||
                    box.Tabs.Count == 0 || box.Tabs.Count > MaximumTabsPerBox || box.Opacity < 35 || box.Opacity > 100 ||
                    box.Color == null || !Regex.IsMatch(box.Color, "^#[a-fA-F0-9]{6}$") ||
                    (box.CollapseMode != null && !GridBox.CollapseModes.Contains(box.CollapseMode)) ||
                    !GridBox.IsExpansionDirection(box.ExpansionDirection) ||
                    !GridBox.IsCapsuleWidth(box.CapsuleWidth) ||
                    (box.GroupNavigation != null && box.GroupNavigation != "Tabs" && box.GroupNavigation != "Stack") ||
                    (box.GroupTitleDisplay != null && box.GroupTitleDisplay != "IconAndText" &&
                        box.GroupTitleDisplay != "IconOnly" && box.GroupTitleDisplay != "TextOnly"))
                    throw new InvalidDataException("格子外观或分组无效。");
                if (box.BoundMonitor != null && (box.BoundMonitor.Length == 0 || box.BoundMonitor.Length > 256))
                    throw new InvalidDataException("绑定的显示器无效。");
                foreach (var tab in box.Tabs)
                {
                    if (tab == null || !GoodId(tab.Id) || !tabs.Add(tab.Id) || string.IsNullOrWhiteSpace(tab.Name) ||
                        tab.Name.Length > 64 || tab.Items == null || tab.Items.Count > 4096 ||
                        (tab.Folder != null && !References.IsLocalPath(tab.Folder)) ||
                        (tab.Rules != null && tab.Rules.Length > 512) ||
                        !GridTab.SortModes.Contains(tab.Sort) ||
                        !ValidMetadata(tab.StackSettings, StackKeys, 262144) ||
                        !ValidMetadata(tab.Artwork, ArtworkKeys, 128) ||
                        (tab.FolderOpen != null && tab.FolderOpen != "Explorer" && tab.FolderOpen != "Embedded"))
                        throw new InvalidDataException("格子成员、文件夹或排序设置无效。");
                    foreach (string item in tab.Items)
                        if (!References.IsLocalPath(item) || !items.Add(item))
                            throw new InvalidDataException("格子备份包含无效或重复的项目。");
                }
                if (!box.Tabs.Any(tab => tab.Id == box.Active))
                    box.Active = box.Tabs[0].Id;
            }
            foreach (var profile in Layouts)
            {
                if (profile.Key.Length > 192 || profile.Value == null || profile.Value.Count > 64)
                    throw new InvalidDataException("显示器布局无效。");
                foreach (var item in profile.Value)
                {
                    var pos = item.Value;
                    if (!groups.Contains(item.Key) || pos == null || string.IsNullOrEmpty(pos.Monitor) ||
                        pos.Monitor.Length > 256 || !ValidFraction(pos.X) || !ValidFraction(pos.Y) ||
                        pos.Width < 144 || pos.Width > 10000 || pos.Height < 80 || pos.Height > 10000)
                        throw new InvalidDataException("格子位置或大小无效。");
                }
            }
            if (EverythingCli != null && (!References.IsLocalPath(EverythingCli) ||
                !string.Equals(Path.GetFileName(EverythingCli), "es.exe", StringComparison.OrdinalIgnoreCase)))
                throw new InvalidDataException("Everything 命令行路径无效。");
        }

        private static bool GoodId(string id)
        {
            Guid value;
            return id != null && Guid.TryParseExact(id, "N", out value);
        }

        internal static readonly string[] StackKeys = { "FileStacksEnabled", "FileStackGroupBy", "FileStackThreshold",
            "FileStackOrderBy", "FileStackOpenMode", "FileStackDisabledGroups", "FileStackNameOverrides",
            "FileStackGroupOrder", "FileStackMemberOverrides" };
        internal static readonly string[] ArtworkKeys = { "TitleIconEmoji", "TitleIconImage",
            "BackgroundImage", "BackgroundFit", "BackgroundDim", "BackgroundColor" };

        private static bool ValidMetadata(Dictionary<string, string> values, string[] keys, int limit)
        {
            return values != null && values.Count <= keys.Length &&
                values.All(pair => keys.Contains(pair.Key) && pair.Value != null && pair.Value.Length <= limit);
        }

        private static bool ValidFraction(double value)
        {
            return !double.IsNaN(value) && !double.IsInfinity(value) && value >= 0 && value <= 1;
        }

        internal GridBox Add(string name, string folder)
        {
            var tab = new GridTab { Name = name, Folder = folder };
            var box = new GridBox { Active = tab.Id, Tabs = new List<GridTab> { tab } };
            Boxes.Add(box);
            return box;
        }

        internal void RemoveBox(string id)
        {
            Boxes.RemoveAll(box => box.Id == id);
            foreach (var map in Layouts.Values)
                map.Remove(id);
        }

        internal void Attach(string tabId, IEnumerable<string> references, string before = null)
        {
            var target = Tabs().Single(tab => tab.Id == tabId);
            if (target.Folder != null)
                return;
            var items = references.Where(References.IsLocalPath).Distinct(StringComparer.OrdinalIgnoreCase).ToArray();
            if (items.Contains(before, StringComparer.OrdinalIgnoreCase))
                return;
            foreach (string item in items)
            {
                foreach (var other in Tabs())
                    other.Items.RemoveAll(value => References.Equal(item, value));
            }
            int index = target.Items.FindIndex(item => References.Equal(item, before));
            target.Items.InsertRange(index < 0 ? target.Items.Count : index, items);
        }

        internal void Release(IEnumerable<string> references)
        {
            var ids = new HashSet<string>(references, StringComparer.OrdinalIgnoreCase);
            foreach (var tab in Tabs())
                tab.Items.RemoveAll(ids.Contains);
        }

        internal void MoveTab(string boxId, string tabId, string destination)
        {
            var from = Boxes.Single(box => box.Id == boxId);
            var tab = from.Tabs.Single(item => item.Id == tabId);
            from.Tabs.Remove(tab);
            GridBox target;
            if (destination == null)
            {
                target = new GridBox();
                Boxes.Add(target);
            }
            else
                target = Boxes.Single(box => box.Id == destination);
            target.Tabs.Add(tab);
            target.Active = tabId;
            if (from.Tabs.Count == 0)
                RemoveBox(from.Id);
        }

        internal void MergeBoxes(string source, string destination)
        {
            var from = Boxes.Single(box => box.Id == source);
            var target = Boxes.SingleOrDefault(box => box.Id == destination && box.Id != source);
            if (target == null || from.Tabs.Count + target.Tabs.Count > MaximumTabsPerBox)
                throw new InvalidOperationException("无法合并这两个格子。");
            target.Tabs.AddRange(from.Tabs);
            RemoveBox(source);
        }

        internal void RemoveTab(string boxId, string tabId)
        {
            var box = Boxes.SingleOrDefault(item => item.Id == boxId);
            if (box == null || box.Tabs.RemoveAll(tab => tab.Id == tabId) != 1)
                throw new InvalidOperationException("格子或标签已不存在。");
            if (box.Tabs.Count == 0) RemoveBox(boxId);
        }

        internal void ReorderTab(string boxId, string source, string target, string[] expectedTabs)
        {
            var tabs = Boxes.Single(box => box.Id == boxId).Tabs;
            if (expectedTabs != null && !tabs.Select(tab => tab.Id).SequenceEqual(expectedTabs))
                throw new InvalidOperationException("标签顺序已变更，请重新操作。");
            int from = tabs.FindIndex(tab => tab.Id == source);
            int to = tabs.FindIndex(tab => tab.Id == target);
            if (from < 0 || to < 0 || from == to)
                throw new InvalidOperationException("无法调整标签顺序。");
            var item = tabs[from];
            tabs.RemoveAt(from);
            tabs.Insert(to, item);
        }

        internal GridPosition Position(string topology, string box, IEnumerable<string> screens)
        {
            Dictionary<string, GridPosition> profile;
            GridPosition result;
            if (Layouts.TryGetValue(topology, out profile) && profile.TryGetValue(box, out result))
                return result;

            // 暂无当前组合的布局时找一个仍连接的屏幕，避免覆盖拔屏前的位置。
            var screenIds = new HashSet<string>(screens, StringComparer.Ordinal);
            return Layouts.OrderBy(entry => entry.Key, StringComparer.Ordinal).Select(entry =>
                entry.Value.TryGetValue(box, out result) ? result : null)
                .FirstOrDefault(position => position != null && screenIds.Contains(position.Monitor));
        }

        internal void Place(string topology, string box, GridPosition position)
        {
            Dictionary<string, GridPosition> profile;
            if (!Layouts.TryGetValue(topology, out profile))
            {
                if (Layouts.Count == 64)
                    Layouts.Remove(Layouts.Keys.OrderBy(key => key, StringComparer.Ordinal).First());
                Layouts[topology] = profile = new Dictionary<string, GridPosition>();
            }
            profile[box] = position;
        }

        internal static bool Matches(GridTab tab, string name)
        {
            return (tab.Rules ?? "").Split(new[] { ';' }, StringSplitOptions.RemoveEmptyEntries)
                .Select(value => value.Trim()).Where(value => value.Length > 0)
                .Any(rule => Regex.IsMatch(name, "^" + Regex.Escape(rule)
                    .Replace("\\*", ".*").Replace("\\?", ".") + "$",
                    RegexOptions.IgnoreCase | RegexOptions.CultureInvariant, TimeSpan.FromMilliseconds(50)));
        }
    }

    internal sealed class GridStore
    {
        private readonly string path;
        private GridState undo;
        internal GridState State { get; private set; }
        internal bool CanUndo { get { return undo != null; } }

        internal GridStore(string root)
        {
            Directory.CreateDirectory(root);
            path = Path.Combine(root, "layout.json");
            State = File.Exists(path) ? ReadLayout(path) :
                File.Exists(path + ".bak") ? ReadLayout(path + ".bak") : GridState.Empty();
        }

        internal static GridState ReadLayout(string source)
        {
            var state = LocalJson.Read<GridState>(source);
            if (state == null)
                throw new InvalidDataException("格子备份没有布局。");
            state.Validate();
            return state;
        }

        internal void Change(Action<GridState> edit, bool remember = false)
        {
            var next = LocalJson.Copy(State);
            edit(next);
            Save(next, remember);
        }

        internal void Save(GridState next, bool remember)
        {
            next.Validate();
            LocalJson.Write(path, next, path + ".bak");
            if (remember)
                undo = State;
            State = next;
        }

        internal bool Undo()
        {
            if (undo == null)
                return false;
            Save(undo, false);
            undo = null;
            return true;
        }

        internal void Export(string destination)
        {
            LocalJson.Write(destination, State);
        }
    }

    internal static class References
    {
        internal static bool Equal(string a, string b) { return StringComparer.OrdinalIgnoreCase.Equals(a, b); }

        internal static bool IsLocalPath(string value)
        {
            return value != null && value.Length < 2048 && value.Length > 3 &&
                char.IsLetter(value[0]) && value[1] == ':' && value[2] == '\\' &&
                value.IndexOfAny(Path.GetInvalidPathChars()) < 0 && value.IndexOf(':', 2) < 0;
        }

    }

    internal sealed class DesktopItem
    {
        internal string Id;
        internal string Name;
        internal Rectangle NativeBounds;
        internal Rectangle ProjectedBounds;
        internal bool Shared;

        internal static DesktopItem FromPath(string path)
        {
            return new DesktopItem { Id = path, Name = Path.GetFileName(path) };
        }
    }
}
