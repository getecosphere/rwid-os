# RWID/OS

The open components of RWID/OS, an operating system that helps Indonesian students and young
people learn and work with AI.

| Crate | What it is |
|---|---|
| `crates/rwid-compositor` | The Wayland compositor (Rust, [Smithay](https://github.com/Smithay/smithay)), started from Smithay's `anvil` reference compositor. DRM/KMS + libinput, XWayland, layer-shell. |
| `crates/rwid-shell` | The shell host: a web desktop in WebKitGTK on layer-shell surfaces (desktop, dock/start-menu chrome, app windows) and the native bridge it talks to. |
| `crates/rwid-shell/lite` | **ecOS Lite**: a plain desktop (top bar, dock, start menu of the installed apps) built into the shell host. It is the fallback when the web desktop cannot load, and the reference for the bridge. |

Design decisions are in `docs/adr/`.

## Build (Fedora 44)

    sudo dnf install rust cargo libinput-devel libseat-devel mesa-libgbm-devel mesa-libEGL-devel \
      libdrm-devel systemd-devel libxkbcommon-devel wayland-devel pixman-devel libdisplay-info-devel \
      gtk4-devel webkitgtk6.0-devel gtk4-layer-shell-devel
    cargo build --release
    cargo run -p rwid-compositor -- --backend winit   # nested, inside an existing desktop

`rwid-shell` loads the web desktop from `RWID_SHELL_URL`; when it cannot load, the shell shows
ecOS Lite. The RWID web desktop (ecOS) itself is not part of this repository.

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
`rwid-compositor` also carries Smithay's anvil MIT notice (`crates/rwid-compositor/LICENSE-smithay-anvil.txt`).
"RWID/OS" and the RWID logo are trademarks, see [TRADEMARKS.md](TRADEMARKS.md).

Contributions are accepted under the same dual license. Fixes for Smithay itself are not sent
upstream from this project; see Smithay's own contribution policy.
