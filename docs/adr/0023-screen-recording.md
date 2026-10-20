# ADR-0023: Screen recording — OBS ready to record, captured by our compositor

- Status: Accepted (owner, 2026-10-16) · v0.2.3 "Subhanallah"
- Context: the owner asked: *"apakah bisa OBS langsung ada scene fullscreen screen record? jadi siswa
  tinggal record?"* Students record tutorials, assignments and portfolio videos; on a fresh RWID/OS
  they must be able to press one thing and record, offline, with no setup wizard.

## Decision

1. **The compositor provides screen capture** with `ext-image-copy-capture-v1` (plus
   `ext-output-image-capture-source-manager-v1`), the standard protocol that xdg-desktop-portal-wlr,
   grim and OBS (through the portal) use. Upstream anvil only stubs it. `rwid-compositor` renders the
   output once more into an offscreen texture and copies it into the client's SHM buffer
   (`capture.rs`, udev backend). SHM only, no DMA-BUF; the cursor is not in the frame.
2. **The portal chain:** OBS → PipeWire → `xdg-desktop-portal` → `xdg-desktop-portal-wlr`
   (ScreenCast, Screenshot) and `xdg-desktop-portal-gtk` for the rest
   (`/usr/share/xdg-desktop-portal/rwid-portals.conf`, `XDG_CURRENT_DESKTOP=RWID`). xdpw uses
   `chooser_type=none`, so there is no picker: a recording is the whole screen.
3. **A session target.** xdg-desktop-portal requires `graphical-session.target`; the session starts
   `rwid-session.target` (BindsTo it) after importing `WAYLAND_DISPLAY` into the user manager, and
   stops it when the compositor ends (`rwid-shell-loop`).
4. **OBS ready to record**, seeded once into a user's home by `seed-home`:
   - scene **"Layar penuh"** (PipeWire screen capture + microphone + desktop audio), Indonesian UI;
   - canvas and output at the screen's native mode (read from DRM before the compositor starts),
     so code text stays sharp;
   - **OpenH264 → MP4** in `~/Videos` (Fedora's OBS has no x264; OpenH264 is Cisco's binary, works
     offline); no first-run wizard;
   - launcher **"Rekam Layar"**: `obs --startrecording` — one click starts recording.
5. Packages arrive in one late image layer (`obs-studio`, the portals, PipeWire, `grim`), so the
   update downloads them once.

## Consequences

- Any portal client (OBS, browsers' screen share, screenshot tools) can capture the screen. The
  compositor only answers on the user's own Wayland socket; there is no network surface.
- Capture works on the udev backend (the device); the nested winit/x11 backends fail frames.
- Found while testing: Qt sends `wl_surface.offset(0, 0)` on every commit, and re-mapping a window
  on commit raised it — OBS's main window covered its own dialogs. Commit-time moves now use
  `relocate_element` (no raise).
- Measured in VM 212 (software rendering, 1280×800): 30 fps recording, OBS ~36% CPU.
