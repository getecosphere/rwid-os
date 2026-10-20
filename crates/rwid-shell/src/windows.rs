//! The surfaces of the RWID desktop (ADR-0022), all in ONE web process:
//! - desktop: the ecOS page (`?surface=desktop`) on the background layer;
//! - chrome: the same page (`?surface=chrome`) on the top layer, transparent,
//!   taking clicks only where ecOS reports it shows something (dock, start menu);
//! - two empty top-layer strips that reserve the top bar and the dock (exclusive
//!   zones), so a maximised window never hides them;
//! - one xdg_toplevel per RWID app, decorated by the compositor like any native
//!   window.
//!
//! Every view after the desktop is a *related view* of it: same web process,
//! network session and cookie (~25 MB per app window instead of ~82 MB).
//!
//! Memory: an app window the compositor minimises is *suspended* (xdg-shell);
//! after `RWID_SHELL_DISCARD_SECS` (default 5 min) — at once when RAM is short —
//! its page is discarded (the view is destroyed, the URL kept) and reloaded when
//! the window comes back, like a phone does.

use std::{cell::RefCell, collections::BTreeMap, rc::Rc, time::Duration};

use gtk4::{cairo, gdk, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use webkit6::{LoadEvent, NavigationPolicyDecision, PolicyDecisionType, WebView, prelude::*};

use crate::native::{self, Request};

pub const DESKTOP: &str = "desktop";
pub const CHROME: &str = "chrome";

struct AppWindow {
    window: gtk4::ApplicationWindow,
    title: String,
    /// The page; `None` while discarded.
    view: Option<WebView>,
    /// Where the page was when it was discarded.
    url: String,
    discard: Option<glib::SourceId>,
}

/// Seconds a minimised app window keeps its page.
fn discard_after() -> Duration {
    Duration::from_secs(
        std::env::var("RWID_SHELL_DISCARD_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(300),
    )
}

/// Less than 15 % of RAM available: discard minimised pages at once.
fn memory_low() -> bool {
    let info = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let field = |k: &str| -> Option<u64> {
        info.lines()
            .find(|l| l.starts_with(k))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    match (field("MemAvailable:"), field("MemTotal:")) {
        (Some(avail), Some(total)) if total > 0 => avail * 100 / total < 15,
        _ => false,
    }
}

pub struct Desk {
    app: gtk4::Application,
    origin: String,
    desktop: WebView,
    chrome: WebView,
    chrome_win: gtk4::ApplicationWindow,
    reserve_top: gtk4::ApplicationWindow,
    reserve_bottom: gtk4::ApplicationWindow,
    apps: RefCell<BTreeMap<String, AppWindow>>,
    /// The dock strip (px from the bottom), reported by the chrome.
    dock_px: std::cell::Cell<i32>,
    chrome_expanded: std::cell::Cell<bool>,
}

/// Room above the dock for its hover labels while the chrome is collapsed.
const DOCK_HEADROOM: i32 = 56;

/// A related view of the desktop: same web process, network session, settings
/// (user agent) and user content (the bridge) — what the C helper
/// `webkit_web_view_new_with_related_view` shares, which the builder does not.
fn related(desktop: &WebView) -> WebView {
    let mut b = WebView::builder().related_view(desktop);
    if let Some(s) = WebViewExt::settings(desktop) {
        b = b.settings(&s);
    }
    if let Some(u) = desktop.user_content_manager() {
        b = b.user_content_manager(&u);
    }
    b.build()
}

/// Mark which surface a view is, in the private world (page code cannot set it).
fn tag(view: &WebView, who: &str) {
    let who = who.to_string();
    view.connect_load_changed(move |v, e| {
        if e == LoadEvent::Committed {
            v.evaluate_javascript(
                &format!(
                    "window.__rwidWin = {};",
                    serde_json::Value::String(who.clone())
                ),
                Some(native::WORLD),
                None,
                None::<&gtk4::gio::Cancellable>,
                |_| {},
            );
        }
    });
}

/// Keep a view on the shell origin; new top-level windows are refused.
fn pin(view: &WebView, origin: &str) {
    let origin = origin.to_string();
    view.connect_decide_policy(move |_, decision, kind| match kind {
        PolicyDecisionType::NavigationAction => {
            let uri = decision
                .downcast_ref::<NavigationPolicyDecision>()
                .and_then(|d| d.navigation_action())
                .and_then(|a| a.request())
                .and_then(|r| r.uri())
                .map(|u| u.to_string())
                .unwrap_or_default();
            if !crate::allowed(&uri, &origin) {
                tracing::info!(uri, "blocked navigation away from the shell");
                decision.ignore();
                return true;
            }
            false
        }
        PolicyDecisionType::NewWindowAction => {
            decision.ignore();
            true
        }
        _ => false,
    });
}

/// A transparent, full-width layer window (chrome, reserved strips).
fn layer_window(
    app: &gtk4::Application,
    ns: &str,
    layer: Layer,
    edges: &[Edge],
) -> gtk4::ApplicationWindow {
    let w = gtk4::ApplicationWindow::builder().application(app).build();
    w.add_css_class("rwid-transparent");
    if gtk4_layer_shell::is_supported() {
        w.init_layer_shell();
        w.set_layer(layer);
        w.set_namespace(Some(ns));
        for e in edges {
            w.set_anchor(*e, true);
        }
    }
    w
}

fn empty_input(w: &gtk4::ApplicationWindow) {
    w.connect_realize(|w| {
        if let Some(s) = w.surface() {
            s.set_input_region(Some(&cairo::Region::create()));
        }
    });
}

impl Desk {
    pub fn new(app: &gtk4::Application, desktop: WebView, origin: &str) -> Rc<Self> {
        // Transparent windows for the chrome and the strips.
        let css = gtk4::CssProvider::new();
        css.load_from_string("window.rwid-transparent { background: transparent; }");
        if let Some(d) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &d,
                &css,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        let chrome = related(&desktop);
        chrome.set_background_color(&gdk::RGBA::TRANSPARENT);
        pin(&chrome, origin);
        tag(&chrome, CHROME);
        tag(&desktop, DESKTOP);
        let chrome_win = layer_window(
            app,
            "rwid-chrome",
            Layer::Top,
            &[Edge::Top, Edge::Bottom, Edge::Left, Edge::Right],
        );
        if gtk4_layer_shell::is_supported() {
            chrome_win.set_exclusive_zone(-1);
            chrome_win.set_keyboard_mode(KeyboardMode::OnDemand);
        }
        chrome_win.set_child(Some(&chrome));
        empty_input(&chrome_win); // until ecOS reports where the dock is

        let strip = |ns: &str, edge: Edge| {
            let w = layer_window(app, ns, Layer::Top, &[edge, Edge::Left, Edge::Right]);
            w.set_default_size(1, 1);
            empty_input(&w);
            w
        };
        let reserve_top = strip("rwid-reserve", Edge::Top);
        let reserve_bottom = strip("rwid-reserve", Edge::Bottom);

        Rc::new(Self {
            app: app.clone(),
            origin: origin.to_string(),
            desktop,
            chrome,
            chrome_win,
            reserve_top,
            reserve_bottom,
            apps: RefCell::new(BTreeMap::new()),
            dock_px: std::cell::Cell::new(0),
            chrome_expanded: std::cell::Cell::new(true),
        })
    }

    /// Load the desktop and the chrome (its own light page: dock, start menu,
    /// launchpad, lock screen — no globe or feeds) and show them.
    pub fn start(&self, desktop_url: &str, chrome_url: &str) {
        self.load(desktop_url, chrome_url);
        self.reserve_top.present();
        self.reserve_bottom.present();
        self.chrome_win.present();
    }

    /// Point both surfaces at a desktop (the RWID one or ecOS Lite).
    pub fn load(&self, desktop_url: &str, chrome_url: &str) {
        self.desktop.load_uri(desktop_url);
        self.chrome.load_uri(chrome_url);
    }

    /// Full screen while a menu or modal shows; otherwise only the dock strip
    /// (a smaller surface costs less memory). Unknown dock height → full screen.
    fn size_chrome(&self) {
        if !gtk4_layer_shell::is_supported() {
            return;
        }
        let strip = self.dock_px.get();
        let full = self.chrome_expanded.get() || strip <= 0;
        self.chrome_win.set_anchor(Edge::Top, full);
        if !full {
            self.chrome_win.set_default_size(-1, strip + DOCK_HEADROOM);
        }
    }

    pub fn chrome_view(&self) -> &WebView {
        &self.chrome
    }

    /// A window request from surface `from` (`desktop`, `chrome` or an app id).
    pub fn request(self: &Rc<Self>, from: &str, req: Request) -> String {
        tracing::debug!(from, ?req, "window request");
        match req {
            Request::OpenWindow { id, title, path } => {
                if !native::valid_app_id(&id) || !native::valid_path(&path) || title.len() > 120 {
                    return r#"{"error":"bad window"}"#.into();
                }
                self.open(&id, &title, &path);
                r#"{"ok":true}"#.into()
            }
            Request::FocusWindow { id } => {
                if let Some(w) = self.apps.borrow().get(&id) {
                    w.window.present();
                }
                r#"{"ok":true}"#.into()
            }
            Request::CloseWindow => {
                let w = self.apps.borrow().get(from).map(|w| w.window.clone());
                if let Some(w) = w {
                    w.close();
                }
                r#"{"ok":true}"#.into()
            }
            Request::Windows => {
                self.push_windows();
                self.windows_json()
            }
            Request::InputRegion { rects } if from == CHROME => {
                tracing::debug!(
                    w = self.chrome_win.width(),
                    h = self.chrome_win.height(),
                    "chrome size"
                );
                if let Some(s) = self.chrome_win.surface() {
                    let region = cairo::Region::create();
                    for [x, y, w, h] in rects.into_iter().take(64) {
                        let _ = region.union_rectangle(&cairo::RectangleInt::new(
                            x,
                            y,
                            w.max(0),
                            h.max(0),
                        ));
                    }
                    s.set_input_region(Some(&region));
                }
                r#"{"ok":true}"#.into()
            }
            Request::Reserve { edge, px } => {
                let px = px.clamp(0, 400);
                match (edge.as_str(), from) {
                    ("top", DESKTOP) => self.reserve_top.set_exclusive_zone(px),
                    ("bottom", CHROME) => {
                        self.reserve_bottom.set_exclusive_zone(px);
                        self.dock_px.set(px);
                        self.size_chrome();
                    }
                    _ => return r#"{"error":"not yours"}"#.into(),
                }
                r#"{"ok":true}"#.into()
            }
            Request::ChromeSize { expanded } if from == CHROME => {
                self.chrome_expanded.set(expanded);
                self.size_chrome();
                r#"{"ok":true}"#.into()
            }
            Request::InputRegion { .. } | Request::ChromeSize { .. } => {
                r#"{"error":"not yours"}"#.into()
            }
            _ => r#"{"error":"bad request"}"#.into(),
        }
    }

    /// The page of an app window: a related view, pinned to the shell, tagged
    /// with the app id, its title following the page.
    fn app_view(&self, id: &str, window: &gtk4::ApplicationWindow, base: &str) -> WebView {
        let view = related(&self.desktop);
        pin(&view, &self.origin);
        tag(&view, id);
        let (window, base) = (window.clone(), base.to_string());
        view.connect_title_notify(move |v| {
            let t = v.title().map(|t| t.to_string()).unwrap_or_default();
            window.set_title(Some(if t.is_empty() || t == base {
                base.as_str()
            } else {
                t.as_str()
            }));
        });
        view
    }

    fn open(self: &Rc<Self>, id: &str, title: &str, path: &str) {
        tracing::debug!(id, path, "opening app window");
        if let Some(w) = self.apps.borrow().get(id) {
            w.window.present();
            return;
        }
        let window = gtk4::ApplicationWindow::builder()
            .application(&self.app)
            .title(title)
            .default_width(1100)
            .default_height(720)
            .decorated(false) // the compositor draws the RWID card (ADR-0021)
            .build();
        let view = self.app_view(id, &window, title);
        window.set_child(Some(&view));
        let desk = Rc::downgrade(self);
        let key = id.to_string();
        window.connect_close_request(move |_| {
            if let Some(d) = desk.upgrade() {
                if let Some(w) = d.apps.borrow_mut().remove(&key)
                    && let Some(t) = w.discard
                {
                    t.remove();
                }
                d.push_windows();
            }
            glib::Propagation::Proceed
        });
        // Minimised (suspended) → discard later; back → reload if discarded.
        let (desk, key) = (Rc::downgrade(self), id.to_string());
        window.connect_realize(move |w| {
            let Some(top) = w.surface().and_downcast::<gdk::Toplevel>() else {
                return;
            };
            let (desk, key) = (desk.clone(), key.clone());
            top.connect_state_notify(move |t| {
                if let Some(d) = desk.upgrade() {
                    d.suspended(&key, t.state().contains(gdk::ToplevelState::SUSPENDED));
                }
            });
        });
        view.load_uri(&format!("{}{}", self.origin, path));
        window.present();
        self.apps.borrow_mut().insert(
            id.to_string(),
            AppWindow {
                window,
                title: title.to_string(),
                view: Some(view),
                url: String::new(),
                discard: None,
            },
        );
        tracing::info!(id, "app window opened");
        self.push_windows();
    }

    fn suspended(self: &Rc<Self>, id: &str, on: bool) {
        let mut apps = self.apps.borrow_mut();
        let Some(w) = apps.get_mut(id) else {
            return;
        };
        if let Some(t) = w.discard.take() {
            t.remove();
        }
        if on {
            if w.view.is_none() {
                return;
            }
            let after = if memory_low() {
                Duration::ZERO
            } else {
                discard_after()
            };
            let (desk, key) = (Rc::downgrade(self), id.to_string());
            w.discard = Some(glib::timeout_add_local_once(after, move || {
                if let Some(d) = desk.upgrade() {
                    d.discard(&key);
                }
            }));
        } else if w.view.is_none() {
            // Back on screen: a fresh page at the URL it had.
            let view = self.app_view(id, &w.window, &w.title);
            w.window.set_child(Some(&view));
            view.load_uri(&w.url);
            w.view = Some(view);
            tracing::info!(id, "app window restored");
        }
    }

    /// Free a minimised app window's page; keep its URL to reload it.
    fn discard(&self, id: &str) {
        let mut apps = self.apps.borrow_mut();
        let Some(w) = apps.get_mut(id) else {
            return;
        };
        w.discard = None;
        if let Some(view) = w.view.take() {
            w.url = view
                .uri()
                .map(|u| u.to_string())
                .unwrap_or_else(|| format!("{}/", self.origin));
            w.window.set_child(None::<&gtk4::Widget>);
            view.try_close();
            tracing::info!(id, "minimised app window discarded");
        }
    }

    fn windows_json(&self) -> String {
        let list: Vec<_> = self
            .apps
            .borrow()
            .iter()
            .map(|(id, w)| serde_json::json!({ "id": id, "title": w.title }))
            .collect();
        serde_json::Value::Array(list).to_string()
    }

    /// Tell the chrome (dock chips) which app windows are open.
    fn push_windows(&self) {
        let js = format!(
            "document.documentElement.setAttribute('data-rwid-windows', {}); document.dispatchEvent(new Event('rwid:windows'));",
            serde_json::Value::String(self.windows_json())
        );
        self.chrome.evaluate_javascript(
            &js,
            Some(native::WORLD),
            None,
            None::<&gtk4::gio::Cancellable>,
            |_| {},
        );
    }
}
