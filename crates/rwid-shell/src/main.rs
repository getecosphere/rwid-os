//! rwid-shell — the RWID/OS desktop (blueprint §6.4, ADR-0019, ADR-0022).
//!
//! The ecOS desktop on a `wlr-layer-shell` background surface, its chrome (dock,
//! start menu) on the top layer, and every RWID app as a native window — all in
//! one shared web process (`windows`). It shows the device estate's
//! gateway (`RWID_SHELL_URL`, default `http://127.0.0.1:4610/webos`) — the same ecOS
//! desktop as remoteworker.id. When that desktop cannot load (boot, restart, a
//! failing frontend) it falls back to ecOS Lite (`lite`) and returns as soon as the
//! gateway answers again. Top-level navigation is
//! pinned to the shell's own origin; the session cookie (HttpOnly) persists in
//! the user's data dir. The session supervisor restarts the process if it exits.

mod lite;
mod native;
mod windows;

use std::{cell::Cell, os::unix::net::UnixDatagram, path::PathBuf, rc::Rc, time::Duration};

use gtk4::{gio, glib, prelude::*};
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use webkit6::{
    LoadEvent, NavigationPolicyDecision, PolicyDecisionType, UserContentInjectedFrames, UserScript,
    UserScriptInjectionTime, WebView, javascriptcore, prelude::*,
};

const APP_ID: &str = "id.remoteworker.shell";
const RETRY: Duration = Duration::from_secs(5);

fn env(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// `scheme://host:port` of a URL (for the same-origin navigation pin).
fn origin(url: &str) -> String {
    let (scheme, rest) = url.split_once("://").unwrap_or(("", url));
    format!("{scheme}://{}", rest.split('/').next().unwrap_or(""))
}

/// May the main frame navigate to `uri`? Only the shell's origin and ecOS Lite;
/// links elsewhere open inside ecOS (Capt), not over the desktop.
fn allowed(uri: &str, shell_origin: &str) -> bool {
    uri == shell_origin
        || uri.starts_with(&format!("{shell_origin}/"))
        || uri.starts_with(lite::PREFIX)
        || uri == "about:blank"
}

fn data_dir(kind: &str) -> PathBuf {
    let base = std::env::var_os(if kind == "cache" {
        "XDG_CACHE_HOME"
    } else {
        "XDG_DATA_HOME"
    })
    .map(PathBuf::from)
    .unwrap_or_else(|| {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        if kind == "cache" {
            home.join(".cache")
        } else {
            home.join(".local/share")
        }
    });
    base.join("rwid-shell")
}

/// Memory limit for the shared web process: WebKit drops caches before the
/// device has to swap (35 % of RAM, 512 MB – 2 GB).
fn memory_limit_mb() -> u32 {
    let total_kb: u64 = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|m| {
            m.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(4 * 1024 * 1024);
    ((total_kb / 1024) * 35 / 100).clamp(512, 2048) as u32
}

fn build(app: &gtk4::Application) {
    let url = env("RWID_SHELL_URL", "http://127.0.0.1:4610/webos");
    let shell_origin = origin(&url);
    let desktop_url = format!(
        "{url}{}surface=desktop",
        if url.contains('?') { '&' } else { '?' }
    );

    // The chrome has its own light page (ADR-0022); RWID_SHELL_CHROME_URL overrides it.
    let chrome_url = env(
        "RWID_SHELL_CHROME_URL",
        &format!("{shell_origin}/os-chrome/?surface=chrome"),
    );

    let session = webkit6::NetworkSession::new(
        Some(&data_dir("data").to_string_lossy()),
        Some(&data_dir("cache").to_string_lossy()),
    );
    if let Some(cookies) = session.cookie_manager() {
        cookies.set_persistent_storage(
            &data_dir("data").join("cookies.sqlite").to_string_lossy(),
            webkit6::CookiePersistentStorage::Sqlite,
        );
    }
    let mut pressure = webkit6::MemoryPressureSettings::new();
    pressure.set_memory_limit(memory_limit_mb());
    let context = webkit6::WebContext::builder()
        .memory_pressure_settings(&pressure)
        .build();
    lite::register(&context);
    let view = WebView::builder()
        .web_context(&context)
        .network_session(&session)
        .build();
    if let Some(settings) = WebViewExt::settings(&view) {
        // RWID_SHELL_DEVTOOLS: inspector + page console in the journal (debugging only).
        let devtools = std::env::var_os("RWID_SHELL_DEVTOOLS").is_some();
        settings.set_enable_developer_extras(devtools);
        settings.set_enable_write_console_messages_to_stdout(devtools);
        settings.set_user_agent_with_application_details(
            Some("RWID-OS"),
            Some(env!("CARGO_PKG_VERSION")),
        );
    }

    let desk = windows::Desk::new(app, view.clone(), &shell_origin);
    // The RWID desktop or its chrome cannot load (or answers 5xx) → ecOS Lite on
    // both surfaces; the gateway is probed off the main thread until it answers,
    // then the RWID desktop comes back.
    let waiting = Rc::new(Cell::new(false));
    let fall_back: Rc<dyn Fn()> = {
        let (desk, waiting) = (Rc::downgrade(&desk), waiting.clone());
        let (desktop_url, chrome_url) = (desktop_url.clone(), chrome_url.clone());
        Rc::new(move || {
            let Some(d) = desk.upgrade() else { return };
            d.load(&lite::url("desktop"), &lite::url("chrome"));
            if waiting.replace(true) {
                return;
            }
            let (desk, waiting) = (desk.clone(), waiting.clone());
            let (desktop_url, chrome_url) = (desktop_url.clone(), chrome_url.clone());
            glib::timeout_add_local(RETRY, move || {
                let (d, w) = (desk.clone(), waiting.clone());
                let (du, cu) = (desktop_url.clone(), chrome_url.clone());
                glib::spawn_future_local(async move {
                    let probe_url = du.clone();
                    let up = gio::spawn_blocking(move || lite::probe(&probe_url))
                        .await
                        .unwrap_or(false);
                    if up && w.replace(false) {
                        tracing::info!("RWID desktop reachable again; leaving ecOS Lite");
                        if let Some(d) = d.upgrade() {
                            d.load(&du, &cu);
                        }
                    }
                });
                if waiting.get() {
                    glib::ControlFlow::Continue
                } else {
                    glib::ControlFlow::Break
                }
            });
        })
    };
    for v in [view.clone(), desk.chrome_view().clone()] {
        let on_fail = fall_back.clone();
        v.connect_load_failed(move |_, _event, failing_uri, err| {
            if failing_uri.starts_with(lite::PREFIX) {
                return false;
            }
            tracing::warn!(uri = failing_uri, error = %err, "shell load failed; ecOS Lite");
            on_fail();
            true
        });
        let (fall_back, shell_origin) = (fall_back.clone(), shell_origin.clone());
        v.connect_load_changed(move |v, event| {
            let status = v
                .main_resource()
                .and_then(|r| r.response())
                .map(|r| r.status_code())
                .unwrap_or(0);
            if event == LoadEvent::Finished
                && status >= 500
                && v.uri().is_some_and(|u| u.starts_with(&shell_origin))
            {
                tracing::warn!(status, "shell answered with an error; ecOS Lite");
                fall_back();
            }
        });
    }
    {
        let shell_origin = shell_origin.clone();
        view.connect_decide_policy(move |_view, decision, kind| match kind {
            PolicyDecisionType::NavigationAction => {
                let uri = decision
                    .downcast_ref::<NavigationPolicyDecision>()
                    .and_then(|d| d.navigation_action())
                    .and_then(|a| a.request())
                    .and_then(|r| r.uri())
                    .map(|u| u.to_string())
                    .unwrap_or_default();
                if !allowed(&uri, &shell_origin) {
                    tracing::info!(uri, "blocked top-level navigation away from the shell");
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
    // A crashed web process takes every surface (one shared process): reload.
    {
        let desk = Rc::downgrade(&desk);
        view.connect_web_process_terminated(move |view, reason| {
            tracing::error!(?reason, "web process terminated; reloading the shell");
            view.reload();
            if let Some(d) = desk.upgrade() {
                d.chrome_view().reload();
            }
        });
    }

    native_bridge(&view, &shell_origin, &desk);
    let _socket = listen_compositor(desk.chrome_view());

    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title("RWID/OS")
        .child(&view)
        .build();
    if gtk4_layer_shell::is_supported() {
        window.init_layer_shell();
        window.set_layer(Layer::Background);
        window.set_namespace(Some("rwid-shell"));
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            window.set_anchor(edge, true);
        }
        window.set_exclusive_zone(-1);
        window.set_keyboard_mode(KeyboardMode::OnDemand);
    } else {
        tracing::warn!("compositor has no layer-shell; falling back to a fullscreen window");
        window.fullscreen();
    }
    window.present();
    desk.start(&desktop_url, &chrome_url);
}

/// Native-app bridge (see `native`): private world, top frame of the shell origin only.
fn native_bridge(view: &WebView, shell_origin: &str, desk: &Rc<windows::Desk>) {
    let Some(ucm) = view.user_content_manager() else {
        return;
    };
    if !ucm.register_script_message_handler_with_reply(native::HANDLER, Some(native::WORLD)) {
        tracing::error!("cannot register the native-app bridge");
        return;
    }
    // Top frame only; the script installs itself on the shell origin only (a
    // URL allow-list cannot carry the port). Views are pinned to that origin too.
    ucm.add_script(&UserScript::for_world(
        &native::script(shell_origin),
        UserContentInjectedFrames::TopFrame,
        UserScriptInjectionTime::Start,
        native::WORLD,
        &[],
        &[],
    ));
    // The handler owns the desk: it lives as long as the shell (the views keep the
    // handler), so window requests always find it.
    let (weak, shell_origin, desk) = (view.downgrade(), shell_origin.to_string(), desk.clone());
    ucm.connect_script_message_with_reply_received(
        Some(native::HANDLER),
        move |_, value, reply| {
            // Defence in depth: the script installs itself on the shell origin and ecOS Lite only.
            let on_shell = weak.upgrade().and_then(|v| v.uri()).is_some_and(|u| {
                u.starts_with(&format!("{shell_origin}/")) || u.starts_with(lite::PREFIX)
            });
            if !on_shell {
                reply.return_error_message("not the shell");
                return true;
            }
            let Ok(msg) = serde_json::from_str::<native::Envelope>(&value.to_str()) else {
                reply.return_error_message("bad message");
                return true;
            };
            tracing::debug!(from = msg.win, req = msg.req, "bridge request");
            let answer = match native::handle(&msg.req) {
                native::Outcome::Reply(a) => a,
                native::Outcome::Window(req) => desk.request(&msg.win, req),
            };
            match value.context() {
                Some(ctx) => {
                    reply.return_value(&javascriptcore::Value::new_string(&ctx, Some(&answer)))
                }
                None => reply.return_error_message("no context"),
            }
            true
        },
    );
}

/// Compositor → shell commands on a datagram socket in the user's runtime dir
/// (`RWID_SHELL_SOCKET`; no network port). `toggle-start`: Super tapped alone.
fn listen_compositor(view: &WebView) -> Option<glib::SourceId> {
    let path = std::env::var_os("RWID_SHELL_SOCKET")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("rwid-shell.sock"))
        })?;
    let _ = std::fs::remove_file(&path);
    let sock = match UnixDatagram::bind(&path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "no compositor socket; Super will not open the menu");
            return None;
        }
    };
    let _ = sock.set_nonblocking(true);
    let weak = view.downgrade();
    // Non-blocking socket, polled: one tiny datagram per Super tap.
    Some(glib::timeout_add_local(
        Duration::from_millis(50),
        move || {
            let mut buf = [0u8; 64];
            while let Ok(n) = sock.recv(&mut buf) {
                if &buf[..n] == b"toggle-start"
                    && let Some(v) = weak.upgrade()
                {
                    v.evaluate_javascript(
                        "document.dispatchEvent(new Event('rwid:toggle-start'))",
                        Some(native::WORLD),
                        None,
                        None::<&gtk4::gio::Cancellable>,
                        |_| {},
                    );
                }
            }
            glib::ControlFlow::Continue
        },
    ))
}

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .json()
        .with_target(false)
        .with_max_level(if std::env::var_os("RWID_SHELL_DEVTOOLS").is_some() {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        })
        .init();
    let app = gtk4::Application::builder().application_id(APP_ID).build();
    app.connect_activate(build);
    app.run_with_args::<&str>(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_is_pinned_to_the_shell_origin() {
        let o = origin("http://127.0.0.1:4610/");
        assert_eq!(o, "http://127.0.0.1:4610");
        assert!(allowed("http://127.0.0.1:4610/", &o));
        assert!(allowed("http://127.0.0.1:4610/masuk", &o));
        assert!(allowed(&lite::url("desktop"), &o));
        assert!(
            !allowed("http://127.0.0.1:46100/", &o),
            "port prefix must not match"
        );
        assert!(!allowed("https://evil.example/", &o));
        assert!(!allowed("file:///etc/passwd", &o));
        assert!(!allowed("ecos-lite-evil://lite/", &o));
    }
}
