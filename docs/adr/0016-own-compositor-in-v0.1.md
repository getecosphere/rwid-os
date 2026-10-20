# ADR-0016: Our own compositor ships in v0.1; the ISO comes first

- Status: Accepted (owner, 2026-10-13)
- Context: blueprint §11 orders the work compositor (Fase 1) → shell → agent → … → installable
  image (Fase 5). The goal is an installable `rwid-os-<version>.iso` as early as possible, and every
  milestone should be something people can install and see.

## Options
1. Follow the blueprint order; the first ISO arrives at Fase 5 (~4–6 months).
2. ISO first on an existing kiosk compositor (`cage`), swap in `rwid-compositor` later.
3. ISO first, **with `rwid-compositor` from the start** (minimal: floating windows, DRM/KMS, input,
   XWayland, layer-shell for the shell), growing in place.

## Decision
Option 3. v0.1 = Fedora Atomic (bootc) image + `rwid-compositor` + the web shell, installable from
an ISO. Option 2 was rejected by the owner: an OS whose screen is drawn by someone else's
compositor is not RWID/OS.

## Consequences
- Fase 1 (compositor) and Fase 5 (image + ISO) are pulled into v0.1 together, in minimal form.
- The compositor starts from Smithay's reference code (MIT) and is reduced to what v0.1 needs; the
  udev (DRM/KMS) backend is required from day one because the ISO runs on real hardware/VMs.
- Every later phase ships as a new ISO version.
