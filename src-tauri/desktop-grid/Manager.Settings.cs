using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Windows.Forms;

namespace SoulDesk.DesktopGrid
{
    internal sealed partial class GridManager
    {
        private static readonly Color SettingsCard = Color.FromArgb(24, 40, 58);
        private static readonly Color SettingsControl = Color.FromArgb(35, 53, 73);
        private static readonly Color SettingsBorder = Color.FromArgb(76, 102, 129);
        private static readonly Color SettingsAccent = Color.FromArgb(100, 208, 255);
        private Action refreshSettings;
        private Action<int> selectSettingsSection;
        private bool refreshingSettings;
        private Font settingsHeading, settingsTitle, settingsSymbol;

        private Control CreateSettingsPage()
        {
            var page = new Panel { Dock = DockStyle.Fill, Visible = false };
            var navigation = new FlowLayoutPanel { Dock = DockStyle.Top, AutoSize = true,
                AutoSizeMode = AutoSizeMode.GrowAndShrink, WrapContents = true };
            var content = new Panel { Dock = DockStyle.Fill, Padding = new Padding(0, 10, 0, 0) };
            var names = new[] { "布局与展开", "单格设置", "整理与堆栈", "全局搜索", "布局备份" };
            var descriptions = new[]
            {
                "管理跟随默认的格子、联动拖动和胶囊排列。",
                "修改所选格子的显示器、位置、外观与整理规则。",
                "用规则归类桌面项目，并按扩展名叠放文件。",
                "为格子、桌面和开始菜单之外的文件补充索引结果。",
                "将格子位置、标签、规则和文件路径保存到本地。"
            };
            var pages = new Panel[names.Length];
            var cards = new TableLayoutPanel[names.Length];
            var tabs = new RadioButton[names.Length];
            var refresh = names.Select(name => new List<Action<GridState>>()).ToArray();
            int currentSection = 0;
            settingsHeading = new Font(Font.FontFamily, 15, FontStyle.Bold);
            settingsTitle = new Font(Font.FontFamily, Font.Size, FontStyle.Bold);
            settingsSymbol = new Font("Segoe MDL2 Assets", 12);
            page.Disposed += (sender, args) =>
            {
                settingsHeading.Dispose();
                settingsTitle.Dispose();
                settingsSymbol.Dispose();
            };

            for (int i = 0; i < names.Length; i++)
            {
                int section = i;
                var tab = tabs[i] = new RadioButton
                {
                    Text = names[i], AccessibleName = names[i], Appearance = Appearance.Button, FlatStyle = FlatStyle.Flat,
                    UseVisualStyleBackColor = false, AutoSize = true, MinimumSize = new Size(74, 38),
                    TextAlign = ContentAlignment.MiddleCenter, Margin = new Padding(0, 0, 6, 5),
                    Padding = new Padding(9, 0, 9, 0)
                };
                tab.FlatAppearance.CheckedBackColor = SettingsControl;
                tab.FlatAppearance.MouseOverBackColor = SettingsCard;
                tab.CheckedChanged += (sender, args) => { if (tab.Checked) selectSettingsSection(section); };
                navigation.Controls.Add(tab);

                var scroll = pages[i] = new Panel { Dock = DockStyle.Fill, AutoScroll = true, Visible = false,
                    Padding = new Padding(0, 0, 8, 8) };
                var rows = cards[i] = new TableLayoutPanel { Dock = DockStyle.Top, ColumnCount = 1,
                    AutoSize = true, AutoSizeMode = AutoSizeMode.GrowAndShrink, Margin = Padding.Empty };
                rows.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
                AddSettingsRow(rows, SettingsHeading(names[i], descriptions[i]),
                    settingsHeading.Height + Font.Height + 16);
                scroll.Controls.Add(rows);
                content.Controls.Add(scroll);
            }
            selectSettingsSection = index =>
            {
                if (!tabs[index].Checked)
                {
                    tabs[index].Checked = true;
                    return;
                }
                currentSection = index;
                for (int i = 0; i < tabs.Length; i++)
                {
                    pages[i].Visible = i == index;
                    tabs[i].FlatAppearance.BorderColor = i == index ? SettingsAccent : SettingsBorder;
                    tabs[i].ForeColor = i == index ? SettingsAccent : Secondary;
                    tabs[i].BackColor = i == index ? SettingsControl : Canvas;
                }
                refreshSettings();
            };
            refreshSettings = () =>
            {
                if (!page.Visible) return;
                refreshingSettings = true;
                try { foreach (var update in refresh[currentSection]) update(app.Store.State); }
                finally { refreshingSettings = false; }
            };
            page.VisibleChanged += (sender, args) => refreshSettings();

            BuildLayoutSettings(cards[0], refresh[0]);
            BuildSingleGridSettings(cards[1], refresh[1]);
            BuildOrganizationSettings(cards[2], refresh[2]);
            BuildSearchSettings(cards[3], refresh[3]);
            SettingCard(cards[4], "\uE74E", "导出布局", "保存一份可重新导入的 JSON 文件。",
                ActionButton("导出布局", () => Transfer(true)));
            SettingCard(cards[4], "\uE8E5", "导入布局", "读入备份并替换当前布局。导入前可先导出当前布局。",
                ActionButton("导入布局", () => Transfer(false)));
            page.Controls.Add(content);
            page.Controls.Add(navigation);
            selectSettingsSection(0);
            return page;
        }

        private void BuildLayoutSettings(TableLayoutPanel rows, List<Action<GridState>> refresh)
        {
            var collapse = CollapsePicker("默认展开方式", false);
            var collapseModes = GridBox.CollapseModes.Skip(1).ToArray();
            collapse.SelectionChangeCommitted += (sender, args) =>
                app.Edit(state => state.SetDefaultCollapseMode(collapseModes[collapse.SelectedIndex]));
            refresh.Add(state => collapse.SelectedIndex = Array.IndexOf(collapseModes, state.DefaultCollapseMode));
            SettingCard(rows, "\uE70E", "默认展开方式", "只应用于选择「跟随默认」的格子。", collapse);

            SettingCard(rows, "\uE8AB", "联动拖动", "按住 Ctrl 拖动标题，同屏未锁定的格子一起移动。",
                SettingsSwitch("联动拖动", refresh, state => state.CoordinatedMove,
                    (state, value) => state.CoordinatedMove = value, true));

            var arrangement = SettingsCombo("胶囊栏布局", 150);
            var modes = new[] { null, "Horizontal", "Vertical" };
            arrangement.Items.AddRange(new[] { "自由摆放", "横向排列", "纵向排列" });
            arrangement.SelectionChangeCommitted += (sender, args) => app.Edit(state =>
                state.CapsuleBar = modes[arrangement.SelectedIndex], true);
            SettingCard(rows, "\uE8A9", "胶囊栏布局", "将同屏收起的格子排成一行或一列。", arrangement);

            var spacing = SettingsCombo("胶囊间距", 110);
            spacing.Items.AddRange(Enumerable.Range(0, 33).Select(value => value + " 像素").ToArray());
            spacing.SelectionChangeCommitted += (sender, args) => app.Edit(state =>
                state.CapsuleSpacing = spacing.SelectedIndex, true);
            SettingCard(rows, "\uE8F0", "胶囊间距", "用于横向或纵向排列。", spacing);
            refresh.Add(state =>
            {
                arrangement.SelectedIndex = Array.IndexOf(modes, state.CapsuleBar);
                spacing.SelectedIndex = state.CapsuleSpacing;
                spacing.Enabled = state.CapsuleBar != null;
            });
        }

        private void BuildSingleGridSettings(TableLayoutPanel rows, List<Action<GridState>> refresh)
        {
            var target = SettingsCombo("当前格子", 174);
            var collapse = CollapsePicker("单格展开方式", true);
            var monitor = SettingsCombo("显示器绑定", 174);
            var appearance = ActionButton("调整", () =>
            {
                var selected = target.SelectedItem as SettingsChoice;
                if (selected != null) EditAppearance(app, this, selected.Value);
            });
            string boxId = null;
            var rules = ActionButton("编辑归类规则", () =>
            {
                var box = app.Store.State.Boxes.FirstOrDefault(grid => grid.Id == boxId);
                if (box != null) EditRules(app, this, box.Current().Id);
            });
            Action<Action<GridBox>> change = edit =>
            {
                var selection = boxId;
                if (selection != null && !refreshingSettings)
                    app.Edit(state => edit(state.Boxes.Single(box => box.Id == selection)), true);
            };
            collapse.SelectionChangeCommitted += (sender, args) =>
                change(box => box.SetCollapseMode(GridBox.CollapseModes[collapse.SelectedIndex]));
            monitor.SelectionChangeCommitted += (sender, args) =>
            {
                var choice = monitor.SelectedItem as SettingsChoice;
                if (choice != null) change(box => box.BoundMonitor = choice.Value);
            };
            var positionLock = CreateSettingsSwitch("锁定位置", value => change(box => box.PositionLocked = value));
            var sizeLock = CreateSettingsSwitch("锁定大小", value => change(box => box.SizeLocked = value));
            SettingCard(rows, "\uE80A", "当前格子", "格子组共用位置、展开方式和显示器绑定。", target);
            SettingCard(rows, "\uE70E", "展开方式", "可单独选择展开方式，或使用全局默认值。", collapse);
            SettingCard(rows, "\uE7F4", "显示器绑定", "跟随拖动、主屏或一台指定的显示器。", monitor);
            SettingCard(rows, "\uE72E", "锁定位置", "固定格子的桌面位置。", positionLock);
            SettingCard(rows, "\uE740", "锁定大小", "防止拖动边缘时更改格子尺寸。", sizeLock);
            SettingCard(rows, "\uE790", "底色与透明度", "标题图标和背景图可在格子标题菜单调整。", appearance);
            SettingCard(rows, "\uE71C", "归类规则", "按名称或扩展名匹配桌面项目，归入当前标签。", rules);

            refresh.Add(state =>
            {
                var choices = state.Boxes.Select(box => new SettingsChoice(box.Id, box.Current().Name +
                    (box.Tabs.Count > 1 ? "（" + box.Tabs.Count + " 个标签）" : ""))).ToArray();
                var current = state.Boxes.FirstOrDefault(box => box.Id == boxId) ?? state.Boxes.FirstOrDefault();
                boxId = current == null ? null : current.Id;
                RefreshChoices(target, choices.Length == 0 ? new[] { new SettingsChoice(null, "暂无格子") } : choices, boxId);
                target.Enabled = collapse.Enabled = monitor.Enabled = appearance.Enabled =
                    positionLock.Enabled = sizeLock.Enabled = rules.Enabled = current != null;
                collapse.SelectedIndex = current == null ? -1 : Array.IndexOf(GridBox.CollapseModes, current.DisplayMode);
                positionLock.Checked = current != null && current.LocksPosition;
                sizeLock.Checked = current != null && current.LocksSize;
                var screens = new List<SettingsChoice> { new SettingsChoice(null, "跟随拖动"), new SettingsChoice("@primary", "始终在主屏") };
                string bound = current == null ? null : current.BoundMonitor;
                if (current != null)
                    screens.AddRange(app.DisplayChoices().Select(display => new SettingsChoice(display.Id, display.Name)));
                if (!screens.Any(screen => screen.Value == bound))
                    screens.Add(new SettingsChoice(bound, "显示器离线 · 保留绑定"));
                RefreshChoices(monitor, screens, bound);
            });
            target.SelectionChangeCommitted += (sender, args) =>
            {
                boxId = ((SettingsChoice)target.SelectedItem).Value;
                refreshSettings();
            };
        }

        private void BuildOrganizationSettings(TableLayoutPanel rows, List<Action<GridState>> refresh)
        {
            SettingCard(rows, "\uE945", "自动归类", "新桌面项目稳定后按规则原地归组。",
                SettingsSwitch("自动归类", refresh, state => state.AutoSort, (state, value) => state.AutoSort = value));
            SettingCard(rows, "\uE716", "包含公共桌面", "整理时也检查公共桌面上的项目。",
                SettingsSwitch("包含公共桌面", refresh, state => state.IncludePublic, (state, value) => state.IncludePublic = value));
            var button = ActionButton("编辑规则", StackRules);
            var detail = SettingCard(rows, "\uE81E", "自定义堆栈", "", button);
            refresh.Add(state => detail.Text = state.StackRules.Count + " 条扩展名规则，按第一条匹配规则归组。");
        }

        private void BuildSearchSettings(TableLayoutPanel rows, List<Action<GridState>> refresh)
        {
            var index = SettingsSwitch("Windows 搜索索引", refresh, state => state.SearchIndexed,
                (state, value) => state.SearchIndexed = value);
            index.Enabled = !app.Isolated;
            SettingCard(rows, "\uE721", "Windows 搜索索引",
                app.Isolated ? "隔离预览只查询本地样本。" : "补充 Windows 已索引的文件。", index);

            var choose = ActionButton("选择程序", () =>
            {
                using (var picker = new OpenFileDialog { FileName = "es.exe", Filter = "Everything CLI (es.exe)|es.exe" })
                {
                    if (picker.ShowDialog(this) != DialogResult.OK) return;
                    if (!References.IsLocalPath(picker.FileName) || !References.Equal(Path.GetFileName(picker.FileName), "es.exe"))
                    {
                        app.Report("请选择本机的 Everything es.exe。");
                        return;
                    }
                    app.Edit(state => state.EverythingCli = picker.FileName);
                }
            });
            var clear = ActionButton("清除", () => app.Edit(state => state.EverythingCli = null));
            choose.Enabled = !app.Isolated;
            var actions = new FlowLayoutPanel { AutoSize = true, AutoSizeMode = AutoSizeMode.GrowAndShrink, WrapContents = false };
            actions.Controls.AddRange(new Control[] { choose, clear });
            var path = SettingCard(rows, "\uE8A5", "Everything", "", actions);
            var pathTip = new ToolTip();
            rows.Disposed += (sender, args) => pathTip.Dispose();
            refresh.Add(state =>
            {
                path.Text = state.EverythingCli ?? "尚未配置。可选择本机的 es.exe。";
                pathTip.SetToolTip(path, state.EverythingCli);
                clear.Enabled = !app.Isolated && state.EverythingCli != null;
            });
        }

        private Panel SettingsHeading(string title, string description)
        {
            var header = new Panel { Dock = DockStyle.Fill, Margin = Padding.Empty };
            header.Controls.Add(new Label { Text = description, Dock = DockStyle.Top, Height = Font.Height + 4,
                ForeColor = Secondary, TextAlign = ContentAlignment.MiddleLeft });
            header.Controls.Add(new Label { Text = title, Dock = DockStyle.Top, Height = settingsHeading.Height + 5,
                Font = settingsHeading, TextAlign = ContentAlignment.MiddleLeft });
            return header;
        }

        private Label SettingCard(TableLayoutPanel rows, string glyph, string title, string description, Control editor)
        {
            var card = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 3, RowCount = 1,
                BackColor = SettingsCard, Margin = new Padding(0, 0, 0, 6), Padding = new Padding(13, 1, 13, 1) };
            card.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 42));
            card.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
            card.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
            card.Controls.Add(new Label { Text = glyph, Font = settingsSymbol, ForeColor = SettingsAccent, BackColor = SettingsControl,
                Size = new Size(31, 31), Anchor = AnchorStyles.Left, Margin = Padding.Empty,
                TextAlign = ContentAlignment.MiddleCenter }, 0, 0);
            var copy = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 1, RowCount = 2,
                Margin = new Padding(1, 0, 9, 0) };
            int titleHeight = settingsTitle.Height + 9;
            copy.RowStyles.Add(new RowStyle(SizeType.Absolute, titleHeight));
            copy.RowStyles.Add(new RowStyle(SizeType.Percent, 100));
            copy.Controls.Add(new Label { Text = title, Dock = DockStyle.Fill, Margin = Padding.Empty,
                Font = settingsTitle, TextAlign = ContentAlignment.BottomLeft }, 0, 0);
            var details = new Label { Text = description, Dock = DockStyle.Fill, Margin = new Padding(0, 4, 0, 1),
                ForeColor = Secondary, AutoEllipsis = true };
            copy.Controls.Add(details, 0, 1);
            editor.Anchor = AnchorStyles.Right;
            editor.Margin = Padding.Empty;
            card.Controls.Add(copy, 1, 0);
            card.Controls.Add(editor, 2, 0);
            AddSettingsRow(rows, card, Math.Max(79, titleHeight + Font.Height * 2 + 15));
            return details;
        }

        private static void AddSettingsRow(TableLayoutPanel rows, Control control, int height)
        {
            int index = rows.RowCount++;
            rows.RowStyles.Add(new RowStyle(SizeType.Absolute, height));
            rows.Controls.Add(control, 0, index);
        }

        private ComboBox SettingsCombo(string label, int width)
        {
            var picker = new ComboBox { AccessibleName = label, Width = width, DropDownWidth = Math.Max(width, 235),
                Font = Font, DropDownStyle = ComboBoxStyle.DropDownList, DrawMode = DrawMode.OwnerDrawFixed,
                ItemHeight = Font.Height + 4, BackColor = SettingsControl, ForeColor = Ink,
                FlatStyle = FlatStyle.Flat, MaxDropDownItems = 12 };
            picker.DrawItem += (sender, args) =>
            {
                var selected = (args.State & DrawItemState.Selected) != 0 &&
                    (args.State & DrawItemState.ComboBoxEdit) == 0;
                using (var background = new SolidBrush(selected ? SettingsBorder : SettingsControl))
                    args.Graphics.FillRectangle(background, args.Bounds);
                if (args.Index < 0) return;
                var text = Rectangle.Inflate(args.Bounds, -2, 0);
                TextRenderer.DrawText(args.Graphics, picker.GetItemText(picker.Items[args.Index]), picker.Font,
                    text, picker.Enabled ? Ink : Secondary, TextFormatFlags.VerticalCenter |
                    TextFormatFlags.SingleLine | TextFormatFlags.EndEllipsis | TextFormatFlags.NoPrefix);
                args.DrawFocusRectangle();
            };
            return picker;
        }

        private ComboBox CollapsePicker(string name, bool includeDefault)
        {
            var labels = includeDefault ? GridBox.CollapseLabels : GridBox.CollapseLabels.Skip(1).ToArray();
            int width = labels.Max(label => TextRenderer.MeasureText(label, Font).Width) +
                SystemInformation.VerticalScrollBarWidth + 14;
            var picker = SettingsCombo(name, Math.Max(150, width));
            picker.Items.AddRange(labels);
            return picker;
        }

        private CheckBox SettingsSwitch(string label, List<Action<GridState>> refresh, Func<GridState, bool> read,
            Action<GridState, bool> write, bool undo = false)
        {
            var toggle = CreateSettingsSwitch(label, value => app.Edit(state => write(state, value), undo));
            refresh.Add(state => toggle.Checked = read(state));
            return toggle;
        }

        private CheckBox CreateSettingsSwitch(string label, Action<bool> change)
        {
            var toggle = new CheckBox { Text = "关闭", AccessibleName = label, Appearance = Appearance.Button,
                FlatStyle = FlatStyle.Flat, Size = new Size(71, 32), UseVisualStyleBackColor = false,
                TextAlign = ContentAlignment.MiddleCenter, BackColor = SettingsControl, ForeColor = Ink };
            toggle.FlatAppearance.BorderColor = SettingsBorder;
            toggle.FlatAppearance.CheckedBackColor = SettingsAccent;
            toggle.CheckedChanged += (sender, args) =>
            {
                toggle.Text = toggle.Checked ? "开启" : "关闭";
                toggle.ForeColor = toggle.Checked ? Color.FromArgb(20, 35, 49) : Ink;
                if (!refreshingSettings) change(toggle.Checked);
            };
            return toggle;
        }

        private static void RefreshChoices(ComboBox picker, IEnumerable<SettingsChoice> choices, string selected)
        {
            var items = choices.ToArray();
            if (!picker.Items.Cast<SettingsChoice>().Select(choice => Tuple.Create(choice.Value, choice.Label))
                .SequenceEqual(items.Select(choice => Tuple.Create(choice.Value, choice.Label))))
            {
                picker.Items.Clear();
                picker.Items.AddRange(items);
            }
            picker.SelectedIndex = Array.FindIndex(items, choice => choice.Value == selected);
        }

        private sealed class SettingsChoice
        {
            internal readonly string Value, Label;
            internal SettingsChoice(string value, string label) { Value = value; Label = label; }
            public override string ToString() { return Label; }
        }
    }
}
