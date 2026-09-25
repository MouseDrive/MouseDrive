#![deny(unsafe_code)]

use eframe::egui::{
    Button, ComboBox, DragValue, EventFilter, Id, Key, Painter, Pos2, Rect, Response, RichText,
    Sense, Shape, Stroke, StrokeKind, Ui, WidgetInfo, WidgetType, emath, pos2, vec2,
};
use mousedrive::curve::{Curve, CurveMode, CurvePreset};

use crate::lang::{Lang, Strings, fill, parse_num};

const EDITOR_HEIGHT: f32 = 150.0;
const EDITOR_MAX_WIDTH: f32 = 320.0;
const HANDLE_SIZE: f32 = 14.0;
const CURVE_SAMPLES: usize = 64;
const KEY_STEP: f64 = 0.02;
const KEY_STEP_FINE: f64 = 0.005;
const PRESETS: [CurvePreset; 4] = [
    CurvePreset::Linear,
    CurvePreset::SCurve,
    CurvePreset::Aggressive,
    CurvePreset::Progressive,
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CurveDisplay {
    Normal,
    MirrorX,
    MirrorY,
}

struct Mapping {
    to_screen: emath::RectTransform,
    flip_x: bool,
    flip_y: bool,
}

impl Mapping {
    fn new(rect: Rect, display: CurveDisplay) -> Self {
        Self {
            to_screen: emath::RectTransform::from_to(
                Rect::from_min_size(Pos2::ZERO, vec2(1.0, 1.0)),
                rect,
            ),
            flip_x: display == CurveDisplay::MirrorX,
            flip_y: display == CurveDisplay::MirrorY,
        }
    }

    fn screen(&self, t: f64, v: f64) -> Pos2 {
        let x = if self.flip_x { 1.0 - t } else { t } as f32;
        let y = if self.flip_y { v } else { 1.0 - v } as f32;
        self.to_screen.transform_pos(pos2(x, y))
    }

    fn curve(&self, p: Pos2) -> (f64, f64) {
        let n = self.to_screen.inverse().transform_pos(p);
        let (x, y) = (f64::from(n.x), f64::from(n.y));
        let t = if self.flip_x { 1.0 - x } else { x };
        let v = if self.flip_y { y } else { 1.0 - y };
        (t.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
    }

    fn key_delta(&self, right: f64, up: f64, step: f64) -> (f64, f64) {
        let sx = if self.flip_x { -1.0 } else { 1.0 };
        let sy = if self.flip_y { -1.0 } else { 1.0 };
        (right * step * sx, up * step * sy)
    }
}

pub fn curve_editor(
    ui: &mut Ui,
    title: &str,
    curve: &mut Curve,
    display: CurveDisplay,
    s: &Strings,
    lang: Lang,
) -> bool {
    ui.label(RichText::new(title).strong());
    let id = ui.make_persistent_id(("curve_editor", title));
    let mut changed = header(ui, id, curve, s);
    let mut sel = load_selection(ui, id, curve);

    let width = ui.available_width().min(EDITOR_MAX_WIDTH);
    let (response, painter) =
        ui.allocate_painter(vec2(width, EDITOR_HEIGHT), Sense::click_and_drag());
    let map = Mapping::new(response.rect, display);
    paint_background(ui, &painter, &map, response.has_focus());
    paint_curve(ui, &painter, &map, curve);

    changed |= pointer_edits(ui, &painter, &response, &map, curve, &mut sel);
    if response.has_focus() {
        lock_arrow_keys(ui, response.id);
        changed |= keyboard_edits(ui, &map, curve, &mut sel);
    }
    describe(&response, title, curve, sel, s, lang);
    changed |= point_fields(ui, curve, sel, s, lang);
    ui.label(RichText::new(s.curve_hint).small().weak());
    ui.data_mut(|d| d.insert_temp(id, sel));
    changed
}

fn header(ui: &mut Ui, id: Id, curve: &mut Curve, s: &Strings) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let before = curve.mode;
        let mode_name = match CurveMode::from_i32(curve.mode) {
            CurveMode::Linear => s.curve_linear,
            CurveMode::Smooth => s.curve_smooth,
        };
        ComboBox::new(id.with("mode"), s.curve_mode)
            .selected_text(mode_name)
            .width(90.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut curve.mode, 0, s.curve_linear);
                ui.selectable_value(&mut curve.mode, 1, s.curve_smooth);
            });
        changed |= curve.mode != before;
        ComboBox::from_id_salt(id.with("preset"))
            .selected_text(s.curve_preset)
            .width(110.0)
            .show_ui(ui, |ui| {
                for (preset, name) in PRESETS.iter().zip(s.preset_names) {
                    if ui.selectable_label(false, name).clicked() {
                        *curve = Curve::preset(*preset);
                        changed = true;
                    }
                }
            });
        if ui
            .add_enabled(!curve.is_identity(), Button::new(s.curve_reset))
            .clicked()
        {
            *curve = Curve::default();
            changed = true;
        }
    });
    changed
}

fn load_selection(ui: &Ui, id: Id, curve: &Curve) -> Option<usize> {
    ui.data(|d| d.get_temp::<Option<usize>>(id))
        .flatten()
        .filter(|&i| i > 0 && i + 1 < curve.points.len())
}

fn paint_background(ui: &Ui, painter: &Painter, map: &Mapping, focused: bool) {
    let rect = map.to_screen.to();
    let visuals = ui.visuals();
    painter.rect_filled(*rect, 4.0, visuals.extreme_bg_color);
    let grid = Stroke::new(1.0_f32, visuals.widgets.noninteractive.bg_stroke.color);
    for i in 1..4 {
        let f = f64::from(i) / 4.0;
        painter.line_segment([map.screen(f, 0.0), map.screen(f, 1.0)], grid);
        painter.line_segment([map.screen(0.0, f), map.screen(1.0, f)], grid);
    }
    let frame = if focused {
        Stroke::new(2.0_f32, visuals.selection.bg_fill)
    } else {
        visuals.widgets.noninteractive.bg_stroke
    };
    painter.rect_stroke(*rect, 4.0, frame, StrokeKind::Inside);
}

fn paint_curve(ui: &Ui, painter: &Painter, map: &Mapping, curve: &Curve) {
    let samples: Vec<Pos2> = (0..=CURVE_SAMPLES)
        .map(|i| {
            let t = i as f64 / CURVE_SAMPLES as f64;
            map.screen(t, curve.eval(t))
        })
        .collect();
    painter.add(Shape::line(
        samples,
        Stroke::new(2.0_f32, ui.visuals().selection.bg_fill),
    ));
    let weak = ui.visuals().weak_text_color();
    if let (Some(first), Some(last)) = (curve.points.first(), curve.points.last()) {
        painter.circle_filled(map.screen(first[0], first[1]), 4.0, weak);
        painter.circle_filled(map.screen(last[0], last[1]), 4.0, weak);
    }
}

fn pointer_edits(
    ui: &Ui,
    painter: &Painter,
    area: &Response,
    map: &Mapping,
    curve: &mut Curve,
    sel: &mut Option<usize>,
) -> bool {
    let mut changed = false;
    let mut remove = None;
    for i in 1..curve.points.len().saturating_sub(1) {
        let [t, v] = curve.points[i];
        let pos = map.screen(t, v);
        let rect = Rect::from_center_size(pos, vec2(HANDLE_SIZE, HANDLE_SIZE));
        let handle = ui.interact(rect, area.id.with(i), Sense::click_and_drag());
        if handle.clicked() || handle.drag_started() {
            *sel = Some(i);
            area.request_focus();
        }
        if handle.dragged() {
            let (nt, nv) = map.curve(pos + handle.drag_delta());
            changed |= curve.move_point(i, nt, nv);
        }
        if handle.secondary_clicked() {
            remove = Some(i);
        }
        let [t, v] = curve.points[i];
        paint_handle(ui, painter, map.screen(t, v), *sel == Some(i), &handle);
    }
    if let Some(i) = remove {
        changed |= remove_point(curve, sel, i);
    }
    if area.double_clicked()
        && let Some(p) = area.interact_pointer_pos()
    {
        let (t, v) = map.curve(p);
        if let Some(i) = curve.insert_point(t, v) {
            *sel = Some(i);
            changed = true;
        }
    }
    changed
}

fn paint_handle(ui: &Ui, painter: &Painter, pos: Pos2, selected: bool, handle: &Response) {
    let visuals = ui.visuals();
    let active = handle.hovered() || handle.dragged();
    let radius = if active || selected { 6.0 } else { 4.5 };
    let color = if selected {
        visuals.warn_fg_color
    } else {
        visuals.strong_text_color()
    };
    painter.circle_filled(pos, radius, color);
    if selected {
        painter.circle_stroke(pos, radius + 2.5, Stroke::new(1.5_f32, color));
    }
}

fn remove_point(curve: &mut Curve, sel: &mut Option<usize>, i: usize) -> bool {
    if !curve.remove_point(i) {
        return false;
    }
    let middle = curve.points.len().saturating_sub(2);
    *sel = (middle > 0).then(|| i.min(middle));
    true
}

fn lock_arrow_keys(ui: &Ui, id: Id) {
    let filter = EventFilter {
        horizontal_arrows: true,
        vertical_arrows: true,
        ..Default::default()
    };
    ui.memory_mut(|m| m.set_focus_lock_filter(id, filter));
}

fn cycle(sel: Option<usize>, middle: usize, forward: bool) -> usize {
    match (sel, forward) {
        (None, true) => 1,
        (None, false) => middle,
        (Some(i), true) => i % middle + 1,
        (Some(i), false) => (i + middle - 2) % middle + 1,
    }
}

fn keyboard_edits(ui: &Ui, map: &Mapping, curve: &mut Curve, sel: &mut Option<usize>) -> bool {
    let middle = curve.points.len().saturating_sub(2);
    if middle == 0 {
        *sel = None;
        return false;
    }
    let (next, prev, delete, fine, right, up) = ui.input(|i| {
        let axis = |pos: Key, neg: Key| {
            f64::from(i8::from(i.key_pressed(pos)) - i8::from(i.key_pressed(neg)))
        };
        (
            i.key_pressed(Key::PageDown),
            i.key_pressed(Key::PageUp),
            i.key_pressed(Key::Delete),
            i.modifiers.shift,
            axis(Key::ArrowRight, Key::ArrowLeft),
            axis(Key::ArrowUp, Key::ArrowDown),
        )
    });
    let moving = right != 0.0 || up != 0.0;
    if next || (sel.is_none() && moving) {
        *sel = Some(cycle(*sel, middle, true));
    } else if prev {
        *sel = Some(cycle(*sel, middle, false));
    }
    let Some(i) = *sel else {
        return false;
    };
    if delete {
        return remove_point(curve, sel, i);
    }
    if !moving {
        return false;
    }
    let step = if fine { KEY_STEP_FINE } else { KEY_STEP };
    let (dt, dv) = map.key_delta(right, up, step);
    let [t, v] = curve.points[i];
    curve.move_point(i, t + dt, v + dv)
}

fn describe(
    area: &Response,
    title: &str,
    curve: &Curve,
    sel: Option<usize>,
    s: &Strings,
    lang: Lang,
) {
    let point = sel.map(|i| {
        let [t, v] = curve.points[i];
        fill(
            s.curve_point,
            &[
                ("i", &i.to_string()),
                ("t", &lang.pct(t, 0)),
                ("v", &lang.pct(v, 0)),
            ],
        )
    });
    let text = match point {
        Some(p) => format!("{title}. {p}. {}", s.curve_hint),
        None => format!("{title}. {}", s.curve_hint),
    };
    area.widget_info(|| WidgetInfo::labeled(WidgetType::Other, true, &text));
}

fn point_fields(
    ui: &mut Ui,
    curve: &mut Curve,
    sel: Option<usize>,
    s: &Strings,
    lang: Lang,
) -> bool {
    let Some(i) = sel else {
        return false;
    };
    let [t, v] = curve.points[i];
    let (mut t_pct, mut v_pct) = (t * 100.0, v * 100.0);
    ui.horizontal(|ui| {
        ui.label(format!("#{i}"));
        let t_label = ui.label(s.curve_time);
        ui.add(pct_field(&mut t_pct, lang)).labelled_by(t_label.id);
        let v_label = ui.label(s.curve_value);
        ui.add(pct_field(&mut v_pct, lang)).labelled_by(v_label.id);
    });
    curve.move_point(i, t_pct / 100.0, v_pct / 100.0)
}

fn pct_field(value: &mut f64, lang: Lang) -> DragValue<'_> {
    DragValue::new(value)
        .range(0.0..=100.0)
        .speed(0.25)
        .custom_formatter(move |v, _| lang.num(v, 1))
        .custom_parser(parse_num)
}
