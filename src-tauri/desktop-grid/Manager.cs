using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using System.Windows.Forms;

namespace SoulDesk.DesktopGrid
{
    internal sealed class GridManager : Form
    {
        private static readonly Color Canvas = Color.FromArgb(32, 32, 32);
        private static readonly Color Card = Color.FromArgb(44, 44, 44);
        private static readonly Color Selected = Color.FromArgb(59, 64, 71);
        private static readonly Color Ink = Color.FromArgb(247, 247, 247);
        private static readonly Color Secondary = Color.FromArgb(194, 194, 194);
        private static readonly Color Accent = Color.FromArgb(147, 199, 248);
        private readonly GridApplication app;
        private readonly Label status;
        private readonly Button toggle, undo;
        private readonly ListBox list;
        private readonly ComboBox displayMode;
        private readonly Label empty;
        private readonly NotifyIcon tray;
        private readonly Action<int> selectPage;
        private bool refreshing;
        private bool resourcesDisposed;

        internal GridManager(GridApplication app)
        {
            this.app = app;
            Text = "SoulDesk · 桌面格子";
            ClientSize = new Size(810, 550);
            MinimumSize = new Size(690, 450);
            StartPosition = FormStartPosition.CenterScreen;
            AutoScaleMode = AutoScaleMode.Dpi;
            BackColor = Canvas;
            ForeColor = Ink;
            Font = new Font("Segoe UI", 10);
            Native.Backdrop(Handle);
            var layout = new TableLayoutPanel { Dock = DockStyle.Fill, ColumnCount = 2, RowCount = 1 };
            layout.ColumnStyles.Add(new ColumnStyle(SizeType.Absolute, 172));
            layout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
            var nav = new FlowLayoutPanel { Dock = DockStyle.Fill, FlowDirection = FlowDirection.TopDown,
                WrapContents = false, Padding = new Padding(14, 19, 10, 12), Margin = Padding.Empty };
            var content = new Panel { Dock = DockStyle.Fill, Padding = new Padding(16, 12, 24, 18), Margin = Padding.Empty };
            var header = new TableLayoutPanel { Dock = DockStyle.Top, Height = 62, ColumnCount = 2, RowCount = 1 };
            header.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100));
            header.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
            var title = new Label { Text = "桌面格子", Font = new Font(Font.FontFamily, 21, FontStyle.Bold),
                Dock = DockStyle.Fill, TextAlign = ContentAlignment.MiddleLeft };
            toggle = ActionButton("启用格子", app.Toggle, true);
            toggle.Anchor = AnchorStyles.Right;
            header.Controls.Add(title, 0, 0);
            header.Controls.Add(toggle, 1, 0);
            status = new Label { TextAlign = ContentAlignment.MiddleLeft, AutoEllipsis = true, Dock = DockStyle.Top,
                Height = 36, ForeColor = Secondary };

            list = new ListBox { AccessibleName = "桌面格子列表", Dock = DockStyle.Fill, BorderStyle = BorderStyle.None,
                BackColor = Canvas, ForeColor = Ink, IntegralHeight = false, DrawMode = DrawMode.OwnerDrawFixed, ItemHeight = 57 };
            list.DrawItem += DrawGrid;
            var addItems = ActionButton("添加桌面项目", PickSelected);
            list.DoubleClick += (sender, args) => PickSelected();
            displayMode = new ComboBox { AccessibleName = "展开方式", Width = 146, DropDownStyle = ComboBoxStyle.DropDownList,
                BackColor = Card, ForeColor = Ink, FlatStyle = FlatStyle.Flat };
            displayMode.Items.AddRange(GridBox.CollapseLabels);
            displayMode.SelectionChangeCommitted += (sender, args) =>
            {
                var row = list.SelectedItem as GridRow;
                if (row != null && displayMode.SelectedIndex >= 0)
                    app.Edit(state => state.Boxes.Single(box => box.Id == row.Box.Id)
                        .SetCollapseMode(GridBox.CollapseModes[displayMode.SelectedIndex]));
            };
            var selection = new FlowLayoutPanel { Dock = DockStyle.Bottom, Height = 47, WrapContents = false,
                FlowDirection = FlowDirection.RightToLeft, BackColor = Card, Padding = new Padding(6, 6, 6, 0) };
            var gridMenu = GridLook.Menu();
            var more = ActionButton("更多", () => gridMenu.Show(selection, new Point(selection.Width - 10, selection.Height)));
            selection.Controls.Add(more);
            list.ContextMenuStrip = gridMenu;
            list.MouseDown += (sender, args) =>
            {
                if (args.Button == MouseButtons.Right)
                    list.SelectedIndex = list.IndexFromPoint(args.Location);
            };
            gridMenu.Opening += (sender, args) =>
            {
                foreach (ToolStripItem item in gridMenu.Items.Cast<ToolStripItem>().ToArray()) item.Dispose();
                var row = list.SelectedItem as GridRow;
                args.Cancel = row == null;
                if (row == null) return;
                var box = row.Box;
                var tab = row.Tab;
                gridMenu.Items.Add(app.MenuItem(box.Hidden ? "显示格子" : "隐藏格子", () =>
                    app.Edit(state => state.Boxes.Single(item => item.Id == box.Id).Hidden = !box.Hidden, true)));
                gridMenu.Items.Add(app.MenuItem("重命名", () =>
                {
                    string name = GridLook.Prompt(this, "重命名格子", "输入名称", tab.Name, 64);
                    if (!string.IsNullOrWhiteSpace(name))
                        app.Edit(state => state.Tabs().Single(item => item.Id == tab.Id).Name = name, true);
                }));
                gridMenu.Items.Add(app.MenuItem("底色与透明度", () => EditAppearance(app, this, box.Id)));
                gridMenu.Items.Add(BindingMenu(box));
                gridMenu.Items.Add(app.MenuItem("整理规则", () => EditRules(app, this, tab.Id)));
                if (tab.Folder != null)
                    gridMenu.Items.Add(app.MenuItem("在资源管理器打开映射目录", () => app.Open(tab.Folder)));
                gridMenu.Items.Add(new ToolStripSeparator());
                gridMenu.Items.Add(app.MenuItem("删除格子", () => RemoveGrid(app, this, box.Id, tab.Id)));
            };
            Disposed += (sender, args) => gridMenu.Dispose();
            selection.Controls.Add(addItems);
            displayMode.Margin = new Padding(8, 5, 3, 0);
            selection.Controls.Add(displayMode);
            selection.Controls.Add(new Label { Text = "展开方式", AutoSize = true, Margin = new Padding(3, 10, 2, 0) });
            list.SelectedIndexChanged += (sender, args) =>
            {
                if (!refreshing) UpdateSelection();
                var row = list.SelectedItem as GridRow;
                addItems.Enabled = row != null && row.Tab.Folder == null;
                more.Enabled = row != null;
            };

            var grids = new Panel { Dock = DockStyle.Fill };
            var toolbar = new FlowLayoutPanel { Dock = DockStyle.Top, Height = 44, WrapContents = false };
            toolbar.Controls.AddRange(new Control[] { ActionButton("新建格子", () => CreateGrid(app, this, false)),
                ActionButton("映射目录", () => CreateGrid(app, this, true)), ActionButton("全局搜索", app.ShowSearch) });
            empty = new Label { Text = "还没有格子。可新建格子或映射目录。", ForeColor = Secondary,
                Dock = DockStyle.Fill, TextAlign = ContentAlignment.MiddleCenter };
            var listing = new Panel { Dock = DockStyle.Fill };
            listing.Controls.Add(list);
            listing.Controls.Add(empty);
            grids.Controls.Add(listing);
            grids.Controls.Add(selection);
            grids.Controls.Add(toolbar);

            undo = ActionButton("撤销", app.Undo);
            var organize = new Panel { Dock = DockStyle.Fill, Visible = false };
            var organizeRows = new FlowLayoutPanel { Dock = DockStyle.Fill, FlowDirection = FlowDirection.TopDown,
                WrapContents = false, AutoScroll = true };
            organizeRows.Controls.Add(SettingRow("一键整理", "按格子的归类规则预览待整理的桌面项目。",
                ActionButton("预览整理", () => Categorize(false), true)));
            organizeRows.Controls.Add(SettingRow("搬移原文件", "同盘内移动到格子的文件夹，整理历史可撤回。",
                ActionButton("预览搬移", () => Categorize(true))));
            organizeRows.Controls.Add(SettingRow("整理历史", "查看原路径、目标和结果，或将项目恢复到原路径。",
                ActionButton("查看历史", ShowOrganizationHistory)));
            organizeRows.Controls.Add(SettingRow("撤销整理", "恢复上一次可撤销的布局变更。", undo));
            organize.Controls.Add(organizeRows);

            var settings = new Panel { Dock = DockStyle.Fill, Visible = false };
            var settingRows = new FlowLayoutPanel { Dock = DockStyle.Fill, FlowDirection = FlowDirection.TopDown,
                WrapContents = false, AutoScroll = true };
            settingRows.Controls.Add(SettingRow("格子选项", "设置自动归类、公共桌面与索引搜索。", ActionButton("打开选项", Options)));
            settingRows.Controls.Add(SettingRow("桌面布局", "设置 Ctrl 联动拖动及胶囊排列。",
                ActionButton("布局设置", LayoutOptions)));
            settingRows.Controls.Add(SettingRow("自定义堆栈", "按扩展名分组。每个文件采用第一条匹配规则。",
                ActionButton("编辑规则", StackRules)));
            settingRows.Controls.Add(SettingRow("布局备份", "导入或导出格子位置、标签和文件引用。",
                ActionButton("导入布局", () => Transfer(false)), ActionButton("导出布局", () => Transfer(true))));
            settings.Controls.Add(settingRows);
            foreach (var rows in new[] { organizeRows, settingRows })
                rows.SizeChanged += (sender, args) =>
                {
                    foreach (Control row in rows.Controls) row.Width = Math.Max(0, rows.ClientSize.Width - 4);
                };

            var pages = new[] { grids, organize, settings };
            var names = new[] { "桌面格子", "桌面整理", "设置" };
            var tabs = new Button[names.Length];
            selectPage = index =>
            {
                title.Text = names[index];
                for (int page = 0; page < pages.Length; page++)
                {
                    pages[page].Visible = page == index;
                    tabs[page].BackColor = page == index ? Selected : Canvas;
                }
            };
            for (int i = 0; i < tabs.Length; i++)
            {
                int index = i;
                var button = tabs[i] = ActionButton(names[i], () => selectPage(index));
                button.AutoSize = false;
                button.Size = new Size(142, 39);
                button.TextAlign = ContentAlignment.MiddleLeft;
                button.Padding = new Padding(12, 0, 0, 0);
                button.BackColor = i == 0 ? Selected : Canvas;
                nav.Controls.Add(button);
                content.Controls.Add(pages[i]);
            }
            content.Controls.Add(status);
            content.Controls.Add(header);
            layout.Controls.Add(nav, 0, 0);
            layout.Controls.Add(content, 1, 0);
            Controls.Add(layout);
            var trayMenu = GridLook.Menu();
            trayMenu.Items.Add(GridLook.Item("管理桌面格子", app.ShowManager));
            trayMenu.Items.Add(GridLook.Item("全局搜索", app.ShowSearch));
            trayMenu.Items.Add(app.MenuItem("关闭格子并恢复桌面", () => app.Stop(true)));
            tray = new NotifyIcon { Icon = SystemIcons.Application, Text = "SoulDesk 桌面格子",
                ContextMenuStrip = trayMenu, Visible = !app.Isolated };
            tray.DoubleClick += (sender, args) => app.ShowManager();
        }

        protected override void OnFormClosing(FormClosingEventArgs e)
        {
            if (e.CloseReason == CloseReason.UserClosing || e.CloseReason == CloseReason.TaskManagerClosing)
            {
                e.Cancel = true;
                if (app.Isolated) BeginInvoke(new Action(() => app.Stop(true)));
                else Hide();
            }
            base.OnFormClosing(e);
        }

        internal void RefreshState()
        {
            status.Text = app.Status + (!app.Isolated && !app.Hotkey ? " · 搜索快捷键未注册，可从菜单进入" : "");
            toggle.Text = app.Active ? "暂停格子" : "启用格子";
            undo.Enabled = app.Store.CanUndo;
            var selected = list.SelectedItem as GridRow;
            int index = list.SelectedIndex;
            int top = list.TopIndex;
            refreshing = true;
            list.BeginUpdate();
            list.Items.Clear();
            foreach (var box in app.Store.State.Boxes)
                foreach (var tab in box.Tabs)
                {
                    int row = list.Items.Add(new GridRow { Tab = tab, Box = box });
                    if (selected != null && selected.Tab.Id == tab.Id) list.SelectedIndex = row;
                }
            if (list.Items.Count != 0)
            {
                if (list.SelectedIndex < 0) list.SelectedIndex = Math.Max(0, Math.Min(index, list.Items.Count - 1));
                list.TopIndex = Math.Min(top, list.Items.Count - 1);
            }
            list.EndUpdate();
            refreshing = false;
            empty.Visible = list.Items.Count == 0;
            UpdateSelection();
        }

        private void UpdateSelection()
        {
            var row = list.SelectedItem as GridRow;
            displayMode.Enabled = row != null;
            displayMode.SelectedIndex = row == null ? -1 : Array.IndexOf(GridBox.CollapseModes, row.Box.DisplayMode);
        }

        private void PickSelected()
        {
            var row = list.SelectedItem as GridRow;
            if (row != null && row.Tab.Folder == null) PickItems(app, this, row.Tab.Id);
        }

        private static Button ActionButton(string text, Action action, bool primary = false)
        {
            var button = GridLook.Button(text, action, primary ? Accent : Card);
            button.ForeColor = primary ? Color.FromArgb(20, 35, 49) : Ink;
            button.Padding = new Padding(10, 0, 10, 0);
            button.MinimumSize = new Size(42, 32);
            button.FlatAppearance.MouseOverBackColor = primary ? Color.FromArgb(173, 213, 251) : Selected;
            return button;
        }

        private static Control SettingRow(string title, string description, params Button[] buttons)
        {
            var row = new Panel { BackColor = Card, Height = 72, Width = 415, Margin = new Padding(0, 0, 0, 6) };
            row.Controls.Add(new Label { Dock = DockStyle.Fill, Text = title + "\r\n" + description,
                TextAlign = ContentAlignment.MiddleLeft, AutoEllipsis = true, Padding = new Padding(13, 0, 6, 0) });
            var actions = new FlowLayoutPanel { Dock = DockStyle.Right, AutoSize = true, WrapContents = false,
                Padding = new Padding(4, 16, 8, 0) };
            actions.Controls.AddRange(buttons);
            row.Controls.Add(actions);
            return row;
        }

        private void DrawGrid(object sender, DrawItemEventArgs e)
        {
            if (e.Index < 0 || e.Index >= list.Items.Count) return;
            var row = (GridRow)list.Items[e.Index];
            var bounds = e.Bounds;
            bounds.Height -= 4;
            bool selected = (e.State & DrawItemState.Selected) != 0;
            using (var background = new SolidBrush(selected ? Selected : Card))
                e.Graphics.FillRectangle(background, bounds);
            if (selected)
                using (var indicator = new SolidBrush(Accent))
                    e.Graphics.FillRectangle(indicator, bounds.X, bounds.Y + 13, 3, bounds.Height - 26);
            var line = new Rectangle(bounds.X + 15, bounds.Y + 6, bounds.Width - 28, 22);
            var flags = TextFormatFlags.EndEllipsis | TextFormatFlags.NoPrefix | TextFormatFlags.SingleLine;
            TextRenderer.DrawText(e.Graphics, row.Tab.Name, Font, line, Ink, flags);
            line.Offset(0, 21);
            TextRenderer.DrawText(e.Graphics, row.Description, Font, line, Secondary, flags);
            if ((e.State & DrawItemState.Focus) != 0)
                ControlPaint.DrawFocusRectangle(e.Graphics, Rectangle.Inflate(bounds, -3, -2), Ink, Selected);
        }

        internal static void CreateGrid(GridApplication app, IWin32Window owner, bool mapped)
        {
            string folder = null;
            if (mapped)
            {
                using (var picker = new FolderBrowserDialog { Description = "选择目录。格子会显示该目录中的项目。",
                    ShowNewFolderButton = false })
                {
                    if (picker.ShowDialog(owner) != DialogResult.OK) return;
                    folder = picker.SelectedPath;
                }
                if (!app.TestPath(folder)) { app.Report("请选择可访问的本地目录。"); return; }
            }
            string name = GridLook.Prompt(owner, mapped ? "映射目录" : "新建格子", "输入格子名称",
                mapped ? new DirectoryInfo(folder).Name : "新格子", 64);
            if (!string.IsNullOrWhiteSpace(name))
                app.Edit(state => state.Add(name, folder).SetCollapseMode(GridBox.SystemDefault), true);
        }

        internal void OpenGridAction(string action, string tabId)
        {
            if (action == "search")
            {
                app.ShowSearch();
                return;
            }
            app.ShowManager();
            if (action == "organization") selectPage(1);
            else selectPage(0);
            var row = list.Items.Cast<GridRow>().FirstOrDefault(item => item.Tab.Id == tabId);
            if (row != null) list.SelectedItem = row;
            switch (action)
            {
                case "new": CreateGrid(app, this, false); break;
                case "map": CreateGrid(app, this, true); break;
                case "pick": PickSelected(); break;
                case "rules": EditRules(app, this, tabId); break;
                case "appearance": if (row != null) EditAppearance(app, this, row.Box.Id); break;
                case "stack-rules": StackRules(); break;
            }
        }

        private void StackRules()
        {
            var copy = LocalJson.Copy(app.Store.State.StackRules);
            using (var dialog = new Form { Text = "自定义堆栈规则", ClientSize = new Size(620, 386),
                StartPosition = FormStartPosition.CenterParent, BackColor = Canvas, ForeColor = Ink, ShowInTaskbar = false })
            {
                var list = new ListBox { Dock = DockStyle.Fill, BackColor = Card, ForeColor = Ink, IntegralHeight = false };
                Action<int> refresh = index =>
                {
                    list.Items.Clear();
                    list.Items.AddRange(copy.ToArray());
                    if (copy.Count > 0) list.SelectedIndex = Math.Max(0, Math.Min(index, copy.Count - 1));
                };
                Action<bool> edit = creating =>
                {
                    var rule = creating ? new GridStackRule() : list.SelectedItem as GridStackRule;
                    if (rule == null || creating && copy.Count >= 64) return;
                    string name = GridLook.Prompt(dialog, "堆栈名称", "输入规则名称", rule.Name, 64);
                    if (string.IsNullOrWhiteSpace(name)) return;
                    string text = GridLook.Prompt(dialog, "扩展名", "用逗号分隔，例如 pdf, docx, txt",
                        string.Join(", ", rule.Extensions), 512);
                    if (text == null) return;
                    var extensions = text.Split(new[] { ',', '，', ';', '；', ' ' }, StringSplitOptions.RemoveEmptyEntries)
                        .Select(ext => ext.Trim().TrimStart('.').ToLowerInvariant()).Distinct().ToList();
                    if (extensions.Count == 0 || extensions.Count > 64 || extensions.Any(ext =>
                        !System.Text.RegularExpressions.Regex.IsMatch(ext, "^[a-z0-9_+-]{1,24}$")))
                    {
                        MessageBox.Show(dialog, "请输入有效的扩展名。", "自定义堆栈");
                        return;
                    }
                    rule.Name = name;
                    rule.Extensions = extensions;
                    if (creating) copy.Add(rule);
                    refresh(copy.IndexOf(rule));
                };
                Action<int> move = offset =>
                {
                    int from = list.SelectedIndex, to = from + offset;
                    if (from < 0 || to < 0 || to >= copy.Count) return;
                    var item = copy[from];
                    copy.RemoveAt(from);
                    copy.Insert(to, item);
                    refresh(to);
                };
                var bar = new FlowLayoutPanel { Dock = DockStyle.Bottom, Height = 48 };
                bar.Controls.AddRange(new Control[] { ActionButton("新增", () => edit(true)), ActionButton("编辑", () => edit(false)),
                    ActionButton("上移", () => move(-1)), ActionButton("下移", () => move(1)),
                    ActionButton("移除规则", () =>
                    {
                        int index = list.SelectedIndex;
                        if (index < 0) return;
                        copy.RemoveAt(index);
                        refresh(index);
                    }),
                    ActionButton("保存", () => { app.Edit(state => state.StackRules = copy, true); dialog.Close(); }, true) });
                dialog.Controls.Add(list);
                dialog.Controls.Add(bar);
                refresh(0);
                dialog.ShowDialog(this);
            }
        }

        private ToolStripMenuItem BindingMenu(GridBox box)
        {
            var menu = new ToolStripMenuItem("显示器绑定");
            Action<string, string> add = (name, monitor) =>
            {
                var item = app.MenuItem(name, () =>
                    app.Edit(state => state.Boxes.Single(entry => entry.Id == box.Id).BoundMonitor = monitor, true));
                item.Checked = box.BoundMonitor == monitor;
                menu.DropDownItems.Add(item);
            };
            add("跟随拖动", null);
            add("始终在主屏", "@primary");
            var displays = app.DisplayChoices();
            foreach (var display in displays) add(display.Name, display.Id);
            if (box.BoundMonitor != null && box.BoundMonitor != "@primary" && displays.All(item => item.Id != box.BoundMonitor))
                menu.DropDownItems.Add(new ToolStripMenuItem("已绑定的显示器离线") { Checked = true, Enabled = false });
            return menu;
        }

        private void LayoutOptions()
        {
            using (var dialog = new Form { Text = "桌面布局设置", ClientSize = new Size(495, 235),
                StartPosition = FormStartPosition.CenterParent, BackColor = Canvas, ForeColor = Ink, ShowInTaskbar = false })
            {
                var state = app.Store.State;
                var rows = new FlowLayoutPanel { Dock = DockStyle.Fill, FlowDirection = FlowDirection.TopDown,
                    WrapContents = false, Padding = new Padding(16) };
                var coordinated = new CheckBox { Text = "按住 Ctrl 拖动同屏未锁定的格子", Checked = state.CoordinatedMove, AutoSize = true };
                var arrangement = new ComboBox { Width = 135, DropDownStyle = ComboBoxStyle.DropDownList, AccessibleName = "胶囊栏布局" };
                var modes = new[] { null, "Horizontal", "Vertical" };
                arrangement.Items.AddRange(new[] { "自由摆放", "横向排列", "纵向排列" });
                arrangement.SelectedIndex = Array.IndexOf(modes, state.CapsuleBar);
                var spacing = new NumericUpDown { Minimum = 0, Maximum = 32, Value = state.CapsuleSpacing, Width = 104,
                    AccessibleName = "胶囊间距" };
                rows.Controls.Add(coordinated);
                rows.Controls.Add(SettingRow("胶囊栏", "将同屏已折叠的格子集中排列。"));
                var options = new FlowLayoutPanel { AutoSize = true, WrapContents = false };
                options.Controls.AddRange(new Control[] { arrangement, new Label { Text = "间距", AutoSize = true,
                    Margin = new Padding(12, 6, 3, 0) }, spacing });
                rows.Controls.Add(options);
                var save = ActionButton("保存", () =>
                {
                    app.Edit(copy =>
                    {
                        copy.CoordinatedMove = coordinated.Checked;
                        copy.CapsuleBar = modes[arrangement.SelectedIndex];
                        copy.CapsuleSpacing = (int)spacing.Value;
                    });
                    dialog.Close();
                }, true);
                rows.Controls.Add(save);
                dialog.AcceptButton = save;
                dialog.Controls.Add(rows);
                dialog.ShowDialog(this);
            }
        }

        internal static void EditRules(GridApplication app, IWin32Window owner, string tabId)
        {
            var tab = app.Store.State.Tabs().SingleOrDefault(item => item.Id == tabId);
            if (tab == null) return;
            string rule = GridLook.Prompt(owner, "自动归类规则", "用分号分隔规则，例如 *.lnk;报告*.pdf", tab.Rules, 512);
            if (rule != null)
                app.Edit(state => state.Tabs().Single(item => item.Id == tabId).Rules = rule, true);
        }

        internal static void EditAppearance(GridApplication app, IWin32Window owner, string boxId)
        {
            var box = app.Store.State.Boxes.SingleOrDefault(item => item.Id == boxId);
            if (box == null) return;
            using (var picker = new ColorDialog { Color = ColorTranslator.FromHtml(box.Color), FullOpen = true })
            {
                if (picker.ShowDialog(owner) != DialogResult.OK) return;
                string value = GridLook.Prompt(owner, "格子透明度", "输入 35–100。100 完全不透明。", box.Opacity.ToString(), 3);
                if (value == null) return;
                int opacity;
                if (!int.TryParse(value, out opacity) || opacity < 35 || opacity > 100)
                {
                    app.Report("格子透明度必须在 35–100 之间。");
                    return;
                }
                app.Edit(state =>
                {
                    var copy = state.Boxes.Single(item => item.Id == boxId);
                    copy.Color = string.Format("#{0:X2}{1:X2}{2:X2}", picker.Color.R, picker.Color.G, picker.Color.B);
                    copy.Opacity = opacity;
                }, true);
            }
        }

        internal static void RemoveGrid(GridApplication app, IWin32Window owner, string box, string tab)
        {
            if (MessageBox.Show(owner, "删除这个格子的布局。格子里的文件会保留在原位置。", "删除格子",
                MessageBoxButtons.OKCancel, MessageBoxIcon.Question, MessageBoxDefaultButton.Button2) == DialogResult.OK)
                app.Edit(state => state.RemoveTab(box, tab), true);
        }

        private void Categorize(bool move)
        {
            var changes = app.Propose(app.DesktopItems(), move);
            if (changes.Count == 0)
            {
                app.Report("没有匹配项。可从格子菜单设置自动归类规则。");
                return;
            }
            var rows = move ? app.Organization.Plan(changes).Select(item =>
                item.Source + "  →  " + item.Destination).ToArray() :
                changes.Select(change => change.Key.Name + "  →  " + change.Value.Name).ToArray();
            using (var dialog = Selection(move ? "搬移预览 · 保留整理历史" : "一键整理预览", rows, checkboxes: true))
            {
                var checkedRows = (CheckedListBox)dialog.Pick;
                for (int i = 0; i < rows.Length; i++) checkedRows.SetItemChecked(i, true);
                dialog.Confirm.Text = move ? "搬移勾选的原文件" : "应用分组";
                if (dialog.ShowDialog(this) == DialogResult.OK)
                {
                    var selected = checkedRows.CheckedIndices.Cast<int>().Select(index => changes[index])
                        .ToDictionary(change => change.Key.Id, change => change.Value.Id, StringComparer.OrdinalIgnoreCase);
                    var current = app.Propose(app.DesktopItems(), move).Where(change =>
                        selected.ContainsKey(change.Key.Id) && selected[change.Key.Id] == change.Value.Id).ToList();
                    if (current.Count > 0)
                    {
                        if (move) app.MoveOriginals(current);
                        else app.Edit(state => GridApplication.Apply(state, current), true);
                    }
                }
            }
        }

        private void ShowOrganizationHistory()
        {
            using (var dialog = new Form { Text = "整理历史", ClientSize = new Size(930, 440), MinimumSize = new Size(650, 360),
                StartPosition = FormStartPosition.CenterParent, BackColor = Canvas, ForeColor = Ink, ShowInTaskbar = false })
            {
                var rows = new ListView { Dock = DockStyle.Fill, FullRowSelect = true, MultiSelect = false, View = View.Details,
                    BackColor = Card, ForeColor = Ink, BorderStyle = BorderStyle.None, HideSelection = false };
                rows.Columns.Add("状态", 128);
                rows.Columns.Add("时间", 133);
                rows.Columns.Add("原路径", 285);
                rows.Columns.Add("目标路径", 340);
                var details = new Label { Dock = DockStyle.Bottom, Height = 43, TextAlign = ContentAlignment.MiddleLeft };
                var bar = new FlowLayoutPanel { Dock = DockStyle.Bottom, Height = 45 };
                Action refresh = () =>
                {
                    rows.Items.Clear();
                    foreach (var item in app.Organization.Read())
                    {
                        string status = item.Status == "moved" ? "已搬移" : item.Status == "restored" ? "已恢复" :
                            item.Status == "failed" ? "失败" : "待核对";
                        rows.Items.Add(new ListViewItem(new[] { status, item.Timestamp.ToLocalTime().ToString("MM-dd HH:mm:ss"),
                            item.Source, item.Destination }) { Tag = item });
                    }
                };
                var restore = ActionButton("恢复到原路径", () => { }, true);
                restore.Enabled = false;
                restore.Click += async (sender, args) =>
                {
                    if (rows.SelectedItems.Count == 0) return;
                    var item = (OrganizedItem)rows.SelectedItems[0].Tag;
                    restore.Enabled = false;
                    try { await app.RestoreOrganizedItem(item.Id); if (!dialog.IsDisposed) refresh(); }
                    catch (Exception error) { if (!dialog.IsDisposed) details.Text = error.Message; }
                };
                rows.SelectedIndexChanged += (sender, args) =>
                {
                    var item = rows.SelectedItems.Count == 0 ? null : (OrganizedItem)rows.SelectedItems[0].Tag;
                    restore.Enabled = item != null && item.Status == "moved";
                    details.Text = item == null ? "" : item.Error ?? item.Title;
                };
                bar.Controls.AddRange(new Control[] { restore, ActionButton("刷新", refresh),
                    ActionButton("清理已恢复和失败记录", () =>
                    {
                        if (MessageBox.Show(dialog, "清理这些历史记录。", "整理历史", MessageBoxButtons.OKCancel,
                            MessageBoxIcon.Question) != DialogResult.OK) return;
                        if (app.IsOrganizing) { details.Text = "正在处理整理记录，请稍后。"; return; }
                        app.Organization.ClearFinished();
                        refresh();
                    }) });
                dialog.Controls.Add(rows);
                dialog.Controls.Add(details);
                dialog.Controls.Add(bar);
                try { refresh(); dialog.ShowDialog(this); }
                catch (Exception error) { app.Report(error.Message); }
            }
        }

        internal static void PickItems(GridApplication app, IWin32Window owner, string tab)
        {
            var items = app.DesktopItems().Where(item => app.TestPath(item.Id))
                .OrderBy(item => item.Name, StringComparer.CurrentCultureIgnoreCase).ToList();
            using (var dialog = Selection("添加桌面项目", items.Select(item => item.Name + "  ·  " + item.Id).ToArray()))
                if (dialog.ShowDialog(owner) == DialogResult.OK)
                    app.Group(tab, dialog.Pick.SelectedIndices.Cast<int>().Select(index => items[index].Id).ToArray());
        }

        private static Picker Selection(string title, string[] rows, bool checkboxes = false)
        {
            var window = new Picker { Text = title, ClientSize = new Size(558, 357), StartPosition = FormStartPosition.CenterParent,
                BackColor = GridLook.Canvas, ForeColor = GridLook.Text, Font = GridLook.Font(), MinimizeBox = false, ShowInTaskbar = false };
            window.Pick = checkboxes ? new CheckedListBox { CheckOnClick = true } :
                new ListBox { SelectionMode = SelectionMode.MultiExtended };
            window.Pick.Dock = DockStyle.Fill;
            window.Pick.HorizontalScrollbar = true;
            window.Pick.BackColor = GridLook.Panel;
            window.Pick.ForeColor = GridLook.Text;
            window.Pick.Items.AddRange(rows);
            var controls = new FlowLayoutPanel { FlowDirection = FlowDirection.RightToLeft, Dock = DockStyle.Bottom, Height = 46 };
            window.Confirm = GridLook.Button("添加选中项目", () => window.DialogResult = DialogResult.OK, GridLook.Accent);
            controls.Controls.Add(window.Confirm);
            var cancel = GridLook.Button("取消", () => window.DialogResult = DialogResult.Cancel);
            controls.Controls.Add(cancel);
            window.Controls.Add(window.Pick);
            window.Controls.Add(controls);
            window.CancelButton = cancel;
            window.AcceptButton = window.Confirm;
            return window;
        }

        private void Transfer(bool exporting)
        {
            using (FileDialog dialog = exporting ? (FileDialog)new SaveFileDialog { FileName = "SoulDesk-桌面格子.json" } : new OpenFileDialog())
            {
                dialog.Filter = "格子布局 (*.json)|*.json";
                dialog.DefaultExt = "json";
                if (dialog.ShowDialog(this) != DialogResult.OK) return;
                if (exporting) app.Export(dialog.FileName);
                else app.Import(dialog.FileName);
            }
        }

        private void Options()
        {
            using (var dialog = new Form { Text = "桌面格子选项", ClientSize = new Size(535, 280), BackColor = GridLook.Canvas,
                ForeColor = GridLook.Text, StartPosition = FormStartPosition.CenterParent, Font = GridLook.Font(), ShowInTaskbar = false })
            {
                var state = app.Store.State;
                var settings = new FlowLayoutPanel { Dock = DockStyle.Fill, FlowDirection = FlowDirection.TopDown,
                    WrapContents = false, Padding = new Padding(14) };
                var auto = new CheckBox { AutoSize = true, Text = "按规则自动归类新桌面项目", Checked = state.AutoSort };
                var shared = new CheckBox { AutoSize = true, Text = "整理时包含公共桌面项目", Checked = state.IncludePublic };
                var indexed = new CheckBox { AutoSize = true, Text = "使用 Windows 搜索索引", Checked = state.SearchIndexed, Enabled = !app.Isolated };
                var collapse = new ComboBox { Width = 150, DropDownStyle = ComboBoxStyle.DropDownList,
                    AccessibleName = "默认展开方式", BackColor = GridLook.Panel, ForeColor = GridLook.Text };
                collapse.Items.AddRange(GridBox.CollapseLabels.Skip(1).ToArray());
                collapse.SelectedIndex = Array.IndexOf(GridBox.CollapseModes, state.DefaultCollapseMode) - 1;
                var collapseRow = new FlowLayoutPanel { AutoSize = true, WrapContents = false };
                collapseRow.Controls.Add(new Label { Text = "默认展开方式", AutoSize = true, Padding = new Padding(0, 6, 0, 0) });
                collapseRow.Controls.Add(collapse);
                var path = new Label { AutoEllipsis = true, Width = 480, Height = 24,
                    Text = state.EverythingCli ?? "Everything 未配置", ForeColor = GridLook.Muted };
                string everything = state.EverythingCli;
                var buttons = new FlowLayoutPanel { AutoSize = true, WrapContents = false };
                buttons.Controls.Add(GridLook.Button("选择 Everything es.exe", () =>
                {
                    using (var picker = new OpenFileDialog { FileName = "es.exe", Filter = "Everything CLI (es.exe)|es.exe" })
                        if (picker.ShowDialog(dialog) == DialogResult.OK && References.IsLocalPath(picker.FileName) &&
                            References.Equal(Path.GetFileName(picker.FileName), "es.exe"))
                            path.Text = everything = picker.FileName;
                }));
                buttons.Controls.Add(GridLook.Button("清除", () => { everything = null; path.Text = "Everything 未配置"; }));
                buttons.Enabled = !app.Isolated;
                var save = GridLook.Button("保存", () =>
                {
                    app.Edit(copy => { copy.AutoSort = auto.Checked; copy.IncludePublic = shared.Checked;
                        copy.SearchIndexed = indexed.Checked; copy.EverythingCli = everything;
                        copy.SetDefaultCollapseMode(GridBox.CollapseModes[collapse.SelectedIndex + 1]); });
                    dialog.Close();
                }, GridLook.Accent);
                settings.Controls.AddRange(new Control[] { collapseRow, auto, shared, indexed, path, buttons, save });
                dialog.Controls.Add(settings);
                dialog.AcceptButton = save;
                dialog.ShowDialog(this);
            }
        }

        protected override void Dispose(bool disposing)
        {
            if (disposing && !resourcesDisposed)
            {
                resourcesDisposed = true;
                if (tray != null)
                {
                    tray.Visible = false;
                    if (tray.ContextMenuStrip != null)
                        tray.ContextMenuStrip.Dispose();
                    tray.Dispose();
                }
                Font.Dispose();
            }
            base.Dispose(disposing);
        }

        private sealed class Picker : Form
        {
            internal ListBox Pick;
            internal Button Confirm;
        }

        private sealed class GridRow
        {
            internal GridTab Tab;
            internal GridBox Box;
            internal string Description
            {
                get
                {
                    return (Box.Hidden ? "已隐藏  ·  " : "") + (Box.Tabs.Count > 1 ? "格子组  ·  " : "") +
                        (Tab.Folder == null ? Tab.Items.Count + " 个项目" : Tab.Folder);
                }
            }
            public override string ToString() { return Tab.Name + "  ·  " + Description; }
        }
    }

    internal sealed class GridSearch : Form
    {
        private readonly GridApplication app;
        private readonly TextBox query;
        private readonly ListBox results;
        private readonly Label status;
        private readonly Timer delay;
        private int generation;
        private bool searching;

        internal GridSearch(GridApplication app)
        {
            this.app = app;
            Text = "SoulDesk · 全局搜索";
            ClientSize = new Size(680, 440);
            MinimumSize = new Size(450, 220);
            StartPosition = FormStartPosition.CenterScreen;
            BackColor = GridLook.Canvas;
            ForeColor = GridLook.Text;
            Font = GridLook.Font(11);
            query = new TextBox { AccessibleName = "搜索文件或快捷方式", BorderStyle = BorderStyle.FixedSingle,
                Dock = DockStyle.Fill, BackColor = GridLook.Panel, ForeColor = GridLook.Text, MaxLength = 120 };
            var top = new Panel { Dock = DockStyle.Top, Height = 54, Padding = new Padding(15, 14, 15, 8) };
            top.Controls.Add(query);
            status = new Label { Dock = DockStyle.Bottom, Height = 34, Padding = new Padding(15, 7, 6, 0), ForeColor = GridLook.Muted,
                Text = "输入文件名或路径。Enter 打开，Esc 关闭。" };
            results = new ListBox { Dock = DockStyle.Fill, BackColor = GridLook.Panel, ForeColor = GridLook.Text,
                BorderStyle = BorderStyle.None, HorizontalScrollbar = true };
            var body = new Panel { Dock = DockStyle.Fill, Padding = new Padding(15, 0, 15, 0) };
            body.Controls.Add(results);
            Controls.Add(body);
            Controls.Add(status);
            Controls.Add(top);
            delay = new Timer { Interval = 235 };
            delay.Tick += (sender, args) => { delay.Stop(); Search(); };
            query.TextChanged += (sender, args) => { generation++; delay.Stop(); delay.Start(); };
            query.KeyDown += Navigate;
            results.KeyDown += Navigate;
            results.DoubleClick += (sender, args) => OpenResult();
            Shown += (sender, args) => query.Focus();
        }

        private void Navigate(object sender, KeyEventArgs args)
        {
            switch (args.KeyCode)
            {
                case Keys.Escape: Close(); break;
                case Keys.Enter: OpenResult(); break;
                case Keys.Down: if (results.Items.Count != 0) results.SelectedIndex = Math.Min(results.Items.Count - 1, results.SelectedIndex + 1); break;
                case Keys.Up: if (results.SelectedIndex > 0) results.SelectedIndex--; break;
                default: return;
            }
            args.Handled = true;
            args.SuppressKeyPress = true;
        }

        private void OpenResult()
        {
            var item = results.SelectedItem as SearchResult;
            if (item != null) app.Open(item.Path);
        }

        private async void Search()
        {
            string term = query.Text.Trim();
            int request = generation;
            if (searching)
            {
                delay.Start();
                return;
            }
            results.Items.Clear();
            if (term.Length == 0)
            {
                status.Text = "输入文件名或路径。Enter 打开，Esc 关闭。";
                return;
            }
            status.Text = "正在搜索…";
            var local = app.DesktopItems().Select(item => item.Id).Concat(app.Store.State.Tabs()
                .SelectMany(tab => tab.Items.Concat(tab.Folder == null ? new string[0] : new[] { tab.Folder })))
                .Where(app.TestPath).ToArray();
            bool isolated = app.Isolated;
            bool indexed = !isolated && app.Store.State.SearchIndexed;
            string everything = isolated ? null : app.Store.State.EverythingCli;
            searching = true;
            SearchBatch search;
            try { search = await Task.Run(() => SearchIndex.Find(term, local, isolated, indexed, everything)); }
            finally { searching = false; }
            if (IsDisposed || request != generation) return;
            results.Items.AddRange(search.Items.ToArray());
            if (results.Items.Count > 0) results.SelectedIndex = 0;
            status.Text = results.Items.Count + " 项" + (string.IsNullOrEmpty(search.Note) ? "" : "  ·  " + search.Note);
        }

        protected override void Dispose(bool disposing)
        {
            if (disposing)
            {
                delay.Dispose();
                Font.Dispose();
            }
            base.Dispose(disposing);
        }
    }

    internal sealed class SearchResult
    {
        internal string Path;
        public override string ToString() { return System.IO.Path.GetFileName(Path) + "  ·  " + Path; }
    }

    internal sealed class SearchBatch
    {
        internal readonly List<SearchResult> Items = new List<SearchResult>();
        internal string Note;
    }
}
