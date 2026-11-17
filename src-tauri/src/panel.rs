//! 面板偏好及逻辑坐标；旧偏好缺少新增字段时使用兼容默认值。
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Deserialize)]
struct ColorPreset {
    color: String,
}
#[derive(Deserialize)]
struct ColorPalette {
    presets: Vec<ColorPreset>,
}
fn color_palette() -> &'static ColorPalette {
    static PALETTE: OnceLock<ColorPalette> = OnceLock::new();
    PALETTE.get_or_init(|| {
        serde_json::from_str(include_str!("../../src/panel-colors.json"))
            .expect("分组预设色板格式无效")
    })
}
fn normalize_color(color: &str) -> Result<String, String> {
    if color.len() != 7
        || !color.starts_with('#')
        || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
    {
        return Err("颜色须为六位十六进制色值，例如 #0A84FF".into());
    }
    Ok(color.to_ascii_uppercase())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub auto_color: String,
    #[serde(default)]
    pub color: Option<String>,
}

/// 未分组使用空归属；不复制实例，也不改变组件业务配置。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Groups {
    pub groups: Vec<Group>,
    pub active: Option<String>,
    pub membership: BTreeMap<String, String>,
    #[serde(default)]
    pub ungrouped_color: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum GroupChange {
    Create {
        name: String,
    },
    Rename {
        id: String,
        name: String,
    },
    Color {
        group: Option<String>,
        color: Option<String>,
    },
    Remove {
        id: String,
    },
    Reorder {
        ids: Vec<String>,
    },
    Select {
        group: Option<String>,
    },
    Move {
        instance: String,
        group: Option<String>,
    },
}

impl Groups {
    fn next_auto_color(&self) -> String {
        color_palette()
            .presets
            .iter()
            .min_by_key(|preset| {
                self.groups
                    .iter()
                    .filter(|g| g.auto_color == preset.color)
                    .count()
            })
            .expect("分组预设色板不能为空")
            .color
            .clone()
    }

    /// 旧记录只在内存中补色；首次成功修改时随原偏好一起保存，排序不重新配色。
    pub fn prepare_colors(&mut self) -> Result<(), String> {
        self.ungrouped_color = self
            .ungrouped_color
            .as_deref()
            .map(normalize_color)
            .transpose()?;
        for group in &mut self.groups {
            group.color = group.color.as_deref().map(normalize_color).transpose()?;
            if !group.auto_color.is_empty() {
                group.auto_color = normalize_color(&group.auto_color)?;
            }
        }
        for index in 0..self.groups.len() {
            if self.groups[index].auto_color.is_empty() {
                self.groups[index].auto_color = self.next_auto_color();
            }
        }
        Ok(())
    }
    pub fn contains(&self, instance: &str) -> bool {
        self.membership.get(instance) == self.active.as_ref()
    }

    fn require_group(&self, id: Option<&str>) -> Result<(), String> {
        if id.is_some_and(|id| !self.groups.iter().any(|group| group.id == id)) {
            return Err("CONFLICT: 分组已不存在，请刷新后重试".into());
        }
        Ok(())
    }

    fn name(&self, name: &str, except: Option<&str>) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
            return Err("分组名称须为1～40个字符，不能包含换行或控制字符".into());
        }
        if name == "未分组"
            || self
                .groups
                .iter()
                .any(|g| Some(g.id.as_str()) != except && g.name == name)
        {
            return Err("分组名称已存在，请换一个名称".into());
        }
        Ok(name.to_owned())
    }

    pub fn change(&mut self, change: GroupChange) -> Result<(), String> {
        match change {
            GroupChange::Create { name } => {
                let name = self.name(&name, None)?;
                let id = uuid::Uuid::new_v4().to_string();
                let auto_color = self.next_auto_color();
                self.groups.push(Group {
                    id: id.clone(),
                    name,
                    auto_color,
                    color: None,
                });
                self.active = Some(id);
            }
            GroupChange::Rename { id, name } => {
                self.require_group(Some(&id))?;
                let name = self.name(&name, Some(&id))?;
                self.groups.iter_mut().find(|g| g.id == id).unwrap().name = name;
            }
            GroupChange::Color { group, color } => {
                self.require_group(group.as_deref())?;
                let color = color.as_deref().map(normalize_color).transpose()?;
                if let Some(id) = group {
                    self.groups.iter_mut().find(|g| g.id == id).unwrap().color = color;
                } else {
                    self.ungrouped_color = color;
                }
            }
            GroupChange::Remove { id } => {
                self.require_group(Some(&id))?;
                self.groups.retain(|g| g.id != id);
                self.membership.retain(|_, group| *group != id);
                if self.active.as_ref() == Some(&id) {
                    self.active = None;
                }
            }
            GroupChange::Reorder { ids } => {
                let mut groups = self.groups.clone();
                let mut ordered = Vec::new();
                for id in ids {
                    let index = groups
                        .iter()
                        .position(|g| g.id == id)
                        .ok_or("CONFLICT: 分组列表已变化，请刷新后重试")?;
                    ordered.push(groups.remove(index));
                }
                if !groups.is_empty() {
                    return Err("CONFLICT: 分组列表已变化，请刷新后重试".into());
                }
                self.groups = ordered;
            }
            GroupChange::Select { group } => {
                self.require_group(group.as_deref())?;
                self.active = group;
            }
            GroupChange::Move { instance, group } => {
                self.require_group(group.as_deref())?;
                if let Some(group) = group {
                    self.membership.insert(instance, group);
                } else {
                    self.membership.remove(&instance);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Preferences {
    pub ratio: f64,
    pub tint: f64,
    pub slide_in_ms: u32,
    pub slide_out_ms: u32,
    pub desktop_double_click: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            ratio: 0.4,
            tint: 0.12,
            slide_in_ms: 250,
            slide_out_ms: 250,
            desktop_double_click: true,
        }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<(), String> {
        if !self.ratio.is_finite()
            || !(0.0..=1.0).contains(&self.ratio)
            || self.ratio == 0.0
            || !self.tint.is_finite()
            || !(0.0..=1.0).contains(&self.tint)
        {
            return Err("宽度比例须大于0且不超过100%，遮罩须为0～100%".into());
        }
        if self.slide_in_ms > 1000 || self.slide_out_ms > 1000 {
            return Err("滑入、滑出时长须为0～1000毫秒；0表示无动效".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
impl Layout {
    pub fn validate(&self, min_width: f64, min_height: f64) -> Result<(), String> {
        if ![self.x, self.y, self.width, self.height]
            .iter()
            .all(|n| n.is_finite())
            || !(0.0..=100_000.0).contains(&self.x)
            || !(0.0..=100_000.0).contains(&self.y)
            || !(min_width..=16_384.0).contains(&self.width)
            || !(min_height..=16_384.0).contains(&self.height)
        {
            return Err("面板布局超出范围或小于组件最小尺寸".into());
        }
        Ok(())
    }
}

pub fn can_auto_collapse(
    focused: bool,
    editing: bool,
    interacting: bool,
    menu: bool,
    dialog: bool,
    detail: bool,
    inside: bool,
) -> bool {
    !editing && !interacting && !menu && !dialog && !detail && (!focused || !inside)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_groups_get_balanced_automatic_colors_that_survive_sort_rename_and_custom_reset() {
        let mut groups: Groups = serde_json::from_str(r##"{"groups":[{"id":"work","name":"工作"},{"id":"life","name":"生活"}],"active":"work","membership":{}}"##).unwrap();
        groups.prepare_colors().unwrap();
        let colors = groups
            .groups
            .iter()
            .map(|g| (g.id.clone(), g.auto_color.clone()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(colors["work"], color_palette().presets[0].color);
        assert_eq!(colors["life"], color_palette().presets[1].color);
        groups
            .change(GroupChange::Color {
                group: Some("work".into()),
                color: Some("#aBc123".into()),
            })
            .unwrap();
        groups
            .change(GroupChange::Reorder {
                ids: vec!["life".into(), "work".into()],
            })
            .unwrap();
        groups
            .change(GroupChange::Rename {
                id: "work".into(),
                name: "办公".into(),
            })
            .unwrap();
        assert_eq!(groups.groups[1].color.as_deref(), Some("#ABC123"));
        groups
            .change(GroupChange::Color {
                group: Some("work".into()),
                color: None,
            })
            .unwrap();
        assert_eq!(groups.groups[1].color, None);
        for group in &groups.groups {
            assert_eq!(group.auto_color, colors[&group.id]);
        }
        groups
            .change(GroupChange::Color {
                group: None,
                color: Some("#FFFFFF".into()),
            })
            .unwrap();
        assert_eq!(groups.ungrouped_color.as_deref(), Some("#FFFFFF"));
        groups
            .change(GroupChange::Color {
                group: None,
                color: None,
            })
            .unwrap();
        assert_eq!(groups.ungrouped_color, None);
        for index in 2..=color_palette().presets.len() {
            groups
                .change(GroupChange::Create {
                    name: format!("分组{index}"),
                })
                .unwrap();
        }
        let counts = color_palette()
            .presets
            .iter()
            .map(|p| {
                groups
                    .groups
                    .iter()
                    .filter(|g| g.auto_color == p.color)
                    .count()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            counts.iter().max().unwrap() - counts.iter().min().unwrap(),
            1
        );
    }

    #[test]
    fn malformed_group_colors_and_missing_group_are_rejected_without_changes() {
        let mut groups = Groups::default();
        groups
            .change(GroupChange::Create {
                name: "工作".into(),
            })
            .unwrap();
        let before = groups.clone();
        for color in [
            "red",
            "#FFF",
            "#12345678",
            "#GGGGGG",
            "#１２",
            "#123456 ",
            "url(x)",
        ] {
            assert!(groups
                .change(GroupChange::Color {
                    group: groups.active.clone(),
                    color: Some(color.into())
                })
                .is_err());
            assert_eq!(groups, before);
        }
        assert!(groups
            .change(GroupChange::Color {
                group: Some("missing".into()),
                color: None
            })
            .is_err());
        assert_eq!(groups, before);
    }
    #[test]
    fn groups_validate_names_and_reject_outdated_sort_without_partial_changes() {
        let mut groups = Groups::default();
        assert!(groups.contains("old-instance"));
        for name in ["", "  ", "未分组", "工作\n生活"] {
            assert!(groups
                .change(GroupChange::Create { name: name.into() })
                .is_err());
        }
        groups
            .change(GroupChange::Create {
                name: " 工作 ".into(),
            })
            .unwrap();
        let work = groups.active.clone().unwrap();
        assert_eq!(groups.groups[0].name, "工作");
        assert!(!groups.contains("old-instance"));
        assert!(groups
            .change(GroupChange::Create {
                name: "工作".into()
            })
            .is_err());
        groups
            .change(GroupChange::Create {
                name: "生活".into(),
            })
            .unwrap();
        let life = groups.active.clone().unwrap();
        let original = groups.clone();
        for ids in [
            vec![work.clone()],
            vec![work.clone(), work.clone()],
            vec![work.clone(), "missing".into()],
        ] {
            assert!(groups.change(GroupChange::Reorder { ids }).is_err());
            assert_eq!(groups, original);
        }
        groups
            .change(GroupChange::Reorder {
                ids: vec![life.clone(), work.clone()],
            })
            .unwrap();
        assert_eq!(groups.groups[0].id, life);
        groups
            .change(GroupChange::Rename {
                id: work.clone(),
                name: "研究".into(),
            })
            .unwrap();
        assert_eq!(groups.groups[1].name, "研究");
        assert!(groups
            .change(GroupChange::Rename {
                id: work,
                name: "生活".into()
            })
            .is_err());
        assert!(groups
            .change(GroupChange::Select {
                group: Some("missing".into())
            })
            .is_err());
    }

    #[test]
    fn deleting_groups_returns_members_to_ungrouped_and_preserves_other_groups() {
        let mut groups = Groups::default();
        groups
            .change(GroupChange::Create {
                name: "工作".into(),
            })
            .unwrap();
        let work = groups.active.clone().unwrap();
        groups
            .change(GroupChange::Move {
                instance: "clock".into(),
                group: Some(work.clone()),
            })
            .unwrap();
        groups
            .change(GroupChange::Create {
                name: "生活".into(),
            })
            .unwrap();
        let life = groups.active.clone();
        groups
            .change(GroupChange::Move {
                instance: "todo".into(),
                group: life.clone(),
            })
            .unwrap();
        groups.change(GroupChange::Remove { id: work }).unwrap();
        assert_eq!(groups.active, life);
        assert!(!groups.membership.contains_key("clock"));
        assert!(groups.contains("todo"));
        groups
            .change(GroupChange::Remove { id: life.unwrap() })
            .unwrap();
        assert_eq!(groups.active, None);
        assert!(groups.membership.is_empty());
        assert!(groups.contains("clock") && groups.contains("todo"));
    }
    #[test]
    fn old_preferences_get_defaults_and_animation_accepts_zero_through_one_second() {
        let mut p: Preferences = serde_json::from_str(r#"{"ratio":0.5,"tint":0.2}"#).unwrap();
        assert!(p.desktop_double_click);
        assert_eq!(p.slide_in_ms, 250);
        p.slide_in_ms = 0;
        p.slide_out_ms = 1000;
        assert!(p.validate().is_ok());
        p.slide_out_ms = 1001;
        assert!(p.validate().is_err());
        p.slide_out_ms = 250;
        p.ratio = f64::NAN;
        assert!(p.validate().is_err());
    }
    #[test]
    fn panel_auto_collapse_on_mouse_leave_or_focus_loss_keeps_interaction_guards() {
        assert!(!can_auto_collapse(
            true, false, false, false, false, false, true
        ));
        for (focused, inside) in [(true, false), (false, true), (false, false)] {
            assert!(can_auto_collapse(
                focused, false, false, false, false, false, inside
            ));
            for at in 0..5 {
                let mut guards = [false; 5];
                guards[at] = true;
                assert!(!can_auto_collapse(
                    focused, guards[0], guards[1], guards[2], guards[3], guards[4], inside
                ));
            }
        }
    }
    #[test]
    fn panel_layout_rejects_invalid_coordinates_and_undersized_widgets() {
        let mut g = Layout {
            x: 0.0,
            y: 50.0,
            width: 300.0,
            height: 180.0,
        };
        assert!(g.validate(200.0, 100.0).is_ok());
        g.x = -1.0;
        assert!(g.validate(200.0, 100.0).is_err());
        g.x = 0.0;
        g.height = 99.0;
        assert!(g.validate(200.0, 100.0).is_err());
        g.height = f64::INFINITY;
        assert!(g.validate(200.0, 100.0).is_err());
    }
}
