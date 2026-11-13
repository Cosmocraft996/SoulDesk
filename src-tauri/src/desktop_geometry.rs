//! 桌面侧栏的纯物理坐标计算，不读取系统或业务数据。
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct WorkArea {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelGeometry {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub requested_ratio: f64,
    pub effective_ratio: f64,
}

pub fn panel_geometry(
    area: WorkArea,
    ratio: f64,
    min_width_logical: f64,
) -> Result<PanelGeometry, String> {
    if !ratio.is_finite() || ratio <= 0.0 || ratio > 1.0 {
        return Err("比例必须为大于 0 且不超过 1 的有限数".into());
    }
    if area.width == 0
        || area.height == 0
        || !area.scale_factor.is_finite()
        || area.scale_factor <= 0.0
        || !min_width_logical.is_finite()
        || min_width_logical < 0.0
    {
        return Err("工作区、缩放或最小宽度无效".into());
    }
    let right = i64::from(area.left) + i64::from(area.width);
    let bottom = i64::from(area.top) + i64::from(area.height);
    if right > i64::from(i32::MAX) || bottom > i64::from(i32::MAX) {
        return Err("工作区坐标超出原生窗口范围".into());
    }
    let width = (area.width as f64 * ratio)
        .ceil()
        .max((min_width_logical * area.scale_factor).ceil())
        .min(area.width as f64) as u32;
    Ok(PanelGeometry {
        left: (right - i64::from(width)) as i32,
        top: area.top,
        width,
        height: area.height,
        requested_ratio: ratio,
        effective_ratio: width as f64 / area.width as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn area() -> WorkArea {
        WorkArea {
            left: 0,
            top: 0,
            width: 1920,
            height: 1040,
            scale_factor: 1.0,
        }
    }
    #[test]
    fn right_edge_and_height_follow_work_area() {
        let g = panel_geometry(area(), 0.4, 280.0).unwrap();
        assert_eq!((g.left, g.top, g.width, g.height), (1152, 0, 768, 1040));
        let g = panel_geometry(
            WorkArea {
                left: -1920,
                top: -100,
                ..area()
            },
            0.4,
            280.0,
        )
        .unwrap();
        assert_eq!((g.left, g.top, g.width, g.height), (-768, -100, 768, 1040));
    }
    #[test]
    fn minimum_uses_current_scale_and_never_exceeds_work_area() {
        let g = panel_geometry(
            WorkArea {
                scale_factor: 1.5,
                ..area()
            },
            0.2,
            280.0,
        )
        .unwrap();
        assert_eq!(g.width, 420);
        assert_eq!(g.effective_ratio, 0.21875);
        let g = panel_geometry(
            WorkArea {
                width: 240,
                ..area()
            },
            0.4,
            280.0,
        )
        .unwrap();
        assert_eq!((g.left, g.width, g.effective_ratio), (0, 240, 1.0));
    }
    #[test]
    fn accepts_full_legal_ratio_range_and_rounds_up() {
        assert_eq!(panel_geometry(area(), 0.01, 0.0).unwrap().width, 20);
        assert_eq!(panel_geometry(area(), 1.0, 280.0).unwrap().left, 0);
        assert_eq!(
            panel_geometry(
                WorkArea {
                    width: 1001,
                    ..area()
                },
                0.4,
                0.0
            )
            .unwrap()
            .width,
            401
        );
    }
    #[test]
    fn rejects_invalid_ratios_dimensions_scales_and_overflow() {
        for ratio in [f64::NAN, f64::INFINITY, 0.0, -0.1, 1.01] {
            assert!(panel_geometry(area(), ratio, 280.0).is_err());
        }
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(panel_geometry(
                WorkArea {
                    scale_factor: scale,
                    ..area()
                },
                0.4,
                280.0
            )
            .is_err());
        }
        for min in [-1.0, f64::NAN, f64::INFINITY] {
            assert!(panel_geometry(area(), 0.4, min).is_err());
        }
        assert!(panel_geometry(WorkArea { width: 0, ..area() }, 0.4, 280.0).is_err());
        assert!(panel_geometry(
            WorkArea {
                height: 0,
                ..area()
            },
            0.4,
            280.0
        )
        .is_err());
        assert!(panel_geometry(
            WorkArea {
                left: i32::MAX,
                ..area()
            },
            0.4,
            280.0
        )
        .is_err());
    }
}
