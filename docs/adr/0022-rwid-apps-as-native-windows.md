# ADR-0022: RWID apps as native windows, chrome above every window, one shared web process

- Status: Accepted (owner, 2026-10-16) · v0.2 "Subhanallah" · supersedes ADR-0021's consequence "the
  start menu is drawn by the background-layer shell"
- Context: the whole RWID desktop is one WebKit view on the background layer, so a native window
  (Terminal) always covers RWID apps and the start menu. The owner: RWID apps and native apps must be
  equals ("satu level, first class citizen"), the start menu must stay on top **and look exactly like
  the web**, and **RAM decides whether the OS makes sense** — opening an app must not cost a new
  browser window.

## Measurement (WebKitGTK 2.54, Fedora 44, 2026-10-16; PSS of the shell process tree)

| Approach | 3 apps | 5 apps | per app |
|---|---|---|---|
| Shell only (globe desktop) | 672 MB | — | — |
| Apps as iframes inside the shell (today; not window-level) | 670 MB | 659 MB | ~0–2 MB |
| **Native window, related view (shares the shell's web process)** | 753 MB | 792 MB | **~25 MB** |
| Same, GPU compositing off for app windows | 739 MB | 781 MB | ~23 MB |
| Native window, own web process (= a new browser window) | 918 MB | — | **~82 MB** |

`WEBKIT_CACHE_MODEL_DOCUMENT_VIEWER` / page cache off changed nothing measurable.

On the installed device (VM 212, software rendering — window buffers count as RAM; a real GPU
holds them in video memory), one web process for every surface:

| Desktop + chrome + | Total PSS |
|---|---|
| 1 app window | 798 MB |
| 3 app windows | 899 MB (~50 MB per window at 1100×720) |

## Decision

1. **Three kinds of surface, one web process.** `rwid-shell` keeps the desktop view (globe, widgets)
   on the background layer and adds, as *related views* of it (same web process, same network
   session and cookie — one sign-in):
   - a **chrome view** on the layer-shell `top` layer, full screen and transparent: ecOS renders its
     dock, start menu, launchpad and lock screen there (the top bar stays on the desktop, kept
     clear by a reserved strip) (the same DOM and CSS as the web, so they look the same),
     and reports the rectangles it occupies as the surface's input region (clicks elsewhere reach the
     windows below). Two empty layer surfaces reserve the top-bar and dock strips (exclusive zones),
     so a maximised window never hides them.
   - one **xdg_toplevel per RWID app window**, decorated by the compositor exactly like a native
     window (ADR-0021). Web apps load their own page; built-in panes (App Store, Settings) load the
     shell URL in `surface=app` mode, which shows only that pane.
2. **Memory.** No per-window web process (~25 MB per app instead of ~82 MB); a memory-pressure limit
   on the web process so WebKit drops caches before the device swaps; closing a window destroys its
   view. GPU compositing stays on (2 MB/app is not worth jank).
3. **ecOS knows the surface** (`?surface=desktop|chrome|app`, device only). On the device,
   `openAppWindow` and the built-in panes ask the shell to open/raise a native window; the dock shows
   running apps from the shell's window list. The web is unchanged.
4. **Bridge** (private world, shell origin only — ADR-0021): new requests `open-window` (app id from
   the catalogue, path on the shell origin only), `focus-window`, `close-window`, `input-region`,
   `reserve`, `windows`. Every surface is our own page on the shell origin and shares the bridge;
   the shell tags each view's surface in the private world, so `close-window` closes only the
   sender's own window and only the chrome may set the input region.

## Implementation notes (2026-10-16)
- `WebView::builder().related_view(v)` shares the web process only; settings (user agent) and the
  user content manager (the bridge) must be passed too — what the C helper
  `webkit_web_view_new_with_related_view` does.
- WebKit allow-list URL patterns cannot carry a port: the bridge script is injected in every top
  frame and checks `location.origin` itself.
- The chrome's click region comes from the dock's children (`#dbar > *`, `#dock`): in the classic
  layout `#dbar` itself is a zero-size static box.
- `RWID_SHELL_DEVTOOLS=1` also writes the pages' console and every bridge request to the journal.

## Memory work (2026-10-16, device VM, software rendering)
- **Light chrome page** (`/os-chrome/`: dock, start menu, launchpad, lock screen — no globe, widgets
  or feeds) and a **chrome surface that shrinks to the dock strip** when no menu is open (ecOS
  reports `chrome-size`). Desktop + chrome idle: **674 MB**; with one app window **728 MB** (was
  798 MB before).
- **Minimised app windows are discarded** after `RWID_SHELL_DISCARD_SECS` (default 300 s; at once
  when MemAvailable < 15 %): the compositor sends xdg-shell `suspended`, the shell destroys the view
  and keeps its URL, and reloads it when the window comes back (Super+M minimises, Alt/Super+Tab
  restores). Measured: the page is freed inside the shared web process, which keeps the memory for
  the next page rather than returning it to the system at once (733 MB after 150 s minimised) — it
  caps growth; the memory-pressure limit returns it when the device runs short.

## Consequences
- RWID apps and native apps stack, focus, minimise and Alt+Tab the same way.
- A crash of the shared web process takes every RWID window with it; the shell reloads them.
- Next RAM steps if needed: discard long-minimised app views (snapshot + reload on restore).
