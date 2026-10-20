# rwid-compositor

The RWID/OS Wayland compositor (ADR-0016). It starts from Smithay's `anvil`
reference compositor (MIT, `LICENSE-smithay-anvil.txt`), pinned to Smithay
commit `9428a670`, and is reduced and branded for RWID/OS v0.1:
DRM/KMS + libinput (udev), XWayland, layer-shell for the web shell.

    cargo run -p rwid-compositor -- --backend winit   # nested, for development
    rwid-compositor --backend udev                    # the session (from greetd)

Build dependencies (Fedora): libinput, libseat, mesa-libgbm, mesa-libEGL, libdrm,
systemd, libxkbcommon, wayland, pixman, libdisplay-info (`-devel` packages).
