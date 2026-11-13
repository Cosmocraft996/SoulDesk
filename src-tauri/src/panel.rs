//! 面板偏好及逻辑坐标；旧偏好缺少新增字段时使用兼容默认值。
use serde::{Deserialize, Serialize};

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
    manual: bool,
    editing: bool,
    interacting: bool,
    menu: bool,
    dialog: bool,
    detail: bool,
    inside: bool,
) -> bool {
    !manual && !editing && !interacting && !menu && !dialog && !detail && !inside
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn hover_closes_immediately_only_when_no_interaction_protects_it() {
        assert!(can_auto_collapse(
            false, false, false, false, false, false, false
        ));
        for at in 0..7 {
            let mut guards = [false; 7];
            guards[at] = true;
            assert!(!can_auto_collapse(
                guards[0], guards[1], guards[2], guards[3], guards[4], guards[5], guards[6]
            ));
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
