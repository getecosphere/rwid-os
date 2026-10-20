//! Native apps in the RWID desktop (ADR-0021): the ecOS start menu lists the
//! installed desktop apps (`.desktop`) next to the RWID apps and launches them, so
//! a student types "terminal" and gets the terminal — one menu, no difference.
//!
//! The bridge lives in a private script world (`rwid-shell`) that is injected in
//! the **top frame** and installs itself only on the **shell origin** (WebKit's
//! allow-list patterns cannot name a port, so the script checks `location.origin`); page code reaches it through DOM
//! events (`rwid:req` → `rwid:res`), app iframes and other origins cannot. Only
//! three native requests exist: list the apps, launch one **by its desktop id from
//! that list** — never a command line from the page — and read/set the window
//! style the compositor draws (`~/.config/rwid-os/window-style`). The window
//! requests (ADR-0022) are answered by `windows`: each message carries the
//! surface it came from (`desktop`, `chrome` or an app window id), set by the
//! shell in the private world where page code cannot change it.

use gio_unix::prelude::*;
use gtk4::{gdk, gio, glib, prelude::*};
use serde::{Deserialize, Serialize};

pub const WORLD: &str = "rwid-shell";
pub const HANDLER: &str = "rwid";

/// Bridge script, run in the private world. The page writes a JSON request to
/// `<html data-rwid-req>` and dispatches `rwid:req`; the answer comes back in
/// `data-rwid-res` with a `rwid:res` event.
/// Bridge script for the shell at `origin` and ecOS Lite (it refuses any other page).
pub fn script(origin: &str) -> String {
    SCRIPT
        .replace(
            "__ORIGIN__",
            &serde_json::Value::String(origin.into()).to_string(),
        )
        .replace(
            "__LITE__",
            &serde_json::Value::String(crate::lite::PREFIX.into()).to_string(),
        )
}

const SCRIPT: &str = r#"(() => {
  if (location.origin !== __ORIGIN__ && !location.href.startsWith(__LITE__)) return;
  const h = window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.rwid;
  if (!h) return;
  document.addEventListener("rwid:req", () => {
    const de = document.documentElement;
    const req = de.getAttribute("data-rwid-req");
    de.removeAttribute("data-rwid-req");
    if (!req) return;
    h.postMessage(JSON.stringify({ win: String(window.__rwidWin || ""), req: req })).then((res) => {
      de.setAttribute("data-rwid-res", String(res));
      document.dispatchEvent(new Event("rwid:res"));
    }, () => {});
  });
})();"#;

/// Largest icon embedded in the app list.
const ICON_MAX: u64 = 256 * 1024;

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "lowercase", deny_unknown_fields)]
pub enum Request {
    Apps,
    Launch {
        id: String,
    },
    #[serde(rename = "window-style")]
    WindowStyle {
        #[serde(default)]
        style: Option<String>,
    },
    /// Open or raise the native window of an RWID app (path on the shell origin).
    #[serde(rename = "open-window")]
    OpenWindow {
        id: String,
        title: String,
        path: String,
    },
    #[serde(rename = "focus-window")]
    FocusWindow {
        id: String,
    },
    /// An app window closes itself.
    #[serde(rename = "close-window")]
    CloseWindow,
    /// The open app windows (also pushed to the chrome on every change).
    Windows,
    /// Chrome: where it takes clicks (`[x, y, w, h]`, logical px).
    #[serde(rename = "input-region")]
    InputRegion {
        rects: Vec<[i32; 4]>,
    },
    /// Strip to keep free of windows: `top` (desktop's top bar) / `bottom` (dock).
    Reserve {
        edge: String,
        px: i32,
    },
    /// Chrome: full screen (menu, launchpad, modal) or just the dock strip.
    #[serde(rename = "chrome-size")]
    ChromeSize {
        expanded: bool,
    },
}

/// A native request is answered here; a window request goes to `windows`.
pub enum Outcome {
    Reply(String),
    Window(Request),
}

/// An RWID app id from the catalogue: `[a-z0-9-]`, at most 48 chars.
pub fn valid_app_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 48
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A path on the shell origin: absolute, not protocol-relative, no backslash.
pub fn valid_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && !path.contains('\\')
        && path.len() <= 512
        && !path.chars().any(char::is_control)
}

pub const STYLES: [&str; 3] = ["mac", "windows", "capsule"];

fn style_path() -> Option<std::path::PathBuf> {
    Some(glib::user_config_dir().join("rwid-os").join("window-style"))
}

/// Read the style, or set it (only one of `STYLES`); answers the current style.
fn window_style(set: Option<&str>) -> Result<String, &'static str> {
    let path = style_path().ok_or("no config dir")?;
    if let Some(style) = set {
        if !STYLES.contains(&style) {
            return Err("unknown style");
        }
        let dir = path.parent().ok_or("no config dir")?;
        std::fs::create_dir_all(dir).map_err(|_| "cannot create config dir")?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, style).map_err(|_| "cannot write")?;
        std::fs::rename(&tmp, &path).map_err(|_| "cannot write")?;
        tracing::info!(style, "window style set");
    }
    let cur = std::fs::read_to_string(&path).unwrap_or_default();
    Ok(STYLES
        .iter()
        .find(|s| **s == cur.trim())
        .unwrap_or(&STYLES[0])
        .to_string())
}

#[derive(Serialize)]
struct NativeApp {
    id: String,
    name: String,
    summary: String,
    keywords: Vec<String>,
    /// `data:` URL of the theme icon, empty when none is found.
    icon: String,
}

/// A desktop id as GIO reports it: `name.desktop`, no path, no spaces.
pub fn valid_id(id: &str) -> bool {
    id.len() <= 200
        && id.ends_with(".desktop")
        && !id.starts_with('.')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The outer message: the sender surface and the request (JSON string).
#[derive(Deserialize)]
pub struct Envelope {
    #[serde(default)]
    pub win: String,
    pub req: String,
}

/// Handle one bridge request (JSON string): answer it, or hand it to `windows`.
pub fn handle(raw: &str) -> Outcome {
    let req: Request = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(_) => return Outcome::Reply(r#"{"error":"bad request"}"#.into()),
    };
    Outcome::Reply(match req {
        Request::Apps => serde_json::json!({ "apps": list() }).to_string(),
        Request::WindowStyle { style } => match window_style(style.as_deref()) {
            Ok(style) => serde_json::json!({ "style": style }).to_string(),
            Err(e) => serde_json::json!({ "error": e }).to_string(),
        },
        Request::Launch { id } => match launch(&id) {
            Ok(()) => r#"{"ok":true}"#.into(),
            Err(e) => {
                tracing::warn!(id, error = e, "native launch refused");
                serde_json::json!({ "error": e }).to_string()
            }
        },
        other => return Outcome::Window(other),
    })
}

fn list() -> Vec<NativeApp> {
    let theme = gdk::Display::default().map(|d| gtk4::IconTheme::for_display(&d));
    let mut apps: Vec<NativeApp> = gio::AppInfo::all()
        .into_iter()
        .filter(|a| a.should_show())
        .filter_map(|a| {
            let id = a.id()?.to_string();
            if !valid_id(&id) {
                return None;
            }
            let keywords = a
                .downcast_ref::<gio_unix::DesktopAppInfo>()
                .map(|d| d.keywords().iter().map(|k| k.to_string()).collect())
                .unwrap_or_default();
            Some(NativeApp {
                icon: a
                    .icon()
                    .and_then(|i| icon_data_url(&i, theme.as_ref()))
                    .unwrap_or_default(),
                name: a.display_name().to_string(),
                summary: a.description().map(|d| d.to_string()).unwrap_or_default(),
                keywords,
                id,
            })
        })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

fn icon_data_url(icon: &gio::Icon, theme: Option<&gtk4::IconTheme>) -> Option<String> {
    let path = if let Some(f) = icon.downcast_ref::<gio::FileIcon>() {
        f.file().path()?
    } else {
        let themed = icon.downcast_ref::<gio::ThemedIcon>()?;
        let names = themed.names();
        let names: Vec<&str> = names.iter().map(|n| n.as_str()).collect();
        let first = *names.first()?;
        theme?
            .lookup_icon(
                first,
                &names[1..],
                64,
                1,
                gtk4::TextDirection::None,
                gtk4::IconLookupFlags::empty(),
            )
            .file()?
            .path()?
    };
    let mime = match path.extension()?.to_str()? {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        _ => return None,
    };
    if std::fs::metadata(&path).ok()?.len() > ICON_MAX {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        glib::base64_encode(&bytes)
    ))
}

fn launch(id: &str) -> Result<(), &'static str> {
    if !valid_id(id) {
        return Err("invalid id");
    }
    let app = gio_unix::DesktopAppInfo::new(id).ok_or("unknown app")?;
    if !app.should_show() {
        return Err("app is hidden");
    }
    let ctx = gdk::Display::default().map(|d| d.app_launch_context());
    let ctx = ctx
        .as_ref()
        .map(|c| c.upcast_ref::<gio::AppLaunchContext>());
    if app.boolean("Terminal") {
        // Terminal apps (vim, htop) open in the RWID terminal.
        let exec = app.commandline().ok_or("no command")?;
        let cmd = strip_field_codes(&exec.to_string_lossy());
        let term = std::env::var("RWID_TERMINAL").unwrap_or_else(|_| "foot".into());
        let wrapped = gio::AppInfo::create_from_commandline(
            format!("{term} {cmd}"),
            Some(&app.display_name()),
            gio::AppInfoCreateFlags::NONE,
        )
        .map_err(|_| "cannot wrap terminal app")?;
        wrapped.launch(&[], ctx).map_err(|_| "launch failed")?;
    } else {
        app.launch(&[], ctx).map_err(|_| "launch failed")?;
    }
    tracing::info!(id, "native app launched");
    Ok(())
}

/// Drop `%f %F %u %U %i %c %k` (no files are passed from the menu).
fn strip_field_codes(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|w| !(w.len() == 2 && w.starts_with('%')))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_plain_desktop_names() {
        assert!(valid_id("foot.desktop"));
        assert!(valid_id("org.gnome.Calculator.desktop"));
        assert!(valid_id("rwid-vim.desktop"));
        assert!(!valid_id("../../etc/passwd.desktop"));
        assert!(!valid_id("/usr/share/applications/foot.desktop"));
        assert!(!valid_id("foot"));
        assert!(!valid_id("a b.desktop"));
        assert!(!valid_id("x.desktop;rm -rf ~"));
        assert!(!valid_id(".hidden.desktop"));
        assert!(!valid_id(&format!("{}.desktop", "a".repeat(200))));
    }

    #[test]
    fn only_known_requests_exist() {
        assert_eq!(handle_parse(r#"{"op":"apps"}"#), Some(Request::Apps));
        assert_eq!(
            handle_parse(r#"{"op":"launch","id":"foot.desktop"}"#),
            Some(Request::Launch {
                id: "foot.desktop".into()
            })
        );
        // A command line is never accepted, nor unknown ops or extra fields.
        assert_eq!(handle_parse(r#"{"op":"exec","cmd":"rm -rf ~"}"#), None);
        assert_eq!(
            handle_parse(r#"{"op":"launch","id":"foot.desktop","args":["-e","sh"]}"#),
            None
        );
        assert_eq!(
            handle_parse(r#"{"op":"window-style","style":"capsule"}"#),
            Some(Request::WindowStyle {
                style: Some("capsule".into())
            })
        );
        assert_eq!(
            handle_parse(r#"{"op":"window-style"}"#),
            Some(Request::WindowStyle { style: None })
        );
        assert_eq!(handle_parse("not json"), None);
        assert!(
            matches!(handle("not json"), Outcome::Reply(r) if r == r#"{"error":"bad request"}"#)
        );
        assert!(matches!(
            handle(r#"{"op":"close-window"}"#),
            Outcome::Window(Request::CloseWindow)
        ));
        // window requests reject extra fields too (no url/command smuggling)
        assert_eq!(
            handle_parse(
                r#"{"op":"open-window","id":"x","title":"X","path":"/x","url":"https://evil"}"#
            ),
            None
        );
    }

    #[test]
    fn launch_refuses_bad_ids_before_touching_gio() {
        assert_eq!(launch("../evil.desktop"), Err("invalid id"));
        assert_eq!(launch("sh -c id"), Err("invalid id"));
    }

    #[test]
    fn window_style_only_known_values() {
        let dir = std::env::temp_dir().join(format!("rwid-shell-test-{}", std::process::id()));
        // SAFETY: single-threaded test; the variable only steers glib's config dir.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", &dir) };
        assert_eq!(window_style(Some("../../etc/passwd")), Err("unknown style"));
        assert_eq!(window_style(None).as_deref(), Ok("mac"));
        assert_eq!(window_style(Some("capsule")).as_deref(), Ok("capsule"));
        assert_eq!(
            std::fs::read_to_string(dir.join("rwid-os/window-style")).unwrap(),
            "capsule"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn window_paths_and_ids_stay_on_the_shell() {
        assert!(valid_path("/ekoin/"));
        assert!(valid_path("/webos?surface=app&app=store"));
        assert!(!valid_path("//evil.example/x"));
        assert!(!valid_path("https://evil.example/"));
        assert!(!valid_path("/x\\y"));
        assert!(!valid_path("/x\ny"));
        assert!(!valid_path(""));
        assert!(valid_app_id("upwork-learner"));
        assert!(!valid_app_id("Store"));
        assert!(!valid_app_id("a/b"));
        assert!(!valid_app_id(""));
    }

    #[test]
    fn bridge_script_is_pinned_to_the_shell_origin_and_lite() {
        let js = script("http://127.0.0.1:4610");
        assert!(js.contains(
            r#"if (location.origin !== "http://127.0.0.1:4610" && !location.href.startsWith("ecos-lite://lite/")) return;"#
        ));
        assert!(!js.contains("__ORIGIN__") && !js.contains("__LITE__"));
        // a hostile origin string cannot break out of the string literal
        let evil = script(r#"x"; alert(1); ""#);
        assert!(evil.contains(r#"!== "x\"; alert(1); \"""#));
    }

    #[test]
    fn field_codes_are_dropped() {
        assert_eq!(strip_field_codes("vim %F"), "vim");
        assert_eq!(strip_field_codes("emacs -nw %f"), "emacs -nw");
    }

    fn handle_parse(raw: &str) -> Option<Request> {
        serde_json::from_str(raw).ok()
    }
}
