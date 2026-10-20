//! RWID window decorations (server side): one rounded "card" around every native
//! window — a title bar plus a thin frame — drawn behind the client, so the
//! client's square corners sit inside the rounded edge. Three user-selectable
//! styles (`~/.config/rwid-os/window-style`, written by RWID Settings through the
//! shell): `mac` (lights left, title centred), `windows` (min/max/close right),
//! `capsule` (RWID: controls in one capsule, gold line on the active window).

use std::{
    cell::RefCell,
    path::PathBuf,
    time::{Instant, SystemTime},
};

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, VariableFont};
use tiny_skia::{
    Color, FillRule, LineCap, Mask, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

/// Title bar height, frame width and corner radius (logical px).
pub const HEADER: i32 = 34;
pub const BORDER: i32 = 3;
pub const RADIUS: f32 = 10.0;

/// Size the card adds around a client (width, height).
pub const fn card_extra() -> (i32, i32) {
    (2 * BORDER, HEADER + BORDER)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Style {
    #[default]
    Mac,
    Windows,
    Capsule,
}

impl Style {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "mac" => Some(Self::Mac),
            "windows" => Some(Self::Windows),
            "capsule" => Some(Self::Capsule),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Close,
    Maximize,
    Minimize,
}

fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("rwid-os").join("window-style"))
}

struct StyleCache {
    style: Style,
    checked: Option<Instant>,
    mtime: Option<SystemTime>,
}

thread_local! {
    static STYLE: RefCell<StyleCache> = const {
        RefCell::new(StyleCache { style: Style::Mac, checked: None, mtime: None })
    };
}

/// The user's style; the file is looked at no more than once a second.
pub fn style() -> Style {
    STYLE.with(|c| {
        let mut c = c.borrow_mut();
        if c.checked.is_some_and(|t| t.elapsed().as_millis() < 1000) {
            return c.style;
        }
        c.checked = Some(Instant::now());
        let Some(path) = config_path() else {
            return c.style;
        };
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if mtime != c.mtime {
            c.mtime = mtime;
            c.style = std::fs::read_to_string(&path)
                .ok()
                .and_then(|s| Style::parse(&s))
                .unwrap_or_default();
        }
        c.style
    })
}

// ── Geometry of the controls (card coordinates, logical px) ──────────────
const MAC_D: f32 = 12.0;
const MAC_GAP: f32 = 8.0;
const WIN_W: f32 = 46.0;
const CAP_W: f32 = 26.0;
const CAP_H: f32 = 22.0;
const CAP_GAP: f32 = 2.0;
const CAP_PAD: f32 = 3.0;
const CAP_RIGHT: f32 = 10.0;

/// Buttons in drawing order with their hit boxes `(x, y, w, h)`.
fn buttons(style: Style, card_w: f32) -> [(Button, Rect); 3] {
    let h = HEADER as f32;
    let r = |x: f32, y: f32, w: f32, hh: f32| Rect::from_xywh(x, y, w, hh).unwrap();
    match style {
        Style::Mac => {
            let y = (h - MAC_D) / 2.0;
            let x0 = BORDER as f32 + 12.0;
            [
                (Button::Close, r(x0, y, MAC_D, MAC_D)),
                (Button::Minimize, r(x0 + MAC_D + MAC_GAP, y, MAC_D, MAC_D)),
                (
                    Button::Maximize,
                    r(x0 + 2.0 * (MAC_D + MAC_GAP), y, MAC_D, MAC_D),
                ),
            ]
        }
        Style::Windows => [
            (Button::Minimize, r(card_w - 3.0 * WIN_W, 0.0, WIN_W, h)),
            (Button::Maximize, r(card_w - 2.0 * WIN_W, 0.0, WIN_W, h)),
            (Button::Close, r(card_w - WIN_W, 0.0, WIN_W, h)),
        ],
        Style::Capsule => {
            let y = (h - CAP_H) / 2.0;
            let x3 = card_w - CAP_RIGHT - CAP_PAD - CAP_W;
            [
                (
                    Button::Minimize,
                    r(x3 - 2.0 * (CAP_W + CAP_GAP), y, CAP_W, CAP_H),
                ),
                (Button::Maximize, r(x3 - (CAP_W + CAP_GAP), y, CAP_W, CAP_H)),
                (Button::Close, r(x3, y, CAP_W, CAP_H)),
            ]
        }
    }
}

/// The button under a point of the card (header only).
pub fn button_at(style: Style, card_w: i32, x: f64, y: f64) -> Option<Button> {
    if y < 0.0 || y >= HEADER as f64 {
        return None;
    }
    let slack = if style == Style::Mac { 4.0 } else { 0.0 };
    buttons(style, card_w as f32)
        .into_iter()
        .find_map(|(b, r)| {
            let hit = x >= (r.left() - slack) as f64
                && x < (r.right() + slack) as f64
                && y >= (r.top() - slack) as f64
                && y < (r.bottom() + slack) as f64;
            hit.then_some(b)
        })
}

// ── Painting ─────────────────────────────────────────────────────────────
fn rgb(hex: u32, a: f32) -> Color {
    let c = |s: u32| ((hex >> s) & 0xff) as f32 / 255.0;
    Color::from_rgba(c(16), c(8), c(0), a).unwrap()
}
fn paint(color: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(color);
    p.anti_alias = true;
    p
}

const BG_ACTIVE: u32 = 0x1a2338;
const BG_INACTIVE: u32 = 0x151c2d;
const GOLD: u32 = 0xf2b53a;

pub fn frame_color(active: bool) -> [f32; 4] {
    let hex = if active { BG_ACTIVE } else { BG_INACTIVE };
    let c = |s: u32| ((hex >> s) & 0xff) as f32 / 255.0;
    [c(16), c(8), c(0), 1.0]
}

/// Rounded rectangle with independent top/bottom radii.
fn rounded(x: f32, y: f32, w: f32, h: f32, top: f32, bottom: f32) -> Option<Path> {
    const K: f32 = 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + top, y);
    pb.line_to(x + w - top, y);
    pb.cubic_to(
        x + w - top + top * K,
        y,
        x + w,
        y + top - top * K,
        x + w,
        y + top,
    );
    pb.line_to(x + w, y + h - bottom);
    pb.cubic_to(
        x + w,
        y + h - bottom + bottom * K,
        x + w - bottom + bottom * K,
        y + h,
        x + w - bottom,
        y + h,
    );
    pb.line_to(x + bottom, y + h);
    pb.cubic_to(
        x + bottom - bottom * K,
        y + h,
        x,
        y + h - bottom + bottom * K,
        x,
        y + h - bottom,
    );
    pb.line_to(x, y + top);
    pb.cubic_to(x, y + top - top * K, x + top - top * K, y, x + top, y);
    pb.close();
    pb.finish()
}

fn glyph_line(pm: &mut Pixmap, t: Transform, pts: &[(f32, f32)], color: Color, width: f32) {
    let mut pb = PathBuilder::new();
    for (i, (x, y)) in pts.iter().enumerate() {
        if i == 0 {
            pb.move_to(*x, *y);
        } else {
            pb.line_to(*x, *y);
        }
    }
    if let Some(path) = pb.finish() {
        let stroke = Stroke {
            width,
            line_cap: LineCap::Round,
            ..Stroke::default()
        };
        pm.stroke_path(&path, &paint(color), &stroke, t, None);
    }
}

/// Minimize / maximize / close glyph centred in `r`.
fn glyph(pm: &mut Pixmap, t: Transform, b: Button, r: Rect, color: Color, size: f32) {
    let (cx, cy, s) = (
        r.x() + r.width() / 2.0,
        r.y() + r.height() / 2.0,
        size / 2.0,
    );
    match b {
        Button::Minimize => glyph_line(pm, t, &[(cx - s, cy), (cx + s, cy)], color, 1.2),
        Button::Maximize => {
            if let Some(p) = rounded(cx - s, cy - s, 2.0 * s, 2.0 * s, 1.5, 1.5) {
                let stroke = Stroke {
                    width: 1.2,
                    ..Stroke::default()
                };
                pm.stroke_path(&p, &paint(color), &stroke, t, None);
            }
        }
        Button::Close => {
            glyph_line(pm, t, &[(cx - s, cy - s), (cx + s, cy + s)], color, 1.2);
            glyph_line(pm, t, &[(cx + s, cy - s), (cx - s, cy + s)], color, 1.2);
        }
    }
}

/// What the header looks like; a change re-renders it.
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderKey {
    pub style: Style,
    pub card_w: i32,
    pub scale: i32,
    pub active: bool,
    pub hover: Option<Button>,
    pub title: String,
}

/// Header pixels (premultiplied RGBA, `card_w*scale × HEADER*scale`).
pub fn paint_header(k: &HeaderKey) -> Option<Pixmap> {
    let s = k.scale.max(1) as f32;
    let (w, h) = (k.card_w as f32, HEADER as f32);
    let mut pm = Pixmap::new((w * s) as u32, (h * s) as u32)?;
    let t = Transform::from_scale(s, s);
    let shape = rounded(0.0, 0.0, w, h + RADIUS, RADIUS, 0.0)?;
    let mut clip = Mask::new(pm.width(), pm.height())?;
    clip.fill_path(&shape, FillRule::Winding, true, t);
    let bg = if k.active { BG_ACTIVE } else { BG_INACTIVE };
    pm.fill_path(&shape, &paint(rgb(bg, 1.0)), FillRule::Winding, t, None);

    let fg = if k.active {
        rgb(0xdfe5f1, 1.0)
    } else {
        rgb(0x8792a8, 1.0)
    };
    let btns = buttons(k.style, w);
    match k.style {
        Style::Mac => {
            let colors = [0xff5f57, 0xfebc2e, 0x28c840];
            for (i, (b, r)) in btns.iter().enumerate() {
                let c = if k.active || k.hover.is_some() {
                    rgb(colors[i], 1.0)
                } else {
                    rgb(0x3a4357, 1.0)
                };
                let circle =
                    PathBuilder::from_circle(r.x() + MAC_D / 2.0, r.y() + MAC_D / 2.0, MAC_D / 2.0);
                if let Some(c0) = circle {
                    pm.fill_path(&c0, &paint(c), FillRule::Winding, t, None);
                }
                if k.hover.is_some() {
                    glyph(&mut pm, t, *b, *r, rgb(0x4a1a12, 0.85), 5.0);
                }
            }
        }
        Style::Windows => {
            for (b, r) in btns {
                if k.hover == Some(b) {
                    let c = if b == Button::Close {
                        rgb(0xc42b1c, 1.0)
                    } else {
                        rgb(0xffffff, 0.08)
                    };
                    pm.fill_path(
                        &PathBuilder::from_rect(r),
                        &paint(c),
                        FillRule::Winding,
                        t,
                        Some(&clip),
                    );
                }
                let c = if k.hover == Some(b) && b == Button::Close {
                    rgb(0xffffff, 1.0)
                } else {
                    fg
                };
                glyph(&mut pm, t, b, r, c, 10.0);
            }
        }
        Style::Capsule => {
            if k.active {
                let line = Rect::from_xywh(0.0, 0.0, w, 2.0)?;
                pm.fill_path(
                    &PathBuilder::from_rect(line),
                    &paint(rgb(GOLD, 1.0)),
                    FillRule::Winding,
                    t,
                    Some(&clip),
                );
            }
            let first = btns[0].1;
            let last = btns[2].1;
            let pill = rounded(
                first.x() - CAP_PAD,
                first.y() - CAP_PAD / 1.5,
                last.right() - first.x() + 2.0 * CAP_PAD,
                CAP_H + CAP_PAD * 2.0 / 1.5,
                (CAP_H + CAP_PAD) / 2.0,
                (CAP_H + CAP_PAD) / 2.0,
            )?;
            let alpha = if k.active { 0.08 } else { 0.04 };
            pm.fill_path(
                &pill,
                &paint(rgb(0xe8edf6, alpha)),
                FillRule::Winding,
                t,
                None,
            );
            for (b, r) in btns {
                let hover = k.hover == Some(b);
                if hover {
                    let c = if b == Button::Close {
                        rgb(GOLD, 1.0)
                    } else {
                        rgb(0xffffff, 0.12)
                    };
                    if let Some(p) = rounded(
                        r.x(),
                        r.y(),
                        r.width(),
                        r.height(),
                        CAP_H / 2.0,
                        CAP_H / 2.0,
                    ) {
                        pm.fill_path(&p, &paint(c), FillRule::Winding, t, None);
                    }
                }
                let c = if hover && b == Button::Close {
                    rgb(0x0b1220, 1.0)
                } else if k.active {
                    rgb(0xc9d2e3, 1.0)
                } else {
                    rgb(0x6b7894, 1.0)
                };
                glyph(&mut pm, t, b, r, c, 8.0);
            }
        }
    }

    // Title: centred (mac) or after the left padding; never over the controls.
    let (left, right) = match k.style {
        Style::Mac => (btns[2].1.right() + 12.0, w - (btns[2].1.right() + 12.0)),
        Style::Windows => (BORDER as f32 + 14.0, btns[0].1.x() - 8.0),
        Style::Capsule => (BORDER as f32 + 14.0, btns[0].1.x() - CAP_PAD - 10.0),
    };
    draw_title(&mut pm, &k.title, left, right, k.style == Style::Mac, s, fg);
    Some(pm)
}

/// Bottom strip of the card (`card_w × RADIUS`) with rounded bottom corners.
pub fn paint_bottom(card_w: i32, scale: i32, active: bool) -> Option<Pixmap> {
    let s = scale.max(1) as f32;
    let (w, h) = (card_w as f32, RADIUS);
    let mut pm = Pixmap::new((w * s) as u32, (h * s) as u32)?;
    let shape = rounded(0.0, -RADIUS, w, h + RADIUS, 0.0, RADIUS)?;
    let bg = if active { BG_ACTIVE } else { BG_INACTIVE };
    pm.fill_path(
        &shape,
        &paint(rgb(bg, 1.0)),
        FillRule::Winding,
        Transform::from_scale(s, s),
        None,
    );
    Some(pm)
}

// ── Title text ───────────────────────────────────────────────────────────
thread_local! {
    static FONT: Option<FontVec> = load_font();
}

fn load_font() -> Option<FontVec> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/sora/Sora-wght.ttf",
        "/usr/share/fonts/google-noto-vf/NotoSans[wght].ttf",
        "/usr/share/fonts/adwaita-sans-fonts/AdwaitaSans-Regular.ttf",
        "/usr/share/fonts/abattis-cantarell-vf-fonts/Cantarell-VF.otf",
    ];
    CANDIDATES.iter().find_map(|p| {
        let mut f = FontVec::try_from_vec(std::fs::read(p).ok()?).ok()?;
        f.set_variation(b"wght", 600.0);
        Some(f)
    })
}

fn draw_title(
    pm: &mut Pixmap,
    title: &str,
    left: f32,
    right: f32,
    centred: bool,
    s: f32,
    color: Color,
) {
    if title.is_empty() || right <= left {
        return;
    }
    FONT.with(|font| {
        let Some(font) = font.as_ref() else {
            return;
        };
        let sf = font.as_scaled(PxScale::from(13.0 * s));
        // Lay out, then cut with "…" to the room available.
        let max_w = (right - left) * s;
        let ell = sf.h_advance(font.glyph_id('…'));
        let mut chars: Vec<char> = title.chars().collect();
        let width = |cs: &[char]| {
            cs.iter()
                .map(|c| sf.h_advance(font.glyph_id(*c)))
                .sum::<f32>()
        };
        let mut cut = false;
        while !chars.is_empty() && width(&chars) + if cut { ell } else { 0.0 } > max_w {
            chars.pop();
            cut = true;
        }
        if cut {
            chars.push('…');
        }
        let total = width(&chars);
        let mut x = if centred {
            ((pm.width() as f32 - total) / 2.0).max(left * s)
        } else {
            left * s
        };
        let baseline = (HEADER as f32 * s + sf.ascent() + sf.descent()) / 2.0;
        let (w, h) = (pm.width() as i32, pm.height() as i32);
        let (cr, cg, cb) = (color.red(), color.green(), color.blue());
        let pixels = pm.pixels_mut();
        for c in chars {
            let g = font
                .glyph_id(c)
                .with_scale_and_position(sf.scale(), ab_glyph::point(x, baseline));
            x += sf.h_advance(font.glyph_id(c));
            let Some(outline) = font.outline_glyph(g) else {
                continue;
            };
            let bb = outline.px_bounds();
            outline.draw(|gx, gy, cov| {
                let (px, py) = (bb.min.x as i32 + gx as i32, bb.min.y as i32 + gy as i32);
                if px < 0 || py < 0 || px >= w || py >= h {
                    return;
                }
                let i = (py * w + px) as usize;
                let d = pixels[i];
                let a = cov.clamp(0.0, 1.0) * color.alpha();
                let mix =
                    |src: f32, dst: u8| ((src * 255.0 * a) + dst as f32 * (1.0 - a)).round() as u8;
                let na = ((a * 255.0) + d.alpha() as f32 * (1.0 - a)).round() as u8;
                if let Some(p) = tiny_skia::PremultipliedColorU8::from_rgba(
                    mix(cr, d.red()).min(na),
                    mix(cg, d.green()).min(na),
                    mix(cb, d.blue()).min(na),
                    na,
                ) {
                    pixels[i] = p;
                }
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn styles_parse_strictly() {
        assert_eq!(Style::parse("mac\n"), Some(Style::Mac));
        assert_eq!(Style::parse("windows"), Some(Style::Windows));
        assert_eq!(Style::parse("capsule"), Some(Style::Capsule));
        assert_eq!(Style::parse("Mac"), None);
        assert_eq!(Style::parse(""), None);
    }

    #[test]
    fn hit_boxes_follow_the_style() {
        let w = 600;
        // mac: close is the first light on the left
        assert_eq!(button_at(Style::Mac, w, 21.0, 17.0), Some(Button::Close));
        assert_eq!(button_at(Style::Mac, w, 300.0, 17.0), None);
        // windows: 46 px each from the right: close, maximize, minimize
        assert_eq!(
            button_at(Style::Windows, w, 590.0, 10.0),
            Some(Button::Close)
        );
        assert_eq!(
            button_at(Style::Windows, w, 530.0, 10.0),
            Some(Button::Maximize)
        );
        assert_eq!(
            button_at(Style::Windows, w, 480.0, 10.0),
            Some(Button::Minimize)
        );
        // capsule: close is the right end of the capsule
        assert_eq!(
            button_at(Style::Capsule, w, 570.0, 17.0),
            Some(Button::Close)
        );
        // below the header: never a button
        assert_eq!(button_at(Style::Windows, w, 590.0, 40.0), None);
    }

    /// `DECO_DUMP=dir cargo test -p rwid-compositor deco` writes the headers as PNGs.
    #[test]
    fn dump_for_review() {
        let Some(dir) = std::env::var_os("DECO_DUMP") else {
            return;
        };
        for (name, style) in [
            ("a-mac", Style::Mac),
            ("b-windows", Style::Windows),
            ("d-capsule", Style::Capsule),
        ] {
            for (state, active, hover) in [
                ("active", true, None),
                ("hover", true, Some(Button::Close)),
                ("inactive", false, None),
            ] {
                let k = HeaderKey {
                    style,
                    card_w: 640,
                    scale: 2,
                    active,
                    hover,
                    title: "fedora@rwid: ~/tugas — foot".into(),
                };
                paint_header(&k)
                    .unwrap()
                    .save_png(std::path::Path::new(&dir).join(format!("{name}-{state}.png")))
                    .unwrap();
            }
        }
    }

    #[test]
    fn every_style_paints() {
        for style in [Style::Mac, Style::Windows, Style::Capsule] {
            for hover in [None, Some(Button::Close)] {
                let k = HeaderKey {
                    style,
                    card_w: 480,
                    scale: 2,
                    active: true,
                    hover,
                    title: "Terminal — foot".into(),
                };
                let pm = paint_header(&k).expect("header");
                assert_eq!((pm.width(), pm.height()), (960, HEADER as u32 * 2));
                // top-left corner is transparent (rounded)
                assert_eq!(pm.pixel(0, 0).unwrap().alpha(), 0);
            }
        }
        let b = paint_bottom(480, 1, false).expect("bottom");
        assert_eq!(b.pixel(0, RADIUS as u32 - 1).unwrap().alpha(), 0);
    }
}
