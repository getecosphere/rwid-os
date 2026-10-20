//! Server-side decorations: state, input and render elements of the RWID window
//! card (drawing and styles in `deco`). The card is the title bar (`deco::HEADER`)
//! on top and a `deco::BORDER` frame on the other sides; the client sits at
//! `(BORDER, HEADER)` inside it.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            ImportMem, Renderer,
            element::{
                Kind,
                memory::{MemoryRenderBuffer, MemoryRenderBufferRenderElement},
                solid::{SolidColorBuffer, SolidColorRenderElement},
            },
        },
    },
    desktop::WindowSurface,
    input::Seat,
    utils::{Logical, Physical, Point, Scale, Serial, Transform},
    wayland::shell::xdg::XdgShellHandler,
};

use std::cell::{RefCell, RefMut};

use crate::{AnvilState, state::Backend};

use super::{
    WindowElement,
    deco::{self, BORDER, Button, HEADER, HeaderKey, RADIUS},
};

pub struct WindowState {
    pub is_ssd: bool,
    pub header_bar: HeaderBar,
}

#[derive(Debug, Default)]
pub struct HeaderBar {
    pub pointer_loc: Option<Point<f64, Logical>>,
    /// Card width (client width + 2 × BORDER).
    pub width: u32,
    header: Option<(HeaderKey, MemoryRenderBuffer)>,
    bottom: Option<((i32, i32, bool), MemoryRenderBuffer)>,
    left: SolidColorBuffer,
    right: SolidColorBuffer,
}

/// The decoration parts of one window, in card coordinates.
pub enum DecoElement<R: Renderer> {
    Mem(Box<MemoryRenderBufferRenderElement<R>>),
    Solid(SolidColorRenderElement),
}

fn pixmap_buffer(pm: &tiny_skia::Pixmap, scale: i32) -> MemoryRenderBuffer {
    // tiny-skia: premultiplied RGBA bytes = DRM ABGR8888 (little endian).
    MemoryRenderBuffer::from_slice(
        pm.data(),
        Fourcc::Abgr8888,
        (pm.width() as i32, pm.height() as i32),
        scale,
        Transform::Normal,
        None,
    )
}

impl HeaderBar {
    pub fn pointer_enter(&mut self, loc: Point<f64, Logical>) {
        self.pointer_loc = Some(loc);
    }

    pub fn pointer_leave(&mut self) {
        self.pointer_loc = None;
    }

    fn button(&self) -> Option<Button> {
        let loc = self.pointer_loc?;
        deco::button_at(deco::style(), self.width as i32, loc.x, loc.y)
    }

    fn press<BackendData: Backend>(
        &mut self,
        state: &mut AnvilState<BackendData>,
        window: &WindowElement,
        button: Button,
    ) {
        match button {
            Button::Close => match window.0.underlying_surface() {
                WindowSurface::Wayland(w) => w.send_close(),
                #[cfg(feature = "xwayland")]
                WindowSurface::X11(w) => {
                    let _ = w.close();
                }
            },
            Button::Maximize => match window.0.underlying_surface() {
                WindowSurface::Wayland(w) => {
                    if w.with_pending_state(|s| {
                        s.states
                            .contains(smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State::Maximized)
                    }) {
                        state.unmaximize_request(w.clone());
                    } else {
                        state.maximize_request(w.clone());
                    }
                }
                #[cfg(feature = "xwayland")]
                WindowSurface::X11(w) => {
                    let surface = w.clone();
                    state
                        .handle
                        .insert_idle(move |data| data.maximize_request_x11(&surface));
                }
            },
            Button::Minimize => {
                let window = window.clone();
                state.handle.insert_idle(move |data| data.minimize(&window));
            }
        }
    }

    fn start_move<BackendData: Backend>(
        &self,
        seat: &Seat<AnvilState<BackendData>>,
        state: &mut AnvilState<BackendData>,
        window: &WindowElement,
        serial: Serial,
    ) {
        // Only the title bar moves the window; the thin frame does nothing.
        if !self.pointer_loc.is_some_and(|l| l.y < HEADER as f64) {
            return;
        }
        match window.0.underlying_surface() {
            WindowSurface::Wayland(w) => {
                let seat = seat.clone();
                let toplevel = w.clone();
                state
                    .handle
                    .insert_idle(move |data| data.move_request_xdg(&toplevel, &seat, serial));
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(w) => {
                let window = w.clone();
                state
                    .handle
                    .insert_idle(move |data| data.move_request_x11(&window));
            }
        };
    }

    pub fn clicked<BackendData: Backend>(
        &mut self,
        seat: &Seat<AnvilState<BackendData>>,
        state: &mut AnvilState<BackendData>,
        window: &WindowElement,
        serial: Serial,
    ) {
        match self.button() {
            Some(b) => self.press(state, window, b),
            None => self.start_move(seat, state, window, serial),
        }
    }

    pub fn touch_down<BackendData: Backend>(
        &mut self,
        seat: &Seat<AnvilState<BackendData>>,
        state: &mut AnvilState<BackendData>,
        window: &WindowElement,
        serial: Serial,
    ) {
        if self.button().is_none() {
            self.start_move(seat, state, window, serial);
        }
    }

    pub fn touch_up<BackendData: Backend>(
        &mut self,
        _seat: &Seat<AnvilState<BackendData>>,
        state: &mut AnvilState<BackendData>,
        window: &WindowElement,
    ) {
        if let Some(b) = self.button() {
            self.press(state, window, b);
        }
    }

    /// Render elements of the card around a client of `client` size at
    /// `location` (the card's top-left, physical).
    pub fn elements<R>(
        &mut self,
        renderer: &mut R,
        location: Point<i32, Physical>,
        client: (i32, i32),
        scale: Scale<f64>,
        alpha: f32,
        (active, title): (bool, String),
    ) -> Vec<DecoElement<R>>
    where
        R: Renderer + ImportMem,
        R::TextureId: Send + Clone + 'static,
    {
        let card_w = client.0 + 2 * BORDER;
        let card_h = HEADER + client.1 + BORDER;
        self.width = card_w as u32;
        let px = scale.x.max(1.0).ceil() as i32;
        let style = deco::style();
        let key = HeaderKey {
            style,
            card_w,
            scale: px,
            active,
            hover: self
                .pointer_loc
                .and_then(|l| deco::button_at(style, card_w, l.x, l.y)),
            title,
        };
        if self.header.as_ref().is_none_or(|(k, _)| *k != key)
            && let Some(pm) = deco::paint_header(&key)
        {
            self.header = Some((key, pixmap_buffer(&pm, px)));
        }
        let bkey = (card_w, px, active);
        if self.bottom.as_ref().is_none_or(|(k, _)| *k != bkey)
            && let Some(pm) = deco::paint_bottom(card_w, px, active)
        {
            self.bottom = Some((bkey, pixmap_buffer(&pm, px)));
        }
        let color = deco::frame_color(active);
        let side_h = (card_h - HEADER - RADIUS as i32).max(0);
        self.left.update((BORDER, side_h), color);
        self.right.update((BORDER, side_h), color);

        let at = |x: i32, y: i32| {
            location + Point::<i32, Logical>::from((x, y)).to_physical_precise_round(scale)
        };
        let mut out = Vec::with_capacity(4);
        for (buf, pos) in [
            (self.header.as_ref().map(|(_, b)| b), at(0, 0)),
            (
                self.bottom.as_ref().map(|(_, b)| b),
                at(0, card_h - RADIUS as i32),
            ),
        ] {
            if let Some(buf) = buf
                && let Ok(e) = MemoryRenderBufferRenderElement::from_buffer(
                    renderer,
                    pos.to_f64(),
                    buf,
                    Some(alpha),
                    None,
                    None,
                    Kind::Unspecified,
                )
            {
                out.push(DecoElement::Mem(Box::new(e)));
            }
        }
        for (buf, pos) in [
            (&self.left, at(0, HEADER)),
            (&self.right, at(card_w - BORDER, HEADER)),
        ] {
            out.push(DecoElement::Solid(SolidColorRenderElement::from_buffer(
                buf,
                pos,
                scale,
                alpha,
                Kind::Unspecified,
            )));
        }
        out
    }
}

impl WindowElement {
    pub fn decoration_state(&self) -> RefMut<'_, WindowState> {
        self.user_data().insert_if_missing(|| {
            RefCell::new(WindowState {
                is_ssd: false,
                header_bar: HeaderBar::default(),
            })
        });

        self.user_data()
            .get::<RefCell<WindowState>>()
            .unwrap()
            .borrow_mut()
    }

    pub fn set_ssd(&self, ssd: bool) {
        self.decoration_state().is_ssd = ssd;
    }
}
