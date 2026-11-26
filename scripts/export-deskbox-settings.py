"""从固定 DeskBox 源码提取设置分区、原文和控件类型，不处理事件或绑定。"""

import json
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path


deskbox = Path(sys.argv[1])
xaml = deskbox / "Views/SettingsWindow.xaml"
x = "{http://schemas.microsoft.com/winfx/2006/xaml}"
localized = "{using:DeskBox.Services}Localized."
strings = json.loads((deskbox / "Strings/zh-CN.json").read_text(encoding="utf-8-sig"))
root = ET.parse(xaml).getroot()
types = {
    "ToggleSwitch": "Toggle", "CheckBox": "Toggle", "ComboBox": "Choice",
    "Slider": "Slider", "NumberBox": "Number", "TextBox": "Text",
    "PasswordBox": "Text", "Button": "Button", "HyperlinkButton": "Button",
}


def name(element):
    return element.tag.split("}")[-1]


def literal(element, property_name):
    value = element.get(property_name, "").strip()
    return value if not value.startswith("{") else ""


def text(element, property_name):
    if key := element.get(localized + property_name + "Key"):
        return strings.get(key, "")
    return literal(element, property_name)


def caption(element):
    key = element.get(localized + "Key")
    return strings.get(key, "") if key else literal(element, "Text")


def controls(element):
    for child in element:
        kind = name(child)
        if kind in {"SettingsCard", "SettingsExpander", "DataTemplate", "ResourceDictionary"}:
            continue
        if kind.endswith(".HeaderIcon") or kind.endswith(".Description") or kind.endswith(".Header"):
            continue
        if kind in types:
            yield child
        else:
            yield from controls(child)


def discover(parent):
    for element in parent:
        kind = name(element)
        if kind in {"DataTemplate", "ResourceDictionary"} or kind.endswith(".Resources"):
            continue
        key = element.get(localized + "HeaderKey")
        tag = element.get("Tag", "")
        if kind in {"SettingsCard", "SettingsExpander"} or (
            kind == "Grid" and (tag == "SettingRow" or "SettingRowStyle" in element.get("Style", ""))
        ):
            options = list(controls(element))
            title = text(element, "Header")
            if not title:
                heading = next((block for block in element.iter() if name(block) == "TextBlock"
                                and ("SettingTitleTextStyle" in block.get("Style", "") or
                                     "SettingsSubsectionTitleTextStyle" in block.get("Style", ""))), None)
                if heading is not None:
                    title = caption(heading)
                    key = heading.get(localized + "Key") or key
            if not title and options:
                key = options[0].get(localized + "AutomationNameKey") or key
                title = strings.get(key, "")
            icon = next((node.get("Glyph") for node in element.iter() if name(node) == "FontIcon"), None)
            if title:
                row = {"Key": key or title, "Title": title}
                if description := text(element, "Description"):
                    row["Description"] = description
                if icon:
                    row["Glyph"] = icon
                if tag in routes:
                    row["Route"] = tag
                if options:
                    row["Editors"] = [editor(option) for option in options]
                if kind == "SettingsExpander":
                    children = list(discover(element))
                    if children:
                        row["Children"] = children
                yield row
            else:
                yield from discover(element)
        else:
            yield from discover(element)


def editor(element):
    item = {"Kind": types[name(element)]}
    if item["Kind"] == "Button":
        label = (text(element, "Content") or text(element, "AutomationName") or
                 next((value for block in element.iter() if (value := caption(block))), ""))
        if label:
            item["Text"] = label
    if item["Kind"] == "Choice":
        options = [text(node, "Content") or next((value for block in node.iter() if (value := caption(block))), "")
                   for node in element if name(node) == "ComboBoxItem"]
        if any(options):
            item["Options"] = [option for option in options if option]
    return item


routes = re.findall(r'\["(\w+)"\] = new\("\w+", "([^"]+)", (null|"[^"]+"), "([^"]+)"\)',
                    xaml.with_suffix(".xaml.cs").read_text(encoding="utf-8-sig"))
route_info = {tag: (title, parent.strip('"') if parent != "null" else None, nav)
              for tag, title, parent, nav in routes if tag != "Advanced"}
routes = set(route_info)
templates = {node.get(x + "Key"): node for node in root.iter() if name(node) == "DataTemplate"}
general = next(node for node in root.iter() if node.get(x + "Name") == "GeneralSection")


def body(tag):
    node = general if tag == "General" else templates.get(tag + "SectionTemplate")
    if tag == "Interaction":
        node = templates["InteractionWindowSettingsSectionTemplate"]
    if node is None:
        raise ValueError(tag)
    section = next((part for part in node.iter() if part.tag.startswith("{using:DeskBox.Views.SettingsSections}")), None)
    if section is not None:
        node = ET.parse(deskbox / "Views" / "SettingsSections" / (name(section) + ".xaml")).getroot()
    return node


catalog = {}
for tag, (title, parent, nav) in route_info.items():
    rows = list(discover(body(tag)))
    catalog[tag] = {"Title": strings[title], "Parent": parent, "Nav": nav, "Rows": rows}

# 部分入口由原版代码填入窗口或动态列表，保留其边界和原有名称。
for parent, child in ((details["Parent"], tag) for tag, details in catalog.items() if details["Parent"]):
    if not any(row.get("Route") == child for row in catalog[parent]["Rows"]):
        catalog[parent]["Rows"].append({"Key": child, "Title": catalog[child]["Title"], "Route": child})

for tag, key in (("DesktopOrganizationSettings", "DesktopOrganization.OneClick.Action"),
                 ("FileStackSettings", "Settings.FileStacks.Custom.Rules.Title"),
                 ("BackupRestoreSettings", "Settings.DataBackup.Export")):
    if not any(row["Key"] == key for row in catalog[tag]["Rows"]):
        entry = {"Key": key, "Title": strings[key], "Editors": [{"Kind": "Button"}]}
        if tag == "DesktopOrganizationSettings":
            catalog[tag]["Rows"].insert(0, entry)
        else:
            catalog[tag]["Rows"].append(entry)

navigation = []
for node in root.iter():
    if name(node) != "NavigationViewItem" or node.get("Tag") not in catalog:
        continue
    icon = next((part.get("UriSource") for part in node.iter() if name(part) == "SvgImageSource"), None)
    navigation.append({"Tag": node.get("Tag"), "Title": caption(node), "Icon": icon})
assert len(navigation) == 8 and len(catalog) == 30
Path(sys.argv[2]).write_text(json.dumps({"Navigation": navigation, "Pages": catalog},
                                      ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
