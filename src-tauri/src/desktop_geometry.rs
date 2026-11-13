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

/// 初次摆放优先找工作区内的空位；空间不足时保留可拖动的错位位置。
pub fn initial_position(
    area: WorkArea,
    width: u32,
    height: u32,
    occupied: &[(i32, i32, u32, u32)],
) -> (i32, i32) {
    let margin = (16. * area.scale_factor).round() as i32;
    let right = area.left + area.width.saturating_sub(width) as i32;
    let bottom = area.top + area.height.saturating_sub(height) as i32;
    let mut y = (area.top + margin).min(bottom);
    while y <= bottom {
        let mut x = (area.left + margin).min(right);
        while x <= right {
            if occupied.iter().all(|(ox, oy, ow, oh)| {
                x as i64 + width as i64 + margin as i64 <= *ox as i64
                    || x as i64 >= *ox as i64 + *ow as i64 + margin as i64
                    || y as i64 + height as i64 + margin as i64 <= *oy as i64
                    || y as i64 >= *oy as i64 + *oh as i64 + margin as i64
            }) {
                return (x, y);
            }
            x = x.saturating_add(width as i32 + margin.max(1));
        }
        y = y.saturating_add(height as i32 + margin.max(1));
    }
    let offset = (occupied.len() % 7) as i32 * margin;
    (
        (area.left + margin + offset).min(right),
        (area.top + margin + offset).min(bottom),
    )
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
    fn initial_placement_uses_free_space_and_keeps_overflow_reachable() {
        let a = area();
        let first = initial_position(a, 400, 300, &[]);
        let second = initial_position(a, 400, 300, &[(first.0, first.1, 400, 300)]);
        assert!(second.0 >= first.0 + 400 || second.1 >= first.1 + 300);
        let p = initial_position(a, 1920, 1040, &[(0, 0, 1920, 1040)]);
        assert_eq!(p, (0, 0));
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
