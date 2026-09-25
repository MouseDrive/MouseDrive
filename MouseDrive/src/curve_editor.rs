#![deny(unsafe_code)]

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, emath, pos2, vec2};

use crate::curve::{Curve, MAX_POINTS, MIN_T_SPACING, MIN_V_SPACING};

const EDITOR_HEIGHT: f32 = 160.0;
const EDITOR_MAX_WIDTH: f32 = 300.0;
const HANDLE_SIZE: f32 = 14.0;
const CURVE_SAMPLES: usize = 64;

#[derive(Clone, Copy, PartialEq)]
pub enum CurveDisplay {
    Normal,
    MirrorX,
    MirrorY,
}

pub fn curve_editor(
    ui: &mut egui::Ui,
    curve: &mut Curve,
    display: CurveDisplay,
    live_phase: Option<f64>,
) -> bool {
    let flip_x = display == CurveDisplay::MirrorX;
    let flip_y = display == CurveDisplay::MirrorY;
    let mut changed = false;

    let width = ui.available_width().min(EDITOR_MAX_WIDTH);
    let (response, painter) =
        ui.allocate_painter(vec2(width, EDITOR_HEIGHT), Sense::click_and_drag());
    let rect = response.rect;

    let to_screen =
        emath::RectTransform::from_to(Rect::from_min_size(Pos2::ZERO, vec2(1.0, 1.0)), rect);
    let to_norm = to_screen.inverse();

    let curve_to_screen = |t: f64, v: f64| -> Pos2 {
        let x = if flip_x { 1.0 - t } else { t } as f32;
        let y = if flip_y { v as f32 } else { 1.0 - v as f32 };
        to_screen.transform_pos(pos2(x, y))
    };
    let screen_to_curve = |p: Pos2| -> (f64, f64) {
        let n = to_norm.transform_pos(p);
        let t = if flip_x { 1.0 - n.x as f64 } else { n.x as f64 };
        let v = if flip_y { n.y as f64 } else { 1.0 - n.y as f64 };
        (t.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
    };

    painter.rect_filled(rect, 4.0, ui.visuals().extreme_bg_color);
    let grid_stroke = Stroke::new(1.0, Color32::from_gray(55));
    for i in 1..4 {
        let f = i as f32 / 4.0;
        painter.line_segment(
            [
                to_screen.transform_pos(pos2(f, 0.0)),
                to_screen.transform_pos(pos2(f, 1.0)),
            ],
            grid_stroke,
        );
        painter.line_segment(
            [
                to_screen.transform_pos(pos2(0.0, f)),
                to_screen.transform_pos(pos2(1.0, f)),
            ],
            grid_stroke,
        );
    }
    let frame_stroke = Stroke::new(1.0, Color32::from_gray(80));
    painter.line_segment([rect.left_top(), rect.right_top()], frame_stroke);
    painter.line_segment([rect.right_top(), rect.right_bottom()], frame_stroke);
    painter.line_segment([rect.right_bottom(), rect.left_bottom()], frame_stroke);
    painter.line_segment([rect.left_bottom(), rect.left_top()], frame_stroke);

    let samples: Vec<Pos2> = (0..=CURVE_SAMPLES)
        .map(|i| {
            let t = i as f64 / CURVE_SAMPLES as f64;
            curve_to_screen(t, curve.eval(t))
        })
        .collect();
    painter.add(Shape::line(
        samples,
        Stroke::new(2.0, ui.visuals().selection.stroke.color),
    ));

    let n = curve.points.len();
    let mut remove_idx: Option<usize> = None;
    for i in 0..n {
        let [t, v] = curve.points[i];
        let screen_pos = curve_to_screen(t, v);
        let is_endpoint = i == 0 || i == n - 1;
        let point_rect = Rect::from_center_size(screen_pos, vec2(HANDLE_SIZE, HANDLE_SIZE));
        let pr = ui.interact(point_rect, response.id.with(i), Sense::click_and_drag());

        if !is_endpoint {
            if pr.dragged() {
                let (nt, nv) = screen_to_curve(screen_pos + pr.drag_delta());
                let t_lo = curve.points[i - 1][0] + MIN_T_SPACING;
                let t_hi = curve.points[i + 1][0] - MIN_T_SPACING;
                let v_lo = curve.points[i - 1][1] + MIN_V_SPACING;
                let v_hi = curve.points[i + 1][1] - MIN_V_SPACING;
                if t_lo <= t_hi && v_lo <= v_hi {
                    let np = [nt.clamp(t_lo, t_hi), nv.clamp(v_lo, v_hi)];
                    if curve.points[i] != np {
                        curve.points[i] = np;
                        changed = true;
                    }
                }
            }
            if pr.secondary_clicked() {
                remove_idx = Some(i);
            }
        }

        let radius = if pr.hovered() || pr.dragged() {
            6.0
        } else {
            4.5
        };
        let color = if is_endpoint {
            ui.visuals().weak_text_color()
        } else if pr.dragged() {
            ui.visuals().warn_fg_color
        } else {
            ui.visuals().strong_text_color()
        };
        painter.circle_filled(screen_pos, radius, color);
    }

    if let Some(i) = remove_idx {
        curve.points.remove(i);
        changed = true;
    }

    if response.double_clicked()
        && curve.points.len() < MAX_POINTS
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (t, v) = screen_to_curve(pos);
        let idx = curve
            .points
            .iter()
            .position(|p| p[0] > t)
            .unwrap_or(curve.points.len() - 1)
            .max(1);
        let t_lo = curve.points[idx - 1][0] + MIN_T_SPACING;
        let t_hi = curve.points[idx][0] - MIN_T_SPACING;
        let v_lo = curve.points[idx - 1][1] + MIN_V_SPACING;
        let v_hi = curve.points[idx][1] - MIN_V_SPACING;
        if t_lo <= t_hi && v_lo <= v_hi {
            curve
                .points
                .insert(idx, [t.clamp(t_lo, t_hi), v.clamp(v_lo, v_hi)]);
            changed = true;
        }
    }

    if let Some(p) = live_phase {
        let p = p.clamp(0.0, 1.0);
        let pos = curve_to_screen(p, curve.eval(p));
        painter.circle_filled(pos, 5.0, ui.visuals().warn_fg_color);
    }

    changed
}
