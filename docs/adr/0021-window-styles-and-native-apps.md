# ADR-0021: Window styles, native apps in the RWID menu, Super key

- Status: Accepted (owner, 2026-10-15) · v0.2 "Subhanallah"
- Context: on the device, native windows (terminal, editors) carried anvil's placeholder title
  bar (flat rectangles, square corners), and only RWID web apps were in the start menu. The owner
  asked for one look and one menu: "user tidak akan tahu bedanya". A rethink of window management
  itself (no dock / no taskbar concepts: Orbit, Niat, Alur, Ruang —
  `rwid-os-assets/design/window-concepts.png`) is parked: it needs deeper design.

## Decision

1. **Three window styles, the user picks** (Settings → Appearance → Window style): `mac` (default;
   lights left, title centred), `windows` (title left; minimize, maximize, close right; close turns
   red), `capsule` (RWID: controls in one pill, gold line on the active window). Mockups:
   `rwid-os-assets/design/window-decorations.png` (options A, B, D).
2. **One setting, two renderers.** ecOS stores it per device and styles its own windows
   (`<html data-wm-style>`). On RWID/OS the shell writes it to `~/.config/rwid-os/window-style`
   through its bridge; the compositor reads that file (at most once a second) and draws native
   windows the same way.
3. **Rounded without shaders.** The compositor draws a rounded card behind each client: title bar
   (34 px) plus a 3 px frame with a 10 px radius. 3 px ≥ 0.29 × radius, so the client's square
   corners stay inside the rounded edge. Server-side decorations are the default for every client
   that speaks xdg-decoration. Minimize hides the window; Alt/Super+Tab brings it back.
4. **Native apps in the RWID menu.** `rwid-shell` exposes a bridge in a private script world,
   injected only in the top frame of the shell origin. It answers three requests: list the
   installed desktop apps, launch one **by desktop id from that list** (never a command line), and
   read/set the window style. Terminal apps (vim, emacs -nw) open in foot.
5. **Super alone toggles the menu.** The compositor detects a lone Super tap, gives the shell the
   keyboard and sends `toggle-start` on a datagram socket in `$XDG_RUNTIME_DIR` (no network port).

## Consequences
- The start menu is still drawn by the background-layer shell, so a native window can cover it;
  moving the menu/dock to a top layer belongs to the parked window-management redesign.
- No task switcher yet beyond Alt/Super+Tab; a minimized native window is not shown in the dock.
