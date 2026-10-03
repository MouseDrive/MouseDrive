#![deny(unsafe_code)]

use std::f32::consts::{PI, TAU};

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{
    Color32, FontId, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, WidgetInfo, WidgetType, pos2, vec2,
};

use super::theme;

const BOUNDS: Rect = Rect::from_min_max(pos2(14.0, 10.0), pos2(114.0, 102.0));
const RIM_CENTER: Pos2 = pos2(64.0, 60.0);
const RIM_RADIUS: f32 = 45.0;
const RIM_WIDTH: f32 = 10.0;
const RIM_CUT_Y: f32 = 97.0;
const RIM_STEPS: usize = 48;
const SPOKE: [Pos2; 8] = [
    pos2(21.0, 57.5),
    pos2(50.0, 54.0),
    pos2(78.0, 54.0),
    pos2(107.0, 57.5),
    pos2(107.0, 70.5),
    pos2(78.0, 75.0),
    pos2(50.0, 75.0),
    pos2(21.0, 70.5),
];
const MARKER: Rect = Rect::from_min_max(pos2(59.5, 10.0), pos2(68.5, 20.0));
const CURSOR: [Pos2; 7] = [
    pos2(51.0, 31.0),
    pos2(51.0, 76.5),
    pos2(62.38, 66.0),
    pos2(69.9, 81.75),
    pos2(76.55, 78.6),
    pos2(69.2, 63.55),
    pos2(83.55, 63.55),
];
const CURSOR_EDGE: f32 = 3.0;
const CURSOR_GAP: f32 = 4.0;

const WORDMARK: [(&str, Color32); 2] = [("Mouse", theme::TEXT), ("Drive", theme::ACCENT)];
const WORD_SIZE: f32 = 24.0;
const MARK_HEIGHT: f32 = 27.0;
const MARK_GAP: f32 = 11.0;

fn width_for(height: f32) -> f32 {
    height * BOUNDS.aspect_ratio()
}

pub fn lockup(ui: &mut Ui, background: Color32) {
    let mut job = LayoutJob::default();
    for (text, color) in WORDMARK {
        job.append(
            text,
            0.0,
            TextFormat::simple(FontId::proportional(WORD_SIZE), color),
        );
    }
    let galley = ui.fonts(|fonts| fonts.layout_job(job));
    let mark = vec2(width_for(MARK_HEIGHT), MARK_HEIGHT);
    let bold = 1.0 / ui.ctx().pixels_per_point();
    let word = galley.size() + vec2(bold, 0.0);
    let size = vec2(mark.x + MARK_GAP + word.x, mark.y.max(word.y));
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, galley.text()));
    if !ui.is_rect_visible(rect) {
        return;
    }
    let mark = Rect::from_min_size(pos2(rect.left(), rect.center().y - mark.y / 2.0), mark);
    let painter = ui.painter();
    painter.extend(shapes(mark, background));
    let text = pos2(mark.right() + MARK_GAP, rect.center().y - word.y / 2.0);
    for dx in [0.0, bold] {
        painter.galley(text + vec2(dx, 0.0), galley.clone(), theme::TEXT);
    }
}

fn shapes(rect: Rect, background: Color32) -> Vec<Shape> {
    let scale = rect.height() / BOUNDS.height();
    let at = |p: Pos2| rect.min + (p - BOUNDS.min) * scale;
    let mut shapes = vec![
        Shape::closed_line(
            rim().map(at).collect(),
            Stroke::new(RIM_WIDTH * scale, theme::TEXT),
        ),
        Shape::convex_polygon(SPOKE.map(at).to_vec(), theme::TEXT, Stroke::NONE),
        Shape::rect_filled(
            Rect::from_min_max(at(MARKER.min), at(MARKER.max)),
            0.0,
            theme::ACCENT,
        ),
    ];
    let layers = [
        (background, CURSOR_EDGE + 2.0 * CURSOR_GAP),
        (theme::ACCENT, CURSOR_EDGE),
    ];
    for (color, width) in layers {
        shapes.extend(cursor_parts().map(|part| {
            let points = part.into_iter().map(at).collect();
            Shape::convex_polygon(points, color, Stroke::NONE)
        }));
        let stroke = Stroke::new(width * scale, color);
        for (i, &corner) in CURSOR.iter().enumerate() {
            let next = CURSOR[(i + 1) % CURSOR.len()];
            shapes.push(Shape::line_segment([at(corner), at(next)], stroke));
            shapes.push(Shape::circle_filled(at(corner), stroke.width / 2.0, color));
        }
    }
    shapes
}

fn rim() -> impl Iterator<Item = Pos2> {
    let end = ((RIM_CUT_Y - RIM_CENTER.y) / RIM_RADIUS).asin();
    let (start, stop) = (PI - end, TAU + end);
    (0..=RIM_STEPS).map(move |i| {
        let angle = start + (stop - start) * i as f32 / RIM_STEPS as f32;
        RIM_CENTER + RIM_RADIUS * Vec2::angled(angle)
    })
}

fn cursor_parts() -> [Vec<Pos2>; 3] {
    let [
        tip,
        left,
        inner_left,
        tail_left,
        tail_right,
        inner_right,
        right,
    ] = CURSOR;
    let past_inner = inner_left + (inner_left - left) * 0.15;
    let into_head = (inner_left.lerp(inner_right, 0.5) - tail_left.lerp(tail_right, 0.5)) * 0.2;
    [
        vec![tip, past_inner, left],
        vec![tip, right, inner_left],
        vec![
            inner_left + into_head,
            inner_right + into_head,
            tail_right,
            tail_left,
        ],
    ]
}
