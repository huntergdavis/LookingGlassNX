# Miniquad Horizon title/input spike

This isolated package uses the engine's pinned Macroquad 0.4.14 and vendored
Miniquad 0.4.8. It is a title/input renderer scaffold only. `Start` is a
placeholder; it does not load Alice assets or enter a map.

The Horizon native runner uses the applet EGL window, requests GLES3, resolves
Miniquad GL functions through `eglGetProcAddress`, sets the display dimensions
before constructing Macroquad's stage and exposes controller state to the
existing `RawPad` input path. The engine's UI maps that pad to navigation and
accept/back actions; the host sends no second set of keyboard events. The C
host owns EGL and swaps each frame. Missing GLES3 or core VAO entry points
causes startup failure. This ES3/VAO contract has not been checked on a Switch
in this lane.

This package is not linked with the accepted standalone EGL smoke and does not
change that smoke's source or artifact. It has no game-data dependency. The
current title uses `Window::from_config`; the vendored Miniquad `start()` has a
Horizon arm that installs the callback object supplied by Macroquad. It still
needs a device run and a full engine integration before Alice title/menu
behavior can be claimed.

Build from this package directory so Cargo loads its `.cargo/config.toml`:
`cargo build --release --locked --offline`. Use the pinned patched nightly and
run `make` in the local `local-alice-switch-builder:latest` devkitPro image to
link the NRO. The pinned toolchain source already contains Horizon std
registration from the prior runtime spike; the package config supplies the
`target_family=unix`, `target_env=newlib`, `unix`, and TLS rustflags. Toolchain
cache and builder image remain local infrastructure.
