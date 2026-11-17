//! 桌面组件与侧栏的纯物理坐标计算，不读取系统或业务数据。
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

/// 拖动面板左沿，只改变宽度；以当前实际宽度起算，避免最小宽度造成跳变。
pub fn resized_panel_geometry(
    area: WorkArea,
    initial_width: u32,
    delta_x: i64,
    min_width_logical: f64,
) -> Result<PanelGeometry, String> {
    let mut geometry = panel_geometry(area, 1.0, min_width_logical)?;
    let minimum = (min_width_logical * area.scale_factor)
        .ceil()
        .max(1.0)
        .min(area.width as f64) as i64;
    geometry.width = i64::from(initial_width)
        .saturating_sub(delta_x)
        .clamp(minimum, i64::from(area.width)) as u32;
    geometry.left =
        (i64::from(area.left) + i64::from(area.width) - i64::from(geometry.width)) as i32;
    geometry.effective_ratio = geometry.width as f64 / area.width as f64;
    geometry.requested_ratio = geometry.effective_ratio;
    Ok(geometry)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SnapEdge {
    Left,
    Right,
    Top,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapMatch {
    pub edge: SnapEdge,
    pub peer: Option<usize>,
    pub peer_edge: SnapEdge,
}

/// 同时返回真正选中的边，供双方高亮；视觉反馈不参与下一帧几何计算。
pub fn snap_geometry(
    rect: (i32, i32, u32, u32),
    resize: bool,
    area: WorkArea,
    peers: &[(i32, i32, u32, u32)],
    min_size: (u32, u32),
    max_size: (u32, u32),
    matches: &mut Vec<SnapMatch>,
) -> (i32, i32, u32, u32) {
    use SnapEdge::*;
    matches.clear();
    let (x, y, width, height) = rect;
    let right = i64::from(area.left) + i64::from(area.width);
    let bottom = i64::from(area.top) + i64::from(area.height);
    let mut xs = vec![(i64::from(area.left), None, Left), (right, None, Right)];
    let mut ys = vec![(i64::from(area.top), None, Top), (bottom, None, Bottom)];
    for (peer, &(px, py, pw, ph)) in peers.iter().enumerate() {
        let pr = i64::from(px) + i64::from(pw);
        let pb = i64::from(py) + i64::from(ph);
        // 只使用与当前工作区相交的组件边缘；可见性由调用方筛选。
        if pw > 0
            && ph > 0
            && i64::from(px) < right
            && pr > i64::from(area.left)
            && i64::from(py) < bottom
            && pb > i64::from(area.top)
        {
            xs.extend([(i64::from(px), Some(peer), Left), (pr, Some(peer), Right)]);
            ys.extend([(i64::from(py), Some(peer), Top), (pb, Some(peer), Bottom)]);
        }
    }
    let threshold = (8. * area.scale_factor).round() as i64;
    let mut nearest = |value: i64, candidates: Vec<(i64, SnapMatch)>, min: i64, max: i64| {
        let mut result = value;
        let mut distance = threshold + 1;
        let mut selected = None;
        for (candidate, mark) in candidates {
            let delta = (candidate - value).abs();
            if candidate >= min && candidate <= max && delta < distance {
                result = candidate;
                distance = delta;
                selected = Some(mark);
            }
        }
        if let Some(mark) = selected {
            matches.push(mark);
        }
        result
    };
    if resize {
        let max_width = (right - i64::from(x))
            .min(i64::from(max_size.0))
            .max(i64::from(min_size.0));
        let max_height = (bottom - i64::from(y))
            .min(i64::from(max_size.1))
            .max(i64::from(min_size.1));
        let width = i64::from(width).clamp(i64::from(min_size.0), max_width);
        let height = i64::from(height).clamp(i64::from(min_size.1), max_height);
        (
            x,
            y,
            nearest(
                width,
                xs.into_iter()
                    .map(|(at, peer, peer_edge)| {
                        (
                            at - i64::from(x),
                            SnapMatch {
                                edge: Right,
                                peer,
                                peer_edge,
                            },
                        )
                    })
                    .collect(),
                i64::from(min_size.0),
                max_width,
            ) as u32,
            nearest(
                height,
                ys.into_iter()
                    .map(|(at, peer, peer_edge)| {
                        (
                            at - i64::from(y),
                            SnapMatch {
                                edge: Bottom,
                                peer,
                                peer_edge,
                            },
                        )
                    })
                    .collect(),
                i64::from(min_size.1),
                max_height,
            ) as u32,
        )
    } else {
        (
            nearest(
                i64::from(x),
                xs.into_iter()
                    .flat_map(|(at, peer, peer_edge)| {
                        [
                            (
                                at,
                                SnapMatch {
                                    edge: Left,
                                    peer,
                                    peer_edge,
                                },
                            ),
                            (
                                at - i64::from(width),
                                SnapMatch {
                                    edge: Right,
                                    peer,
                                    peer_edge,
                                },
                            ),
                        ]
                    })
                    .collect(),
                i64::from(area.left),
                right - i64::from(width),
            ) as i32,
            nearest(
                i64::from(y),
                ys.into_iter()
                    .flat_map(|(at, peer, peer_edge)| {
                        [
                            (
                                at,
                                SnapMatch {
                                    edge: Top,
                                    peer,
                                    peer_edge,
                                },
                            ),
                            (
                                at - i64::from(height),
                                SnapMatch {
                                    edge: Bottom,
                                    peer,
                                    peer_edge,
                                },
                            ),
                        ]
                    })
                    .collect(),
                i64::from(area.top),
                bottom - i64::from(height),
            ) as i32,
            width,
            height,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snap_feedback_identifies_chosen_edges_and_clears_when_detached() {
        use SnapEdge::*;
        let peers = [(300, 250, 200, 180)];
        let mut matches = vec![];
        snap_geometry(
            (94, 74, 200, 180),
            false,
            area(),
            &peers,
            (180, 100),
            (1600, 1200),
            &mut matches,
        );
        let expected = vec![
            SnapMatch {
                edge: Right,
                peer: Some(0),
                peer_edge: Left,
            },
            SnapMatch {
                edge: Bottom,
                peer: Some(0),
                peer_edge: Top,
            },
        ];
        assert_eq!(matches, expected);
        snap_geometry(
            (20, 20, 274, 224),
            true,
            area(),
            &peers,
            (180, 100),
            (1600, 1200),
            &mut matches,
        );
        assert_eq!(matches, expected);
        snap_geometry(
            (88, 60, 200, 180),
            false,
            area(),
            &peers,
            (180, 100),
            (1600, 1200),
            &mut matches,
        );
        assert!(matches.is_empty());
        snap_geometry(
            (7, 35, 200, 180),
            false,
            area(),
            &peers,
            (180, 100),
            (1600, 1200),
            &mut matches,
        );
        assert_eq!(
            matches,
            vec![SnapMatch {
                edge: Left,
                peer: None,
                peer_edge: Left
            }]
        );
        assert_eq!(
            serde_json::to_value([Left, Right, Top, Bottom]).unwrap(),
            serde_json::json!(["left", "right", "top", "bottom"])
        );
    }
    #[test]
    fn snap_feedback_preserves_original_peer_index_and_nearest_target() {
        let peers = [
            (-500, 250, 200, 180),
            (300, 250, 200, 180),
            (304, 254, 200, 180),
        ];
        let mut matches = vec![];
        snap_geometry(
            (302, 252, 200, 180),
            false,
            area(),
            &peers,
            (180, 100),
            (1600, 1200),
            &mut matches,
        );
        assert!(matches.iter().all(|mark| mark.peer == Some(1)));
        snap_geometry(
            (303, 253, 200, 180),
            false,
            area(),
            &peers,
            (180, 100),
            (1600, 1200),
            &mut matches,
        );
        assert!(matches.iter().all(|mark| mark.peer == Some(2)));
    }
    fn snap(
        rect: (i32, i32, u32, u32),
        resize: bool,
        peers: &[(i32, i32, u32, u32)],
    ) -> (i32, i32, u32, u32) {
        snap_geometry(
            rect,
            resize,
            area(),
            peers,
            (180, 100),
            (1600, 1200),
            &mut vec![],
        )
    }
    #[test]
    fn dragging_snaps_same_and_adjacent_edges_and_releases_beyond_threshold() {
        let peers = [(300, 250, 200, 180)];
        assert_eq!(snap((94, 74, 200, 180), false, &peers), (100, 70, 200, 180));
        assert_eq!(
            snap((294, 244, 200, 180), false, &peers),
            (300, 250, 200, 180)
        );
        assert_eq!(snap((88, 60, 200, 180), false, &peers), (88, 60, 200, 180));
        assert_eq!(
            snap((108, 78, 200, 180), false, &peers),
            (100, 70, 200, 180)
        );
        assert_eq!(
            snap((109, 79, 200, 180), false, &peers),
            (109, 79, 200, 180)
        );
    }
    #[test]
    fn dragging_uses_work_area_edges_without_blocking_monitor_crossing() {
        assert_eq!(snap((7, 7, 200, 180), false, &[]), (0, 0, 200, 180));
        assert_eq!(
            snap((1714, 854, 200, 180), false, &[]),
            (1720, 860, 200, 180)
        );
        assert_eq!(
            snap((1910, 300, 200, 180), false, &[]),
            (1910, 300, 200, 180)
        );
    }
    #[test]
    fn resizing_snaps_only_active_edges_and_preserves_opposite_corner() {
        let peers = [(300, 250, 200, 180)];
        assert_eq!(snap((20, 20, 274, 224), true, &peers), (20, 20, 280, 230));
        assert_eq!(snap((20, 20, 474, 404), true, &peers), (20, 20, 480, 410));
        assert_eq!(
            snap((400, 300, 1600, 1200), true, &[]),
            (400, 300, 1520, 740)
        );
        assert_eq!(
            snap((20, 20, 180, 100), true, &[(195, 115, 1, 1)]),
            (20, 20, 180, 100)
        );
    }
    #[test]
    fn nearest_target_wins_with_stable_ties_and_exact_alignment() {
        let peers = [(300, 500, 200, 180), (304, 700, 200, 180)];
        assert_eq!(snap((302, 30, 200, 180), false, &peers).0, 300);
        assert_eq!(snap((303, 30, 200, 180), false, &peers).0, 304);
        assert_eq!(snap((300, 30, 200, 180), false, &peers).0, 300);
    }
    #[test]
    fn target_monitor_scale_controls_threshold_and_negative_coordinates_work() {
        let a = WorkArea {
            left: -1920,
            top: -100,
            scale_factor: 1.5,
            ..area()
        };
        let apply =
            |rect| snap_geometry(rect, false, a, &[], (180, 100), (1600, 1200), &mut vec![]);
        assert_eq!(apply((-1908, -88, 300, 270)), (-1920, -100, 300, 270));
        assert_eq!(apply((-1907, -87, 300, 270)), (-1907, -87, 300, 270));
        assert_eq!(apply((-308, 662, 300, 270)), (-300, 670, 300, 270));
    }
    #[test]
    fn widgets_entirely_outside_target_monitor_do_not_supply_alignment_edges() {
        let peers = [(-500, 250, 200, 180), (300, 2000, 200, 180)];
        assert_eq!(
            snap((294, 244, 200, 180), false, &peers),
            (294, 244, 200, 180)
        );
    }
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
    fn panel_left_edge_drag_changes_width_without_moving_right_edge_or_height() {
        let a = area();
        for (delta, width) in [
            (-192, 960),
            (100, 668),
            (10_000, 312),
            (-10_000, 1920),
            (0, 768),
        ] {
            let g = resized_panel_geometry(a, 768, delta, 312.).unwrap();
            assert_eq!(g.width, width);
            assert_eq!(i64::from(g.left) + i64::from(g.width), 1920);
            assert_eq!((g.top, g.height), (0, 1040));
            assert_eq!(g.effective_ratio, width as f64 / 1920.);
            assert_eq!(g.requested_ratio, g.effective_ratio);
        }
        // 从被最小宽度撑大的实际窗口起算，鼠标不动时不跳回请求比例。
        assert_eq!(resized_panel_geometry(a, 900, 0, 900.).unwrap().width, 900);
    }

    #[test]
    fn panel_edge_drag_uses_target_scale_negative_origin_and_narrow_work_area() {
        let a = WorkArea {
            left: -1920,
            top: -100,
            scale_factor: 1.5,
            ..area()
        };
        let g = resized_panel_geometry(a, 768, 1000, 312.).unwrap();
        assert_eq!((g.left, g.top, g.width, g.height), (-468, -100, 468, 1040));
        let narrow = WorkArea { width: 240, ..a };
        let g = resized_panel_geometry(narrow, 240, i64::MAX, 600.).unwrap();
        assert_eq!((g.left, g.width, g.effective_ratio), (-1920, 240, 1.));
        assert!(resized_panel_geometry(WorkArea { width: 0, ..a }, 768, 0, 312.).is_err());
        assert!(resized_panel_geometry(a, 768, 0, f64::NAN).is_err());
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
