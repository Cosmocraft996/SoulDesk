using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Text;
using System.IO;
using System.Linq;
using System.Windows.Forms;

namespace SoulDesk.DesktopGrid
{
    internal static class GridLook
    {
        internal static readonly Color Canvas = Color.FromArgb(28, 32, 34);
        internal static readonly Color Panel = Color.FromArgb(43, 50, 51);
        internal static readonly Color Line = Color.FromArgb(83, 99, 99);
        internal static readonly Color Text = Color.FromArgb(243, 248, 247);
        internal static readonly Color Muted = Color.FromArgb(180, 199, 197);
        internal static readonly Color Accent = Color.FromArgb(57, 94, 90);

        internal static Font Font(float size = 10f, FontStyle style = FontStyle.Regular)
        {
            return new Font("Microsoft YaHei UI", size, style);
        }

        internal static Button Button(string title, Action action, Color? background = null)
        {
            var button = new Button { Text = title, AutoSize = true, MinimumSize = new Size(30, 29),
                FlatStyle = FlatStyle.Flat, BackColor = background ?? Panel, ForeColor = Text,
                Margin = new Padding(3), Padding = new Padding(4, 0, 4, 0), UseVisualStyleBackColor = false };
            button.FlatAppearance.BorderSize = 0;
            button.FlatAppearance.MouseOverBackColor = Accent;
            button.Click += (sender, args) => action();
            return button;
        }

        internal static ToolStripMenuItem Item(string title, Action action)
        {
            var item = new ToolStripMenuItem(title) { ForeColor = Text };
            item.Click += (sender, args) => action();
            return item;
        }

        internal static ContextMenuStrip Menu()
        {
            return new ContextMenuStrip { BackColor = Canvas, ForeColor = Text, ShowImageMargin = false,
                Padding = new Padding(4), Renderer = new ToolStripProfessionalRenderer(new MenuColors()) { RoundedEdges = false } };
        }

        private sealed class MenuColors : ProfessionalColorTable
        {
            public override Color ToolStripDropDownBackground { get { return Canvas; } }
            public override Color MenuBorder { get { return Line; } }
            public override Color MenuItemBorder { get { return Accent; } }
            public override Color MenuItemSelected { get { return Accent; } }
            public override Color MenuItemSelectedGradientBegin { get { return Accent; } }
            public override Color MenuItemSelectedGradientEnd { get { return Accent; } }
            public override Color MenuItemPressedGradientBegin { get { return Panel; } }
            public override Color MenuItemPressedGradientMiddle { get { return Panel; } }
            public override Color MenuItemPressedGradientEnd { get { return Panel; } }
            public override Color SeparatorDark { get { return Line; } }
            public override Color SeparatorLight { get { return Line; } }
        }

        internal static string Prompt(IWin32Window owner, string title, string caption, string initial, int limit)
        {
            using (var window = new Form { Text = title, ClientSize = new Size(448, 145),
                BackColor = Canvas, ForeColor = Text, StartPosition = FormStartPosition.CenterParent,
                FormBorderStyle = FormBorderStyle.FixedDialog, MinimizeBox = false, MaximizeBox = false,
                ShowInTaskbar = false })
            {
                var label = new Label { Text = caption, AutoSize = true, Location = new Point(16, 17) };
                var input = new TextBox { Text = initial ?? "", MaxLength = limit, Location = new Point(16, 48),
                    Width = 414, BackColor = Panel, ForeColor = Text };
                var okay = Button("保存", () => window.DialogResult = DialogResult.OK, Accent);
                var cancel = Button("取消", () => window.DialogResult = DialogResult.Cancel);
                okay.Location = new Point(290, 92);
                cancel.Location = new Point(361, 92);
                window.Controls.AddRange(new Control[] { label, input, okay, cancel });
                window.AcceptButton = okay;
                window.CancelButton = cancel;
                return window.ShowDialog(owner) == DialogResult.OK ? input.Text.Trim() : null;
            }
        }

        internal static GraphicsPath Rounded(Rectangle bounds, int radius)
        {
            int diameter = Math.Max(1, Math.Min(Math.Min(bounds.Width, bounds.Height), radius * 2));
            var path = new GraphicsPath();
            path.AddArc(bounds.Left, bounds.Top, diameter, diameter, 180, 90);
            path.AddArc(bounds.Right - diameter, bounds.Top, diameter, diameter, 270, 90);
            path.AddArc(bounds.Right - diameter, bounds.Bottom - diameter, diameter, diameter, 0, 90);
            path.AddArc(bounds.Left, bounds.Bottom - diameter, diameter, diameter, 90, 90);
            path.CloseFigure();
            return path;
        }
    }

    internal abstract class DesktopSurface : Form
    {
        protected IntPtr DesktopParent;
        internal Rectangle ScreenBounds { get; private set; }
        protected DesktopSurface()
        {
            FormBorderStyle = FormBorderStyle.None;
            ShowInTaskbar = false;
            StartPosition = FormStartPosition.Manual;
            AutoScaleMode = AutoScaleMode.None;
            DoubleBuffered = true;
            BackColor = GridLook.Canvas;
            ForeColor = GridLook.Text;
        }
        protected override bool ShowWithoutActivation { get { return true; } }
        internal void Prepare(IntPtr parent, Rectangle rectangle)
        {
            DesktopParent = parent;
            MoveTo(rectangle);
        }
        internal void MoveTo(Rectangle rectangle)
        {
            ScreenBounds = rectangle;
            if (DesktopParent == IntPtr.Zero)
                Bounds = rectangle;
            else
            {
                var previous = Native.MatchWindowDpi(DesktopParent);
                try { Native.Attach(Handle, DesktopParent, rectangle); }
                finally { Native.RestoreDpi(previous); }
            }
        }
        internal void ShowSurface()
        {
            if (Visible) return;
            Show();
            if (DesktopParent != IntPtr.Zero)
                Native.Place(Handle, DesktopParent, ScreenBounds, true);
        }
    }

    internal sealed class LooseSurface : DesktopSurface
    {
        private readonly GridApplication app;
        private readonly Dictionary<string, GridFile> files = new Dictionary<string, GridFile>(StringComparer.OrdinalIgnoreCase);

        internal LooseSurface(GridApplication app)
        {
            this.app = app;
            BackColor = Color.Fuchsia;
            TransparencyKey = Color.Fuchsia;
        }

        internal void SetItems(Point screenOrigin, List<DesktopItem> items)
        {
            var ids = new HashSet<string>(items.Select(item => item.Id), StringComparer.OrdinalIgnoreCase);
            bool changed = false;
            foreach (string id in files.Keys.Where(id => !ids.Contains(id)).ToArray())
            {
                files[id].Dispose();
                files.Remove(id);
                changed = true;
            }
            foreach (var item in items)
            {
                var bounds = item.ProjectedBounds;
                bounds.Offset(-screenOrigin.X, -screenOrigin.Y);
                GridFile icon;
                if (!files.TryGetValue(item.Id, out icon))
                {
                    icon = new GridFile(app, item, false, true) { Bounds = bounds };
                    files.Add(item.Id, icon);
                    Controls.Add(icon);
                    changed = true;
                }
                if (icon.Bounds != bounds)
                {
                    icon.Bounds = bounds;
                    changed = true;
                }
                if (icon.Text != item.Name)
                {
                    icon.Text = icon.AccessibleName = item.Name;
                    icon.Invalidate();
                }
            }
            if (!changed) return;
            var area = new Region();
            area.MakeEmpty();
            foreach (var icon in files.Values) area.Union(icon.Bounds);
            var previous = Region;
            Region = area;
            if (previous != null) previous.Dispose();
        }
    }

    internal sealed class GridFile : Control
    {
        private readonly GridApplication app;
        private readonly DesktopItem item;
        private readonly bool list, loose;
        private readonly Action open;
        private readonly ContextMenuStrip menu;
        private readonly Font labelFont;
        private Bitmap icon;
        private Point down;
        private bool selected, hover, dragging;

        internal GridFile(GridApplication app, DesktopItem item, bool list, bool loose, Action open = null)
        {
            this.app = app;
            this.item = item;
            this.list = list;
            this.loose = loose;
            this.open = open ?? (() => app.Open(item.Id));
            Text = item.Name;
            AccessibleName = item.Name;
            AccessibleRole = AccessibleRole.ListItem;
            TabStop = true;
            Cursor = Cursors.Default;
            DoubleBuffered = true;
            SetStyle(ControlStyles.SupportsTransparentBackColor | ControlStyles.Selectable |
                ControlStyles.StandardClick | ControlStyles.StandardDoubleClick, true);
            BackColor = Color.Transparent;
            ForeColor = GridLook.Text;
            Font = labelFont = GridLook.Font(8.5f);
            icon = Native.ItemIcon(item.Id);
            menu = GridLook.Menu();
            menu.Items.Add(app.MenuItem("打开", this.open));
            if (References.IsLocalPath(item.Id))
            {
                menu.Items.Add(app.MenuItem("定位原文件", () => app.Reveal(item.Id)));
                menu.Items.Add(app.MenuItem("复制原路径", () => Clipboard.SetText(item.Id)));
                bool grouped = app.Store.State.Tabs().Any(tab => tab.Items.Contains(item.Id, StringComparer.OrdinalIgnoreCase));
                if (loose || grouped || app.DesktopItems().Any(entry => References.Equal(entry.Id, item.Id)))
                {
                    var targets = new ToolStripMenuItem("加入格子");
                    menu.Opening += (sender, args) =>
                    {
                        foreach (ToolStripItem previous in targets.DropDownItems.Cast<ToolStripItem>().ToArray())
                            previous.Dispose();
                        foreach (var tab in app.Store.State.Tabs().Where(tab => tab.Folder == null))
                        {
                            string id = tab.Id;
                            targets.DropDownItems.Add(app.MenuItem(tab.Name, () => app.Group(id, new[] { item.Id })));
                        }
                        targets.Enabled = targets.HasDropDownItems;
                    };
                    menu.Items.Add(targets);
                    if (grouped)
                        menu.Items.Add(app.MenuItem("移出格子", () => app.Edit(state => state.Release(new[] { item.Id }), true)));
                }
            }
            ContextMenuStrip = menu;
        }

        protected override void OnMouseEnter(EventArgs e) { hover = true; Invalidate(); base.OnMouseEnter(e); }
        protected override void OnMouseLeave(EventArgs e) { hover = false; Invalidate(); base.OnMouseLeave(e); }
        protected override void OnMouseDown(MouseEventArgs e)
        {
            if (e.Button == MouseButtons.Left)
            {
                Focus();
                selected = true;
                down = e.Location;
                Invalidate();
            }
            base.OnMouseDown(e);
        }
        protected override void OnLostFocus(EventArgs e) { selected = false; Invalidate(); base.OnLostFocus(e); }
        protected override void OnDoubleClick(EventArgs e) { open(); base.OnDoubleClick(e); }
        protected override void OnKeyDown(KeyEventArgs e)
        {
            if (e.KeyCode == Keys.Enter) { open(); e.Handled = true; }
            base.OnKeyDown(e);
        }
        protected override void OnMouseMove(MouseEventArgs e)
        {
            base.OnMouseMove(e);
            if (e.Button != MouseButtons.Left || dragging || !References.IsLocalPath(item.Id) ||
                Math.Abs(e.X - down.X) + Math.Abs(e.Y - down.Y) < 10)
                return;
            dragging = true;
            try
            {
                var data = new DataObject(DataFormats.FileDrop, new[] { item.Id });
                // 保留标准文件拖放，并为格子提供快捷方式自身的原路径。
                data.SetText("SoulDesk.DesktopGrid.Path/1|" + item.Id, TextDataFormat.UnicodeText);
                DoDragDrop(data, DragDropEffects.Link);
            }
            finally { dragging = false; }
        }
        protected override void OnPaint(PaintEventArgs e)
        {
            e.Graphics.SmoothingMode = SmoothingMode.AntiAlias;
            if (hover || selected)
            {
                using (var brush = new SolidBrush(GridLook.Accent))
                using (var shape = GridLook.Rounded(new Rectangle(1, 1, Width - 2, Height - 2), 6))
                    e.Graphics.FillPath(brush, shape);
            }
            int size = list ? Math.Min(22, Height - 4) : Math.Min(Math.Min(Width - 6, Height / 2 + 6), loose ? 52 : 36);
            if (icon != null && size > 0)
                e.Graphics.DrawImage(icon, list ? 8 : (Width - size) / 2, list ? (Height - size) / 2 : 3, size, size);
            var text = list ? new Rectangle(38, 0, Width - 44, Height) :
                new Rectangle(2, size + 4, Width - 4, Math.Max(0, Height - size - 5));
            var flags = TextFormatFlags.EndEllipsis | TextFormatFlags.NoPrefix |
                (list ? TextFormatFlags.Left | TextFormatFlags.VerticalCenter | TextFormatFlags.SingleLine :
                TextFormatFlags.HorizontalCenter | TextFormatFlags.NoPadding | TextFormatFlags.WordBreak | TextFormatFlags.TextBoxControl);
            if (loose)
                DrawDesktopLabel(e.Graphics, Text, Font, text);
            else
                TextRenderer.DrawText(e.Graphics, Text, Font, text, ForeColor, flags);
        }

        internal static void DrawDesktopLabel(Graphics graphics, string name, Font font, Rectangle bounds)
        {
            // 色键只能隐藏纯洋红，文字边缘不能与洋红背景混色。
            var hint = graphics.TextRenderingHint;
            graphics.TextRenderingHint = TextRenderingHint.SingleBitPerPixelGridFit;
            using (var format = new StringFormat { Alignment = StringAlignment.Center,
                Trimming = StringTrimming.EllipsisCharacter, FormatFlags = StringFormatFlags.LineLimit })
            {
                var shadow = bounds;
                shadow.Offset(1, 1);
                graphics.DrawString(name, font, Brushes.Black, shadow, format);
                graphics.DrawString(name, font, Brushes.White, bounds, format);
            }
            graphics.TextRenderingHint = hint;
        }
        protected override void Dispose(bool disposing)
        {
            if (disposing)
            {
                if (icon != null) icon.Dispose();
                if (!IsDisposed)
                {
                    ContextMenuStrip = null;
                    menu.Dispose();
                    labelFont.Dispose();
                }
            }
            base.Dispose(disposing);
        }
    }

    internal sealed class GridSurface : DesktopSurface
    {
        private readonly GridApplication app;
        private readonly GridTab tab;
        private readonly Label title;
        private readonly FlowLayoutPanel body;
        private readonly Panel header, tabs, footer;
        private readonly Button collapse;
        private readonly ContextMenuStrip options;
        private readonly Font surfaceFont;
        private readonly Timer hover;
        private readonly HashSet<string> openStacks = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        private string folder;
        private string folderSignature;
        private bool expanded, moving, resizing, dropActive;
        private Point pointer;
        private Rectangle start, natural;
        private int headerHeight = 38, capsuleWidth = 198;
        internal GridBox Box { get; private set; }
        private string ExpandMode
        {
            get { return Box.DisplayMode == GridBox.SystemDefault ? app.Store.State.DefaultCollapseMode : Box.DisplayMode; }
        }

        internal GridSurface(GridApplication app, GridBox box)
        {
            this.app = app;
            Box = box;
            tab = box.Current();
            folder = tab.Folder;
            Text = tab.Name + " · SoulDesk";
            AccessibleName = tab.Name;
            Font = surfaceFont = GridLook.Font(9.5f);
            BackColor = ColorTranslator.FromHtml(box.Color);
            SetStyle(ControlStyles.ResizeRedraw, true);
            Padding = new Padding(1);
            header = new Panel { Dock = DockStyle.Top, Height = headerHeight, BackColor = Color.Transparent };
            title = new Label { Dock = DockStyle.Fill, TextAlign = ContentAlignment.MiddleLeft,
                BackColor = Color.Transparent, Padding = new Padding(12, 0, 4, 0), AutoEllipsis = true,
                Text = tab.Name, Cursor = box.LocksPosition ? Cursors.Default : Cursors.SizeAll };
            var commands = new FlowLayoutPanel { Dock = DockStyle.Right, FlowDirection = FlowDirection.RightToLeft,
                BackColor = Color.Transparent, Width = 65, WrapContents = false, Padding = new Padding(0, 3, 4, 0) };
            var menu = CaptionButton("⋯", () => options.Show(commands, new Point(0, headerHeight - 2)));
            menu.AccessibleName = "格子菜单";
            collapse = CaptionButton(box.Collapsed ? "⌄" : "⌃", Flip);
            collapse.Visible = ExpandMode != GridBox.Expanded;
            commands.Controls.AddRange(new Control[] { menu, collapse });
            header.Controls.Add(title);
            header.Controls.Add(commands);
            title.DoubleClick += (sender, args) => Flip();
            title.MouseWheel += (sender, args) => CycleTab(args.Delta > 0 ? -1 : 1);
            AddDrag(title, false);
            AddDrop(title);
            body = new FlowLayoutPanel { Dock = DockStyle.Fill, AutoScroll = true, BackColor = Color.Transparent,
                WrapContents = !tab.List, FlowDirection = tab.List ? FlowDirection.TopDown : FlowDirection.LeftToRight,
                Padding = new Padding(8, 5, 8, 4) };
            AddDrop(body);
            tabs = new Panel { Dock = DockStyle.Top, Height = 34, BackColor = Color.Transparent };
            var tabStrip = new FlowLayoutPanel { Dock = DockStyle.Fill, WrapContents = false, AutoScroll = true,
                BackColor = Color.Transparent, Padding = new Padding(7, 0, 3, 2) };
            foreach (var other in box.Tabs)
            {
                string tabId = other.Id;
                var button = GridLook.Button(other.Name, () => ChangeBox(copy => copy.Active = tabId),
                    tabId == tab.Id ? GridLook.Accent : Color.Transparent);
                button.Margin = new Padding(2, 1, 2, 2);
                button.ForeColor = tabId == tab.Id ? GridLook.Text : GridLook.Muted;
                tabStrip.Controls.Add(button);
            }
            tabs.Controls.Add(tabStrip);
            footer = new Panel { Height = 17, Dock = DockStyle.Bottom, BackColor = Color.Transparent };
            var grip = CaptionButton("◢", () => { });
            grip.AccessibleName = "调整格子大小";
            grip.Dock = DockStyle.Right;
            grip.Cursor = Cursors.SizeNWSE;
            grip.TabStop = false;
            footer.Controls.Add(grip);
            AddDrag(grip, true);
            Controls.Add(body);
            Controls.Add(footer);
            Controls.Add(tabs);
            Controls.Add(header);
            options = Options();
            ContextMenuStrip = title.ContextMenuStrip = body.ContextMenuStrip = options;
            body.SizeChanged += (sender, args) => SizeRows();
            hover = new Timer { Interval = 190 };
            hover.Tick += (sender, args) =>
            {
                if (!Box.Collapsed || ExpandMode != GridBox.Smart || moving || resizing)
                    return;
                bool inside = options.Visible || ScreenBounds.Contains(Cursor.Position);
                if (inside != expanded)
                {
                    expanded = inside;
                    AdjustFold();
                }
            };
            hover.Start();
            Content();
        }

        private static Button CaptionButton(string text, Action action)
        {
            var button = new CaptionControl { Text = text, Size = new Size(27, 29),
                Margin = new Padding(0, 1, 0, 0), ForeColor = GridLook.Text };
            button.Click += (sender, args) => action();
            return button;
        }

        private sealed class CaptionControl : Button
        {
            private bool hover;
            internal CaptionControl()
            {
                BackColor = Color.Transparent;
                FlatStyle = FlatStyle.Flat;
                FlatAppearance.BorderSize = 0;
                SetStyle(ControlStyles.UserPaint | ControlStyles.SupportsTransparentBackColor, true);
            }

            protected override void OnMouseEnter(EventArgs args) { hover = true; Invalidate(); base.OnMouseEnter(args); }
            protected override void OnMouseLeave(EventArgs args) { hover = false; Invalidate(); base.OnMouseLeave(args); }

            protected override void OnPaint(PaintEventArgs args)
            {
                var graphics = args.Graphics;
                graphics.SmoothingMode = SmoothingMode.AntiAlias;
                var center = new Point(Width / 2, Height / 2);
                if (hover || Focused)
                {
                    using (var fill = new SolidBrush(GridLook.Accent))
                    using (var shape = GridLook.Rounded(new Rectangle(2, 2, Width - 4, Height - 4), 6))
                        graphics.FillPath(fill, shape);
                }
                using (var stroke = new Pen(ForeColor, 1.5f) { StartCap = LineCap.Round, EndCap = LineCap.Round })
                {
                    if (Text == "⋯")
                    {
                        using (var fill = new SolidBrush(ForeColor))
                            for (int dot = -1; dot <= 1; dot++)
                                graphics.FillEllipse(fill, center.X + dot * 5 - 1, center.Y - 1, 2, 2);
                    }
                    else if (Text == "◢")
                    {
                        graphics.DrawLine(stroke, Width - 12, Height - 4, Width - 4, Height - 12);
                        graphics.DrawLine(stroke, Width - 8, Height - 4, Width - 4, Height - 8);
                    }
                    else
                    {
                        int direction = Text == "⌄" ? 1 : -1;
                        graphics.DrawLines(stroke, new[] { new Point(center.X - 4, center.Y - 2 * direction),
                            new Point(center.X, center.Y + 2 * direction), new Point(center.X + 4, center.Y - 2 * direction) });
                    }
                }
            }
        }

        protected override void OnPaintBackground(PaintEventArgs e)
        {
            if (ClientSize.Width < 2 || ClientSize.Height < 2)
                return;
            e.Graphics.SmoothingMode = SmoothingMode.AntiAlias;
            int radius = Box.Collapsed && !expanded ? headerHeight / 2 : 14;
            var outline = new Rectangle(0, 0, ClientSize.Width - 1, ClientSize.Height - 1);
            using (var shape = GridLook.Rounded(outline, radius))
            using (var fill = new LinearGradientBrush(outline, BackColor, GridLook.Canvas, 90f))
            using (var edge = new Pen(dropActive ? Color.FromArgb(128, 205, 194) : GridLook.Line, dropActive ? 2f : 1f))
            {
                e.Graphics.FillPath(fill, shape);
                e.Graphics.DrawPath(edge, shape);
            }
        }

        protected override bool ProcessCmdKey(ref Message message, Keys keys)
        {
            if (Box.Tabs.Count > 1 && (keys == (Keys.Control | Keys.Tab) || keys == (Keys.Control | Keys.Shift | Keys.Tab)))
            {
                CycleTab((keys & Keys.Shift) == 0 ? 1 : -1);
                return true;
            }
            return base.ProcessCmdKey(ref message, keys);
        }

        private void CycleTab(int step)
        {
            if (Box.Tabs.Count < 2)
                return;
            int index = Box.Tabs.FindIndex(item => item.Id == tab.Id);
            string target = Box.Tabs[(index + step + Box.Tabs.Count) % Box.Tabs.Count].Id;
            ChangeBox(copy => copy.Active = target);
        }

        internal void PrepareGrid(IntPtr parent, Screen screen, Rectangle rectangle)
        {
            natural = rectangle;
            headerHeight = Native.Scale(screen, 38);
            capsuleWidth = Native.Scale(screen, 198);
            header.Height = headerHeight;
            Prepare(parent, rectangle);
            if (parent == IntPtr.Zero)
            {
                Opacity = Box.Opacity / 100.0;
                Native.Backdrop(Handle);
            }
            else
                Native.SetLayerOpacity(Handle, Box.Opacity);
            AdjustFold();
        }

        private void AdjustFold()
        {
            bool open = ExpandMode == GridBox.Expanded || !Box.Collapsed || expanded;
            body.Visible = open;
            tabs.Visible = open && Box.Tabs.Count > 1;
            footer.Visible = open && !Box.LocksSize;
            collapse.AccessibleName = Box.Collapsed ? "展开格子" : "折叠格子";
            var next = natural;
            if (!open)
            {
                next.Height = headerHeight;
                next.Width = Math.Min(next.Width, capsuleWidth);
            }
            MoveTo(next);
            using (var shape = GridLook.Rounded(new Rectangle(Point.Empty, ClientSize), open ? 12 : headerHeight / 2))
            {
                var former = Region;
                Region = new Region(shape);
                if (former != null) former.Dispose();
            }
            Invalidate(true);
        }

        private void Flip()
        {
            if (ExpandMode != GridBox.Expanded)
                ChangeBox(copy => copy.Collapsed = !copy.Collapsed);
        }
        private void ChangeBox(Action<GridBox> edit)
        {
            app.Post(() => app.Edit(state => edit(state.Boxes.Single(copy => copy.Id == Box.Id))));
        }
        private void ChangeTab(Action<GridTab> edit)
        {
            app.Post(() => app.Edit(state => edit(state.Tabs().Single(copy => copy.Id == tab.Id))));
        }

        private void AddDrag(Control control, bool resize)
        {
            control.MouseDown += (sender, args) =>
            {
                if (args.Button != MouseButtons.Left || (resize ? Box.LocksSize : Box.LocksPosition))
                    return;
                pointer = Cursor.Position;
                start = ScreenBounds;
                resizing = resize;
                moving = true;
                control.Capture = true;
            };
            control.MouseMove += (sender, args) =>
            {
                if (!moving || !control.Capture)
                    return;
                var cursor = Cursor.Position;
                int dx = cursor.X - pointer.X, dy = cursor.Y - pointer.Y;
                var rect = start;
                if (resizing)
                {
                    rect.Width = Math.Max(160, rect.Width + dx);
                    rect.Height = Math.Max(108, rect.Height + dy);
                }
                else rect.Offset(dx, dy);
                MoveTo(rect);
            };
            control.MouseUp += (sender, args) =>
            {
                if (!moving)
                    return;
                moving = false;
                control.Capture = false;
                if (start != ScreenBounds)
                {
                    var moved = resizing ? ScreenBounds : new Rectangle(ScreenBounds.Location, natural.Size);
                    app.Post(() => app.Place(Box, moved));
                }
            };
        }

        private void AddDrop(Control control, string before = null)
        {
            if (tab.Folder != null) return;
            control.AllowDrop = true;
            DragEventHandler accept = (sender, args) =>
            {
                if (!args.Data.GetDataPresent(DataFormats.FileDrop)) return;
                args.Effect = (args.AllowedEffect & DragDropEffects.Link) != 0 ? DragDropEffects.Link :
                    (args.AllowedEffect & DragDropEffects.Copy) != 0 ? DragDropEffects.Copy : DragDropEffects.None;
                ShowDrop(args.Effect != DragDropEffects.None);
            };
            control.DragEnter += accept;
            control.DragOver += accept;
            control.DragLeave += (sender, args) => ShowDrop(false);
            control.DragDrop += (sender, args) =>
            {
                ShowDrop(false);
                var paths = args.Data.GetData(DataFormats.FileDrop) as string[];
                if (paths != null && args.Effect != DragDropEffects.None)
                    app.Post(() => app.Group(tab.Id, paths, before));
            };
        }

        private void ShowDrop(bool active)
        {
            if (dropActive == active)
                return;
            dropActive = active;
            Invalidate();
        }

        internal void RefreshMappedFolder()
        {
            if (folder != null)
                Content();
        }

        private void Content()
        {
            var entries = new List<DesktopItem>();
            if (folder != null)
            {
                if (app.TestPath(folder))
                {
                    try { entries = Directory.EnumerateFileSystemEntries(folder).Take(400).Select(DesktopItem.FromPath).ToList(); }
                    catch (IOException) { }
                    catch (UnauthorizedAccessException) { }
                }
                string signature = string.Join("\n", entries.Select(item => item.Id));
                if (folderSignature == signature) return;
                folderSignature = signature;
            }
            else
            {
                var desktop = app.DesktopItems().ToDictionary(item => item.Id, StringComparer.OrdinalIgnoreCase);
                foreach (string id in tab.Items.Where(app.TestPath))
                {
                    DesktopItem item;
                    if (desktop.TryGetValue(id, out item))
                        entries.Add(item);
                    else
                        entries.Add(DesktopItem.FromPath(id));
                }
            }
            entries = Sort(entries).ToList();
            body.SuspendLayout();
            foreach (Control existing in body.Controls.Cast<Control>().ToArray()) existing.Dispose();
            if (folder != null && !References.Equal(folder, tab.Folder))
                body.Controls.Add(GridLook.Button("‹ 上一级", () => app.Post(() => SetFolder(Path.GetDirectoryName(folder)))));
            if (entries.Count == 0)
                body.Controls.Add(new Label { Text = folder == null ? "将文件拖到这里" : "当前目录为空或不可访问",
                    ForeColor = GridLook.Muted, BackColor = Color.Transparent, AutoSize = true, Margin = new Padding(8, 12, 0, 0) });
            if (tab.StackByType)
                foreach (var group in entries.GroupBy(Kind, StringComparer.OrdinalIgnoreCase))
                {
                    string type = group.Key;
                    bool open = openStacks.Contains(type);
                    body.Controls.Add(GridLook.Button((open ? "⌄ " : "› ") + type + " · " + group.Count(), () => app.Post(() =>
                    {
                        if (!openStacks.Add(type)) openStacks.Remove(type);
                        folderSignature = null;
                        Content();
                    })));
                    if (open) foreach (var item in group) AddFile(item);
                }
            else
                foreach (var item in entries) AddFile(item);
            SizeRows();
            body.ResumeLayout();
        }

        private void AddFile(DesktopItem item)
        {
            Action openFolder = folder != null && Directory.Exists(item.Id) ? (Action)(() => app.Post(() => SetFolder(item.Id))) : null;
            var tile = new GridFile(app, item, tab.List, false, openFolder) { Size = tab.List ? new Size(180, 32) : new Size(72, 77),
                Margin = new Padding(2) };
            AddDrop(tile, tab.Sort == "manual" ? item.Id : null);
            body.Controls.Add(tile);
        }

        private void SetFolder(string next) { folder = next; folderSignature = null; Content(); }
        private void SizeRows()
        {
            if (tab.List)
                foreach (var tile in body.Controls.OfType<GridFile>())
                    tile.Width = Math.Max(120, body.ClientSize.Width - 39);
        }

        private IEnumerable<DesktopItem> Sort(List<DesktopItem> items)
        {
            IEnumerable<DesktopItem> sorted;
            switch (tab.Sort)
            {
                case "name": sorted = items.OrderBy(item => item.Name, StringComparer.CurrentCultureIgnoreCase); break;
                case "type": sorted = items.OrderBy(Kind, StringComparer.CurrentCultureIgnoreCase).ThenBy(item => item.Name); break;
                case "date": sorted = items.OrderBy(item => File.GetLastWriteTimeUtc(item.Id)); break;
                case "size": sorted = items.OrderBy(item => SizeOf(item.Id)); break;
                default: return items;
            }
            return tab.Descending ? sorted.Reverse() : sorted;
        }

        private static long SizeOf(string path)
        {
            try { return new FileInfo(path).Length; }
            catch (IOException) { return 0; }
            catch (UnauthorizedAccessException) { return 0; }
        }

        private static string Kind(DesktopItem item)
        {
            if (Directory.Exists(item.Id)) return "文件夹";
            string extension = Path.GetExtension(item.Id);
            return string.IsNullOrEmpty(extension) ? "其他" : extension.Substring(1).ToUpperInvariant();
        }

        private ContextMenuStrip Options()
        {
            var menu = GridLook.Menu();
            menu.Items.Add(app.MenuItem("重命名", () =>
            {
                string name = GridLook.Prompt(this, "格子名称", "输入名称", tab.Name, 64);
                if (!string.IsNullOrWhiteSpace(name)) ChangeTab(copy => copy.Name = name);
            }));
            if (tab.Folder == null)
            {
                menu.Items.Add(app.MenuItem("添加桌面项目", () => GridManager.PickItems(app, this, tab.Id)));
                menu.Items.Add(app.MenuItem("自动归类规则", () =>
                {
                    GridManager.EditRules(app, this, tab.Id);
                }));
            }
            else
                menu.Items.Add(app.MenuItem("在资源管理器打开映射目录", () => app.Open(tab.Folder)));
            menu.Items.Add(new ToolStripSeparator());
            menu.Items.Add(app.MenuItem(tab.List ? "图标视图" : "列表视图", () => ChangeTab(copy => copy.List = !copy.List)));
            menu.Items.Add(app.MenuItem(tab.StackByType ? "取消按类型堆叠" : "按类型堆叠",
                () => ChangeTab(copy => copy.StackByType = !copy.StackByType)));
            var sort = new ToolStripMenuItem("排序");
            foreach (var pair in new[] { "manual|默认", "name|名称", "type|类型", "date|修改时间", "size|大小" })
            {
                var parts = pair.Split('|');
                var option = app.MenuItem(parts[1], () => ChangeTab(copy => copy.ChangeSort(parts[0])));
                option.Checked = tab.Sort == parts[0];
                sort.DropDownItems.Add(option);
            }
            menu.Items.Add(sort);
            var group = new ToolStripMenuItem("格子分组");
            foreach (var target in app.Store.State.Boxes.Where(box => box.Id != Box.Id))
            {
                string id = target.Id;
                group.DropDownItems.Add(app.MenuItem("移入「" + target.Current().Name + "」",
                    () => app.Edit(state => state.MoveTab(Box.Id, tab.Id, id), true)));
            }
            if (Box.Tabs.Count > 1)
                group.DropDownItems.Add(app.MenuItem("拆成独立格子", () => app.Edit(state => state.MoveTab(Box.Id, tab.Id, null), true)));
            group.Enabled = group.HasDropDownItems;
            menu.Items.Add(group);
            menu.Items.Add(new ToolStripSeparator());
            menu.Items.Add(app.MenuItem(Box.LocksPosition ? "解锁位置" : "锁定位置",
                () => ChangeBox(copy => copy.PositionLocked = !copy.LocksPosition)));
            menu.Items.Add(app.MenuItem(Box.LocksSize ? "解锁大小" : "锁定大小",
                () => ChangeBox(copy => copy.SizeLocked = !copy.LocksSize)));
            var behavior = new ToolStripMenuItem("展开方式");
            for (int i = 0; i < GridBox.CollapseModes.Length; i++)
            {
                string mode = GridBox.CollapseModes[i];
                var item = app.MenuItem(GridBox.CollapseLabels[i], () => ChangeBox(copy => copy.SetCollapseMode(mode)));
                item.Checked = Box.DisplayMode == mode;
                behavior.DropDownItems.Add(item);
            }
            menu.Items.Add(behavior);
            menu.Items.Add(app.MenuItem("底色与透明度", () => GridManager.EditAppearance(app, this, Box.Id)));
            menu.Items.Add(app.MenuItem("隐藏格子", () => ChangeBox(copy => copy.Hidden = true)));
            menu.Items.Add(app.MenuItem("删除当前格子", () => GridManager.RemoveGrid(app, this, Box.Id, tab.Id)));
            menu.Items.Add(new ToolStripSeparator());
            menu.Items.Add(app.MenuItem("新建格子", () => GridManager.CreateGrid(app, this, false)));
            menu.Items.Add(app.MenuItem("全局搜索", app.ShowSearch));
            menu.Items.Add(app.MenuItem("管理桌面格子", app.ShowManager));
            if (app.IsPreview)
                menu.Items.Add(app.MenuItem("退出格子体验", () => app.Stop(true)));
            return menu;
        }

        protected override void Dispose(bool disposing)
        {
            if (disposing && !IsDisposed)
            {
                if (hover != null) hover.Dispose();
                ContextMenuStrip = null;
                if (title != null) title.ContextMenuStrip = null;
                if (body != null) body.ContextMenuStrip = null;
                if (options != null) options.Dispose();
                if (surfaceFont != null) surfaceFont.Dispose();
            }
            base.Dispose(disposing);
        }
    }
}
