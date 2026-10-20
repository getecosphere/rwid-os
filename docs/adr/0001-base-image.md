# ADR-0001: Base image — `quay.io/fedora/fedora-bootc:44`

- Status: Accepted (2026-10-13)
- Context: blueprint §6.1 asks for an immutable, atomically updated base built like a container.
  Options: `fedora-bootc` (minimal, no desktop) vs a Fedora Atomic desktop base (Silverblue/Kinoite).

## Decision
`quay.io/fedora/fedora-bootc:44` (Fedora 44, the current stable release; 45/46 tags exist but are not
released). We add only what RWID/OS needs: Mesa, libinput/libseat, PipeWire, NetworkManager, greetd,
XWayland, WebKitGTK, and our binaries. No GNOME/KDE.

## Consequences
- Smallest image and attack surface; we own the whole session.
- Rebase twice a year with Fedora releases (§6.1).
- ISO and qcow2 come from `bootc-image-builder`; built on the RWID/OS builder VM (x86_64).

## os-release (2026-10-14)
`NAME="RWID/OS"`, `PRETTY_NAME="RWID/OS 0.1.0 (Bismillah)"`, `VERSION="0.1.0 (Bismillah)"`,
`IMAGE_VERSION=0.1.0`, `ID=rwid-os`, `ID_LIKE=fedora`, `VERSION_ID=44`. `VERSION_ID` is the platform
(Fedora) version, as in other Fedora derivatives (Bazzite, Bluefin): the image builder resolves its
installer definitions by `ID`/`ID_LIKE` + `VERSION_ID`, and rejects both a non-Fedora version and
comment lines in os-release.
