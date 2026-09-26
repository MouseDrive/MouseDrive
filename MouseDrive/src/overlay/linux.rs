use std::error::Error;
use std::os::fd::{AsRawFd, RawFd};
use std::sync::atomic::Ordering;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont, point};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::randr::{self, ConnectionExt as _};
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClipOrdering, ColormapAlloc, ConfigureWindowAux,
    ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask, ImageFormat, ImageOrder, PropMode,
    Rectangle, Screen, StackMode, VisualClass, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::{
    ALPHA, Area, BACKGROUND, BASE_HEIGHT, BASE_MARGIN, BASE_WIDTH, Notice, STATE, TEXT,
    TOPMOST_REFRESH_MS, current_label, current_status, place, scaled,
};
use crate::platform::{self, Wake, pollfd};
use crate::status::status_rgb;

const NOTICE_CONFIG: u8 = 1;
const NOTICE_REDRAW: u8 = 2;
const NOTICE_STOP: u8 = 4;

const DEFAULT_DPI: i32 = 96;
const MAX_PUT_BYTES: usize = 60_000;

type XResult<T> = Result<T, Box<dyn Error>>;

static WAKE: Wake = Wake::new();

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_WORKAREA,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_NOTIFICATION,
    }
}

pub(super) fn start() -> std::io::Result<JoinHandle<()>> {
    let wake = WAKE.fd()?;
    WAKE.clear(NOTICE_STOP);
    std::thread::Builder::new()
        .name("overlay".into())
        .spawn(move || thread_main(wake))
}

pub(super) fn notify(notice: Notice) {
    WAKE.post(match notice {
        Notice::Config => NOTICE_CONFIG,
        Notice::Redraw => NOTICE_REDRAW,
        Notice::Stop => NOTICE_STOP,
    });
}

fn thread_main(wake: RawFd) {
    let Ok(font) = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT) else {
        crate::log::line("overlay yazı tipi yüklenemedi");
        return;
    };
    let refresh = Duration::from_millis(u64::from(TOPMOST_REFRESH_MS));
    let mut overlay: Option<Overlay> = None;
    let mut next_raise = Instant::now() + refresh;
    let mut notices = NOTICE_CONFIG;
    loop {
        if notices & NOTICE_STOP != 0 {
            break;
        }
        if let Err(e) = step(&mut overlay, notices, &font) {
            crate::log::line(&format!("overlay (X11): {e}"));
            overlay = None;
        }
        let now = Instant::now();
        if now >= next_raise {
            next_raise = now + refresh;
            if let Some(o) = overlay.as_mut().filter(|o| o.mapped)
                && o.raise().is_err()
            {
                overlay = None;
            }
        }

        let shown = overlay.as_ref().is_some_and(|o| o.mapped);
        let timeout = if shown {
            i32::try_from(next_raise.saturating_duration_since(now).as_millis()).unwrap_or(0)
        } else {
            -1
        };
        let x_fd = overlay.as_ref().map_or(-1, Overlay::fd);
        let mut fds = [pollfd(wake), pollfd(x_fd)];
        if let Err(e) = platform::poll(&mut fds, timeout) {
            crate::log::line(&format!("overlay poll başarısız: {e}"));
            break;
        }
        if fds[0].revents != 0 {
            platform::drain(wake);
        }
        notices = WAKE.take();
    }
}

fn step(overlay: &mut Option<Overlay>, notices: u8, font: &FontRef<'_>) -> XResult<()> {
    if notices & NOTICE_CONFIG != 0 {
        let enabled = STATE.enabled.load(Ordering::Acquire);
        if enabled && overlay.is_none() {
            *overlay = Some(Overlay::connect()?);
        }
        if let Some(o) = overlay.as_mut() {
            if enabled {
                o.show(font)?;
            } else {
                o.hide()?;
            }
        }
    }
    let Some(o) = overlay.as_mut() else {
        return Ok(());
    };
    let redraw = o.pump(font)?;
    if o.mapped && (redraw || notices & NOTICE_REDRAW != 0) {
        o.draw(font)?;
    }
    Ok(())
}

struct Overlay {
    conn: RustConnection,
    screen: usize,
    root: Window,
    window: Window,
    gc: u32,
    depth: u8,
    argb: bool,
    atoms: Atoms,
    size: (u16, u16),
    mapped: bool,
    error_logged: bool,
}

impl Overlay {
    fn connect() -> XResult<Self> {
        let (conn, screen) = x11rb::connect(None)?;
        let atoms = Atoms::new(&conn)?.reply()?;
        let cm_name = format!("_NET_WM_CM_S{screen}");
        let cm = conn.intern_atom(false, cm_name.as_bytes())?.reply()?.atom;
        let composited = conn.get_selection_owner(cm)?.reply()?.owner != x11rb::NONE;
        let info = &conn.setup().roots[screen];
        let root = info.root;
        let argb_visual = composited.then(|| argb_visual(info)).flatten();
        let (depth, visual) = argb_visual.unwrap_or((info.root_depth, info.root_visual));
        let bpp32 = conn
            .setup()
            .pixmap_formats
            .iter()
            .any(|f| f.depth == depth && f.bits_per_pixel == 32);
        if !bpp32 {
            return Err(format!("{depth} bit derinliğin piksel biçimi 32 bpp değil").into());
        }

        let colormap = conn.generate_id()?;
        conn.create_colormap(ColormapAlloc::NONE, colormap, root, visual)?;
        let window = conn.generate_id()?;
        let aux = CreateWindowAux::new()
            .background_pixel(0)
            .border_pixel(0)
            .override_redirect(1)
            .colormap(colormap)
            .event_mask(EventMask::EXPOSURE);
        conn.create_window(
            depth,
            window,
            root,
            0,
            0,
            BASE_WIDTH as u16,
            BASE_HEIGHT as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &aux,
        )?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            b"MouseDrive overlay",
        )?;
        conn.change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"mousedrive-overlay\0MouseDrive\0",
        )?;
        conn.change_property32(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_WINDOW_TYPE,
            AtomEnum::ATOM,
            &[atoms._NET_WM_WINDOW_TYPE_NOTIFICATION],
        )?;
        conn.shape_rectangles(
            shape::SO::SET,
            shape::SK::INPUT,
            ClipOrdering::UNSORTED,
            window,
            0,
            0,
            &[],
        )?;
        let gc = conn.generate_id()?;
        conn.create_gc(gc, window, &CreateGCAux::new().graphics_exposures(0))?;
        conn.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;
        let _ = conn.randr_select_input(root, randr::NotifyMask::SCREEN_CHANGE);
        conn.flush()?;
        Ok(Self {
            conn,
            screen,
            root,
            window,
            gc,
            depth,
            argb: argb_visual.is_some(),
            atoms,
            size: (BASE_WIDTH as u16, BASE_HEIGHT as u16),
            mapped: false,
            error_logged: false,
        })
    }

    fn fd(&self) -> RawFd {
        self.conn.stream().as_raw_fd()
    }

    fn show(&mut self, font: &FontRef<'_>) -> XResult<()> {
        self.layout()?;
        if !self.mapped {
            self.conn.map_window(self.window)?;
            self.mapped = true;
        }
        self.draw(font)
    }

    fn hide(&mut self) -> XResult<()> {
        if self.mapped {
            self.conn.unmap_window(self.window)?;
            self.conn.flush()?;
            self.mapped = false;
        }
        Ok(())
    }

    fn raise(&mut self) -> XResult<()> {
        let aux = ConfigureWindowAux::new().stack_mode(StackMode::ABOVE);
        self.conn.configure_window(self.window, &aux)?;
        self.conn.flush()?;
        Ok(())
    }

    fn layout(&mut self) -> XResult<()> {
        let dpi = self.dpi();
        let (w, h) = (scaled(BASE_WIDTH, dpi), scaled(BASE_HEIGHT, dpi));
        let corner = STATE.corner.load(Ordering::Acquire);
        let (x, y) = place(self.work_area(), w, h, corner, scaled(BASE_MARGIN, dpi));
        let (w, h) = (w.clamp(1, 4000) as u16, h.clamp(1, 1000) as u16);
        self.size = (w, h);
        let aux = ConfigureWindowAux::new()
            .x(x)
            .y(y)
            .width(u32::from(w))
            .height(u32::from(h))
            .stack_mode(StackMode::ABOVE);
        self.conn.configure_window(self.window, &aux)?;
        if !self.argb {
            self.conn.shape_rectangles(
                shape::SO::SET,
                shape::SK::BOUNDING,
                ClipOrdering::YX_BANDED,
                self.window,
                0,
                0,
                &pill_spans(w, h),
            )?;
        }
        Ok(())
    }

    fn pump(&mut self, font: &FontRef<'_>) -> XResult<bool> {
        let mut redraw = false;
        let mut relayout = false;
        while let Some(event) = self.conn.poll_for_event()? {
            match event {
                Event::Expose(e) if e.window == self.window && e.count == 0 => redraw = true,
                Event::PropertyNotify(e) if e.window == self.root => {
                    let resources = u32::from(AtomEnum::RESOURCE_MANAGER);
                    relayout |= e.atom == self.atoms._NET_WORKAREA || e.atom == resources;
                }
                Event::RandrScreenChangeNotify(_) => relayout = true,
                Event::Error(e) if !self.error_logged => {
                    self.error_logged = true;
                    crate::log::line(&format!("overlay (X11) protokol hatası: {e:?}"));
                }
                _ => {}
            }
        }
        if relayout && self.mapped {
            self.show(font)?;
            return Ok(false);
        }
        Ok(redraw)
    }

    fn draw(&mut self, font: &FontRef<'_>) -> XResult<()> {
        let (w, h) = self.size;
        let status = current_status();
        let pixels = render(
            usize::from(w),
            usize::from(h),
            status_rgb(status),
            &current_label(status),
            font,
            self.argb,
        );
        let lsb = self.conn.setup().image_byte_order == ImageOrder::LSB_FIRST;
        let width = usize::from(w);
        let rows_per_put = (MAX_PUT_BYTES / (width * 4)).max(1);
        for (i, band) in pixels.chunks(rows_per_put * width).enumerate() {
            let data: Vec<u8> = band
                .iter()
                .flat_map(|p| {
                    if lsb {
                        p.to_le_bytes()
                    } else {
                        p.to_be_bytes()
                    }
                })
                .collect();
            let rows = (band.len() / width) as u16;
            let top = (i * rows_per_put) as i16;
            self.conn.put_image(
                ImageFormat::Z_PIXMAP,
                self.window,
                self.gc,
                w,
                rows,
                0,
                top,
                0,
                self.depth,
                &data,
            )?;
        }
        self.conn.flush()?;
        Ok(())
    }

    fn dpi(&self) -> i32 {
        self.conn
            .get_property(
                false,
                self.root,
                AtomEnum::RESOURCE_MANAGER,
                AtomEnum::STRING,
                0,
                u32::MAX / 4,
            )
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| parse_xft_dpi(&String::from_utf8_lossy(&r.value)))
            .unwrap_or(DEFAULT_DPI)
    }

    fn work_area(&self) -> Area {
        let info = &self.conn.setup().roots[self.screen];
        let screen = Area {
            left: 0,
            top: 0,
            right: i32::from(info.width_in_pixels),
            bottom: i32::from(info.height_in_pixels),
        };
        let monitor = self.primary_monitor().unwrap_or(screen);
        self.net_workarea()
            .and_then(|work| intersect(monitor, work))
            .unwrap_or(monitor)
    }

    fn primary_monitor(&self) -> Option<Area> {
        let reply = self
            .conn
            .randr_get_monitors(self.root, true)
            .ok()?
            .reply()
            .ok()?;
        let m = reply
            .monitors
            .iter()
            .find(|m| m.primary)
            .or_else(|| reply.monitors.first())?;
        Some(Area {
            left: i32::from(m.x),
            top: i32::from(m.y),
            right: i32::from(m.x) + i32::from(m.width),
            bottom: i32::from(m.y) + i32::from(m.height),
        })
    }

    fn net_workarea(&self) -> Option<Area> {
        let reply = self
            .conn
            .get_property(
                false,
                self.root,
                self.atoms._NET_WORKAREA,
                AtomEnum::CARDINAL,
                0,
                4,
            )
            .ok()?
            .reply()
            .ok()?;
        let v: Vec<i32> = reply.value32()?.map(|n| n as i32).collect();
        let [x, y, w, h] = v.get(..4)?.try_into().ok()?;
        Some(Area {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        })
    }
}

fn argb_visual(screen: &Screen) -> Option<(u8, u32)> {
    screen
        .allowed_depths
        .iter()
        .filter(|d| d.depth == 32)
        .flat_map(|d| d.visuals.iter())
        .find(|v| {
            v.class == VisualClass::TRUE_COLOR
                && v.red_mask == 0x00FF_0000
                && v.green_mask == 0x0000_FF00
                && v.blue_mask == 0x0000_00FF
        })
        .map(|v| (32, v.visual_id))
}

fn parse_xft_dpi(resources: &str) -> Option<i32> {
    resources
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            if key.trim() != "Xft.dpi" {
                return None;
            }
            value.trim().parse::<f64>().ok()
        })
        .map(|dpi| dpi.round() as i32)
        .filter(|&dpi| dpi > 0)
}

fn intersect(a: Area, b: Area) -> Option<Area> {
    let area = Area {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.min(b.right),
        bottom: a.bottom.min(b.bottom),
    };
    (area.right > area.left && area.bottom > area.top).then_some(area)
}

fn capsule_coverage(px: f32, py: f32, w: f32, h: f32) -> f32 {
    let r = h / 2.0;
    let cx = px.clamp(r, (w - r).max(r));
    circle_coverage(px, py, cx, r, r)
}

fn circle_coverage(px: f32, py: f32, cx: f32, cy: f32, r: f32) -> f32 {
    let d = (px - cx).hypot(py - cy) - r;
    (0.5 - d).clamp(0.0, 1.0)
}

fn pill_spans(w: u16, h: u16) -> Vec<Rectangle> {
    let (wf, hf) = (f32::from(w), f32::from(h));
    let mut spans: Vec<Rectangle> = Vec::new();
    for y in 0..h {
        let py = f32::from(y) + 0.5;
        let Some(left) =
            (0..w.div_ceil(2)).find(|&x| capsule_coverage(f32::from(x) + 0.5, py, wf, hf) >= 0.5)
        else {
            continue;
        };
        let width = w - 2 * left;
        match spans.last_mut() {
            Some(last)
                if last.x == left as i16
                    && last.width == width
                    && last.y as i32 + last.height as i32 == i32::from(y) =>
            {
                last.height += 1;
            }
            _ => spans.push(Rectangle {
                x: left as i16,
                y: y as i16,
                width,
                height: 1,
            }),
        }
    }
    spans
}

fn render(
    w: usize,
    h: usize,
    dot: [u8; 3],
    text: &str,
    font: &FontRef<'_>,
    argb: bool,
) -> Vec<u32> {
    let (wf, hf) = (w as f32, h as f32);
    let d = (h / 2) as f32;
    let pad = ((h as f32) - d) / 2.0;
    let text_left = pad * 2.0 + d;
    let text_right = wf - pad;
    let glyphs = text_coverage(w, h, text, font, text_left, text_right);
    let (dot_cx, dot_cy, dot_r) = (pad + d / 2.0, pad + d / 2.0, d / 2.0);
    let opacity = f32::from(ALPHA) / 255.0;

    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let pill = capsule_coverage(px, py, wf, hf);
            let mut color = BACKGROUND.map(f32::from);
            color = mix(color, dot, circle_coverage(px, py, dot_cx, dot_cy, dot_r));
            color = mix(color, TEXT, glyphs[y * w + x]);
            out.push(pack(color, if argb { pill * opacity } else { 1.0 }));
        }
    }
    out
}

fn mix(base: [f32; 3], over: [u8; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| base[i] + (f32::from(over[i]) - base[i]) * t)
}

fn pack(color: [f32; 3], alpha: f32) -> u32 {
    let a = alpha.clamp(0.0, 1.0);
    let [r, g, b] = color.map(|c| (c * a).round().clamp(0.0, 255.0) as u32);
    let a = (a * 255.0).round() as u32;
    a << 24 | r << 16 | g << 8 | b
}

fn text_coverage(
    w: usize,
    h: usize,
    text: &str,
    font: &FontRef<'_>,
    left: f32,
    right: f32,
) -> Vec<f32> {
    let mut cov = vec![0.0f32; w * h];
    let em = h as f32 * 15.0 / 28.0;
    let upem = font.units_per_em().unwrap_or(1000.0);
    let scale = PxScale::from(em * font.height_unscaled() / upem);
    let sf = font.as_scaled(scale);
    let text = fit(text, |s| text_width(&sf, s), right - left);
    let baseline = (h as f32 + sf.ascent() + sf.descent()) / 2.0;

    let mut x = left;
    let mut prev: Option<GlyphId> = None;
    for c in text.chars() {
        let id = sf.glyph_id(c);
        if let Some(p) = prev {
            x += sf.kern(p, id);
        }
        let glyph = id.with_scale_and_position(scale, point(x, baseline));
        x += sf.h_advance(id);
        prev = Some(id);
        let Some(outline) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, c| {
            let px = bounds.min.x as i64 + i64::from(gx);
            let py = bounds.min.y as i64 + i64::from(gy);
            for (dx, weight) in [(0, 1.0), (1, 0.5)] {
                let px = px + dx;
                if px < 0 || py < 0 || px as f32 >= right || px as usize >= w || py as usize >= h {
                    continue;
                }
                let i = py as usize * w + px as usize;
                cov[i] = (cov[i] + c * weight).min(1.0);
            }
        });
    }
    cov
}

fn text_width<F: Font>(sf: &ab_glyph::PxScaleFont<F>, text: &str) -> f32 {
    let mut prev: Option<GlyphId> = None;
    text.chars()
        .map(|c| {
            let id = sf.glyph_id(c);
            let kern = prev.map_or(0.0, |p| sf.kern(p, id));
            prev = Some(id);
            kern + sf.h_advance(id)
        })
        .sum()
}

fn fit(text: &str, width: impl Fn(&str) -> f32, max: f32) -> String {
    if width(text) <= max {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate = format!("{}…", chars.iter().collect::<String>().trim_end());
        if width(&candidate) <= max {
            return candidate;
        }
    }
    String::new()
}
