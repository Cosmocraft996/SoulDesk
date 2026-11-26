using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.RegularExpressions;

namespace SoulDesk.DesktopGrid
{
    public sealed class GridPreference
    {
        public string Row, Key, Kind;
        public string[] Values, Labels;
        public double Min, Max, Step;

        internal bool Accepts(string value)
        {
            if (string.IsNullOrEmpty(value) || value.Length > 64) return false;
            if (Values != null) return Values.Contains(value, StringComparer.Ordinal);
            if (Kind == "toggle") return value == "true" || value == "false";
            if (Kind == "color") return Regex.IsMatch(value, "^#[0-9a-fA-F]{6}$");
            if (Kind == "image")
                return Regex.IsMatch(value, (Key == "WidgetBackgroundPanoramaImage" ? "^panorama" : "^background") +
                    @"\.(png|jpg|jpeg|bmp|ico|svg)$");
            double number;
            return double.TryParse(value, NumberStyles.AllowDecimalPoint, CultureInfo.InvariantCulture, out number) &&
                number >= Min && number <= Max && (Kind != "integer" || number == Math.Truncate(number));
        }
    }

    internal static class GridPreferences
    {
        private static GridPreference Toggle(string row, string key)
        {
            return new GridPreference { Row = row, Key = key, Kind = "toggle" };
        }

        private static GridPreference Range(string row, string key, double min, double max, double step, bool integer = false)
        {
            return new GridPreference { Row = row, Key = key, Kind = integer ? "integer" : "number",
                Min = min, Max = max, Step = step };
        }

        private static GridPreference Choice(string row, string key, string values, string labels)
        {
            return new GridPreference { Row = row, Key = key, Kind = "choice", Values = values.Split('|'), Labels = labels.Split('|') };
        }

        internal static readonly GridPreference[] Rows =
        {
            Choice("Settings.Theme.Title", "Theme", "System|Light|Dark", "跟随系统|浅色|深色"),
            Choice("Settings.Accent.Source.Title", "AccentColorMode", "System|Custom", "跟随系统|自定义"),
            new GridPreference { Row = "Settings.Accent.Title", Key = "CustomAccentColor", Kind = "color" },
            Toggle("Settings.ShowFileExtensions.Title", "ShowFileExtensions"),
            Toggle("Settings.HideShortcutExtension.Title", "HideShortcutExtensionWhenShowingFileExtensions"),
            Toggle("Settings.ShowImageFilesAsIcons.Title", "ShowImageFilesAsIcons"),
            Toggle("Settings.HideShortcutArrow.Title", "HideShortcutArrowOverlay"),
            Toggle("Settings.ListDetails.Title", "ShowListItemDetails"),
            Toggle("Settings.FilePathTooltips.Title", "ShowFileItemPathTooltips"),
            Range("Settings.FileNameWidth.Title", "FileNameWidthScale", 0, 1, .05),
            Choice("Settings.FileNameLines.Title", "FileNameLineCount", "0|1|2", "隐藏|单行|双行"),
            Choice("Settings.FileWidget.FolderOpenBehavior.Title", "FileWidgetFolderOpenBehavior", "Embedded|Explorer", "在格子内打开|在资源管理器中打开"),
            Choice("Settings.FileStacks.OpenMode.Title", "FileStackOpenMode", "Inline|Popover", "格子内展开|弹窗展开"),
            Choice("Settings.FileStacks.PopoverLayout.Title", "FileStackPopoverLayout", "Adaptive|Grid3|Grid5", "自适应|3 × 3|5 × 5"),
            Choice("Settings.FileStacks.PopoverStyle.Title", "FileStackPopoverStyle", "Neutral|FollowMaterial", "中性|跟随格子材质"),
            Toggle("Settings.FileStacks.Auto.Title", "FileStackAutoStacking"),
            Choice("Settings.FileStacks.Threshold.Title", "FileStackThreshold", "2|3|5", "2 个|3 个|5 个"),
            Choice("Settings.FileStacks.GroupBy.Title", "FileStackGroupBy", "Kind|DateAdded|DateModified|Custom", "文件类型|添加日期|修改日期|自定义格式"),
            Choice("Settings.FileStacks.Unmatched.Title", "FileStackUnmatchedBehavior", "KeepLoose|Other", "保持散放|归入其他"),
            Choice("Settings.FileStacks.OrderBy.Title", "FileStackOrderBy", "Widget|Name|DateAdded|DateModified", "跟随格子|名称|添加日期|修改日期"),
            Choice("Settings.Capsule.WidthMode.Title", "WidgetCompactWidthMode", "Aligned|Independent", "沿用胶囊宽度|独立宽度"),
            Choice("Settings.Capsule.ExpansionDirection.Title", "WidgetCompactExpansionDirection", "Auto|Down|Up", "自动|向下|向上"),
            Choice("Settings.CompactContent.Title", "WidgetCompactContentMode", "Smart|Minimal|Summary", "智能|精简|摘要"),
            Choice("Settings.Capsule.ArrangementDetails.Title", "WidgetCapsuleBarPlacement", "Floating|Top|Bottom|Left|Right", "自由摆放|上边缘|下边缘|左边缘|右边缘"),
            Choice("Settings.Capsule.HoverResponse.Title", "CompactHoverResponse", "Sensitive|Balanced|PreventAccidental|Custom", "灵敏|均衡|防误触|自定义"),
            Range("Settings.Capsule.ExpandDelay.Title", "WidgetCompactExpandDelayMs", 100, 1000, 20, true),
            Range("Settings.Capsule.CollapseDelay.Title", "WidgetCompactCollapseDelayMs", 200, 1500, 20, true),
            Choice("Settings.Capsule.Animation.Title", "WidgetCompactAnimationEffect", "Smooth|Slow|Snappy|Custom|None", "平滑|舒缓|快速|自定义|无动画"),
            Range("Settings.Capsule.AnimationDuration.Title", "WidgetCompactAnimationDurationMs", 120, 400, 10, true),
            Toggle("Settings.CompactPrivacy.Title", "WidgetCompactHideSensitiveContent"),
            Choice("Settings.WidgetGroupNavigation.Title", "WidgetGroupDefaultNavigationStyle", "Tabs|Stack", "并排标签|堆叠标签"),
            Choice("Settings.WidgetGroupTitle.Title", "WidgetGroupDefaultTitleDisplayMode", "IconAndText|IconOnly|TextOnly", "图标和标题|仅图标|仅标题"),
            Choice("Settings.WidgetGroupSwitchAnimation.Title", "WidgetGroupSwitchAnimationStyle", "Auto|Vertical|Horizontal|Fade|None", "自动|纵向滑动|横向滑动|淡入淡出|无动画"),
            Toggle("Settings.WidgetGroupWheel.Title", "WidgetGroupWheelSwitchEnabled"),
            Toggle("Settings.WidgetGroupHover.Title", "WidgetGroupHoverSwitchEnabled"),
            Toggle("Settings.ResizeSnap.Title", "ResizeSnapEnabled"),
            Range("Settings.WidgetSnap.Spacing.Title", "WidgetSnapSpacing", 0, 32, 1),
            Choice("Settings.OpenMethod.Title", "DoubleClickToOpen", "true|false", "双击打开|单击打开"),
            Toggle("Settings.Interaction.Hover.Title", "ShowHoverButtons"),
            Choice("Settings.Material.Title", "WidgetMaterialType", "Mica|MicaAlt|Acrylic|AcrylicBase|Solid", "云母|深层云母|亚克力|基础亚克力|纯色"),
            Range("Settings.Opacity.Title", "WidgetOpacity", 0, 1, .05),
            Range("Settings.MaterialIntensity.Title", "WidgetMaterialIntensity", 0, 1, .05),
            Choice("Settings.WidgetForeground.Title", "WidgetForegroundMode", "FollowTheme|Light|Dark|Custom", "跟随主题|浅色|深色|自定义"),
            new GridPreference { Row = "Settings.WidgetForeground.CustomColor.Title", Key = "WidgetForegroundColor", Kind = "color" },
            Toggle("Settings.TextShadow.Title", "WidgetTextShadowEnabled"),
            Choice("Settings.WidgetBackground.Mode.Title", "WidgetBackgroundMode", "Material|UnifiedImage|Panorama", "跟随材质|统一背景|全景背景"),
            Choice("Settings.WidgetBackground.Fit.Title", "WidgetBackgroundUnifiedFit", "Fill|Contain", "填满|完整显示"),
            new GridPreference { Key = "WidgetBackgroundUnifiedImage", Kind = "image" },
            new GridPreference { Key = "WidgetBackgroundPanoramaImage", Kind = "image" },
            Range("Settings.WidgetBackground.Dim.Title", "WidgetBackgroundDim", 0, 100, 5),
            Choice("Settings.BorderColor.Title", "WidgetBorderColorMode", "Neutral|Accent|None", "中性|主题色|无边框"),
            Choice("Settings.Border.Title", "WidgetBorderStyle", "Thin|Medium|Thick", "细|中|粗"),
            Choice("Settings.Corner.Title", "WidgetCornerPreference", "Square|Small|Round", "直角|小圆角|圆角"),
            Choice("Settings.Density.Title", "LayoutDensity", "Compact|Standard|Relaxed|Custom", "紧凑|标准|宽松|自定义"),
            Range("Settings.Density.Scale.Title", "LayoutDensityScale", 0, 1, .05),
            Range("Settings.TextSize.Title", "TextSize", 10, 16, .5),
            Range("Settings.IconSize.Title", "IconSize", 24, 56, 1),
            Range("Settings.HorizontalSpacing.Title", "HorizontalSpacingScale", 0, 1, .05),
            Range("Settings.VerticalSpacing.Title", "VerticalSpacingScale", 0, 1, .05),
            Choice("Settings.WidgetTitleIcon.Title", "WidgetTitleIconMode", "FilledMono|LineMono|Color|Hidden|TextLabel", "填充单色|线框单色|彩色|隐藏|文字标签"),
            Choice("Settings.InteractiveWidgetChrome.Title", "InteractiveWidgetChromeMode", "Standard|Compact|Overlay|Hidden", "标准|紧凑|覆盖|隐藏"),
            Range("Settings.DefaultWidth.Title", "DefaultWidgetWidth", 144, 1200, 5, true),
            Choice("DesktopOrganization.Auto.Interval.Title", "AutoSortDelaySeconds", "4|60|3600", "文件稳定后|1 分钟|1 小时"),
        };

        private static readonly Dictionary<string, GridPreference> ByKey = Rows.ToDictionary(item => item.Key, StringComparer.Ordinal);
        internal static readonly string[] DensityValues = { "IconSize", "TextSize", "LayoutDensityScale",
            "HorizontalSpacingScale", "VerticalSpacingScale", "FileNameWidthScale" };

        internal static bool Known(string key) { return key != null && ByKey.ContainsKey(key); }

        internal static bool Accepts(string key, string value)
        {
            GridPreference option;
            return key != null && ByKey.TryGetValue(key, out option) && option.Accepts(value);
        }

        internal static int Integer(GridState state, string key, int fallback)
        {
            string value;
            double number;
            return state.Preferences.TryGetValue(key, out value) &&
                double.TryParse(value, NumberStyles.AllowDecimalPoint, CultureInfo.InvariantCulture, out number)
                ? (int)number : fallback;
        }

        internal static void Set(GridState state, string key, string value)
        {
            if (value == null)
            {
                state.Preferences.Remove(key);
                return;
            }
            if (!Accepts(key, value))
                throw new InvalidOperationException("桌面格子选项的设置值无效。");
            state.Preferences[key] = value;
            if (key == "LayoutDensity" && value != "Custom")
                foreach (var property in DensityValues) state.Preferences.Remove(property);
            if (key == "CompactHoverResponse" && value != "Custom")
            {
                state.Preferences.Remove("WidgetCompactExpandDelayMs");
                state.Preferences.Remove("WidgetCompactCollapseDelayMs");
            }
        }
    }
}
