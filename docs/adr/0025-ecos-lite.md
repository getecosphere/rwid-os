# ADR-0025: ecOS Lite — the fallback desktop, and the open shell

- Status: Accepted (owner, 2026-10-20) · ships with the next ISO
- Context: when the RWID desktop cannot load (gateway down, the frontend failing or answering 5xx),
  the shell showed a static offline page and reloaded the RWID URL every 3 s: no menu, no dock, only
  the compositor's `Super+Enter` terminal. Separately, opening the OS components (ADR-0010) needs a
  desktop that is not ecOS, which stays closed. The owner named it **ecOS Lite** and asked that it
  also be the fallback.

## Decision

1. **ecOS Lite** is a plain page — no framework, no build step, three files
   (`crates/rwid-shell/lite/`): a top bar with the clock and a short note on the desktop surface;
   a dock (start button, the terminal, open-window chips) and a start menu of the installed native
   apps (search, arrow keys, Enter, Esc, Super) on the chrome surface. Indonesian or English from the
   system language. No sign-in, network, storage, RWID apps, settings or themes: those are ecOS.
2. It speaks **the same bridge as ecOS** (`apps`, `launch`, `windows`, `focus-window`,
   `input-region`, `reserve`, `chrome-size`; `rwid:toggle-start`, `rwid:windows`), so it is also the
   reference for anyone writing another desktop for `rwid-shell`.
3. **Served from the binary** on the `ecos-lite://` scheme (`include_str!`, registered as local and
   secure): it cannot be missing from an image and opens no port. The bridge accepts the shell origin
   and `ecos-lite://lite/` only; navigation is pinned to the same two.
4. **Fallback:** a failed load or a 5xx on the desktop or the chrome puts both surfaces on ecOS Lite.
   The gateway is then probed every 5 s off the main thread (HTTP/1.0 GET over loopback, < 500 is
   up) and the RWID desktop is loaded back once it answers — no reload loop, so the Lite menu stays
   usable meanwhile. The static offline page is removed.
5. ecOS Lite is open (ADR-0010): it lives inside `crates/rwid-shell`, which the public export
   carries.

## Consequences

- An RWID app window that was open when the desktop failed keeps pointing at the gateway; it works
  again when the gateway does.
- Native windows (foot, …) are toplevels of the compositor, not shell windows: the dock chips list
  only the shell's app windows, as in ecOS.
- Checked: `cargo fmt`, `clippy -D warnings` and the unit tests (scheme serves only its three files,
  status-line parsing, probe of a closed port, bridge pinned to the shell origin and Lite) in a
  Fedora 44 container. Not yet seen on a screen: verify on the laptop or a test VM before the ISO.
