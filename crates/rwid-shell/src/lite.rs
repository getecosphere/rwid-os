//! ecOS Lite (ADR-0025): the desktop the shell falls back to when the RWID desktop
//! cannot load (gateway down, frontend failing, 5xx). A plain page — top bar, dock,
//! start menu of the installed apps, open-window chips — speaking the same bridge
//! as ecOS, so the terminal and every native app stay one Super tap away.
//!
//! Its three files are compiled into this binary (it can never be missing) and
//! served on the `ecos-lite://` scheme: no network port, and the scheme is
//! registered as local, so web pages cannot load or frame it.

use gtk4::{gio, glib};
use webkit6::{URISchemeRequest, WebContext};

pub const SCHEME: &str = "ecos-lite";
/// Every ecOS Lite URL starts with this (navigation pin, bridge).
pub const PREFIX: &str = "ecos-lite://lite/";

/// The ecOS Lite page for `surface` (`desktop` or `chrome`).
pub fn url(surface: &str) -> String {
    format!("{PREFIX}index.html?surface={surface}")
}

fn file(path: &str) -> Option<(&'static str, &'static str)> {
    match path {
        "/index.html" => Some((include_str!("../lite/index.html"), "text/html")),
        "/lite.css" => Some((include_str!("../lite/lite.css"), "text/css")),
        "/lite.js" => Some((include_str!("../lite/lite.js"), "text/javascript")),
        _ => None,
    }
}

pub fn register(context: &WebContext) {
    if let Some(sm) = context.security_manager() {
        sm.register_uri_scheme_as_local(SCHEME);
        sm.register_uri_scheme_as_secure(SCHEME);
    }
    context.register_uri_scheme(SCHEME, serve);
}

fn serve(req: &URISchemeRequest) {
    let path = req.path().map(|p| p.to_string()).unwrap_or_default();
    match file(&path) {
        Some((body, mime)) => {
            let stream =
                gio::MemoryInputStream::from_bytes(&glib::Bytes::from_static(body.as_bytes()));
            req.finish(&stream, body.len() as i64, Some(mime));
        }
        None => {
            let mut err = glib::Error::new(gio::IOErrorEnum::NotFound, "not part of ecOS Lite");
            req.finish_error(&mut err);
        }
    }
}

/// Does the shell at `url` answer (any status below 500)? A plain HTTP/1.0 GET over
/// loopback TCP, run off the main thread; non-HTTP URLs are just tried.
pub fn probe(url: &str) -> bool {
    use std::io::{Read, Write};
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    let Some(rest) = url.strip_prefix("http://") else {
        return true;
    };
    let (host, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let path = if path.is_empty() { "/" } else { path };
    let addr = if host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:80")
    };
    let Some(sock) = addr.to_socket_addrs().ok().and_then(|mut a| a.next()) else {
        return false;
    };
    let Ok(mut s) = TcpStream::connect_timeout(&sock, Duration::from_secs(2)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    if write!(s, "GET {path} HTTP/1.0\r\nHost: {host}\r\n\r\n").is_err() {
        return false;
    }
    let mut head = [0u8; 16];
    let n = s.read(&mut head).unwrap_or(0);
    status_ok(&head[..n])
}

/// `HTTP/1.x NNN` with NNN below 500.
fn status_ok(head: &[u8]) -> bool {
    std::str::from_utf8(head)
        .ok()
        .and_then(|h| h.strip_prefix("HTTP/1."))
        .and_then(|h| h.get(2..5))
        .and_then(|c| c.parse::<u16>().ok())
        .is_some_and(|c| (100..500).contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_only_its_own_files() {
        assert!(file("/index.html").is_some());
        assert!(file("/lite.js").is_some());
        assert!(file("/../etc/passwd").is_none());
        assert!(file("/").is_none());
        assert!(url("chrome").starts_with(PREFIX));
    }

    #[test]
    fn status_line() {
        assert!(status_ok(b"HTTP/1.1 200 OK\r\n"));
        assert!(status_ok(b"HTTP/1.0 302 Found"));
        assert!(!status_ok(b"HTTP/1.1 502 Bad G"));
        assert!(!status_ok(b"garbage"));
        assert!(!status_ok(b""));
    }

    #[test]
    fn probe_refuses_a_closed_port() {
        // Port 9 (discard) is not listening on a test machine's loopback.
        assert!(!probe("http://127.0.0.1:9/webos"));
        assert!(probe("file:///anything"));
    }
}
