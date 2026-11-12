# Nintendo Switch port investigation

**Status: experimental native Horizon bring-up, not a playable port.** This
report is based on upstream `LookingGlass` commit `3427ce5` and the separate
Android reference commit `e8f2ebe805e3d8d463c38b953ed0b883f8e3cc89` on branch
`android`. No proprietary game files are in this source tree or the probe.

## Findings

- The Android branch is useful for controller mapping, mobile performance
  profiles, and letting the player choose their own PK3 files. It is an Android
  app: its native Activity/JNI layer, GLES/NDK build, and storage APIs do not
  provide a Horizon runtime. Its Android setup document specifies a six-pack
  startup gate; upstream `src/assets.rs` instead mounts the player's top-level
  PK3s in case-insensitive filename order, with later files overriding earlier
  ones. Keep the upstream loader contract and its user-owned data boundary.
- The official Rust target `aarch64-nintendo-switch-freestanding` is Tier 3 and
  `no_std`. The Rust project does not ship a prebuilt standard library for it;
  the supported community workflow uses nightly `build-std` and `cargo-nx` to
  package NROs. The target page says Rust's test suite cannot run on it. See the
  [Rust target page](https://doc.rust-lang.org/beta/rustc/platform-support/aarch64-nintendo-switch-freestanding.html),
  [Switch Rust setup guide](https://github.com/aarch64-switch-rs/setup-guide),
  and [`cargo-nx`](https://github.com/aarch64-switch-rs/cargo-nx).
- The current game is not `no_std`: it uses `std` file, path, thread, and
  process APIs throughout, and depends on libraries that expect `std`.
  `cargo check` for the official target currently stops first in
  `openssl-sys` through `tungstenite`'s `native-tls` feature because no
  Horizon OpenSSL sysroot/pkg-config configuration is available. Even after
  removing that network dependency from the Switch build, the official target
  still lacks `std` for the game itself.
- Upstream pins Macroquad 0.4.14 over Miniquad 0.4.8. Miniquad documents
  Android, desktop Apple/Linux/Windows, iOS, and WebGL platforms; Switch is
  still an [open upstream issue](https://github.com/not-fl3/miniquad/issues/669),
  not an available backend. The issue discusses the `nx` crate but calls out
  the absence of hardware rendering there. A native game port needs an actual
  display/event implementation, not only an NRO linker.
- Other ports point to native Horizon integration. VoyagerNX is an ioquake3
  fork using libnx with controller support and player-owned `baseEF` data; its
  [GPL-2.0 source](https://github.com/faithvoid/VoyagerNX) is an architectural
  reference only. [OpenTrickyNX](https://github.com/DevCrashDay/OpenTrickyNX)
  uses libnx, SDL2 input/audio, and OpenGL or Mesa NVK, but depends on static
  recompilation of an Xbox game. [openPREY](https://github.com/hazevauks/openPREYswitch)
  supplies its own Horizon controller and OpenGL integration. Local
  the local QuarantineNX reference's `docs/switch-baseline.md` records
  a devkitPro/libnx and patched SDL3 experiment, but marks its graphics, audio,
  and NRO execution as not independently reproduced or device-verified.
- [FlashNX's build notes](https://github.com/Jonathan8520/FlashNX/blob/main/DEVELOPMENT.md)
  show a Rust-heavy alternative: a `staticlib` built for the Switch target and
  linked into a C++/libnx application with Mesa EGL/GLES. It uses a manually
  patched `std`-via-newlib build and a custom GL backend. That proves a possible
  architecture, not a drop-in runtime: its target patches and thread-local
  workarounds need review before adoption.

## NRO cross-build and display/input probe

The isolated `switch/toolchain-probe` package pins the community Rust `nx`
library to commit `5e0a67c630352c93414b421a77ed2de587f61393`. It uses that
library's HID and GPU-canvas APIs to show a short on-screen message, wait for
controller state, and exit on `+`. This dependency is limited to the probe
because it supplies the freestanding Rust NRO runtime and basic Horizon HID/GPU
service wrappers; the boring fallback is libnx FFI from a C/C++ host shim. The
probe does not render through Macroquad, OpenGL, or a game map.

Rebuild it with Rust nightly `nightly-2026-11-10` plus `rust-src`, and
`cargo-nx` at `0dfe4a3ca6de4dcb0dfabed7dbfe9cacbbe21fcc`:

```sh
rustup toolchain install nightly-2026-11-10 --component rust-src
cargo install cargo-nx --git https://github.com/aarch64-switch-rs/cargo-nx \
  --rev 0dfe4a3ca6de4dcb0dfabed7dbfe9cacbbe21fcc --locked
cd switch/toolchain-probe
cargo nx build --release
```

The cross-build produced:

```text
switch/toolchain-probe/target/aarch64-nintendo-switch-freestanding/release/looking-glass-switch-probe.nro
size:   161826 bytes
sha256: bd17db988f7e7b4d95ac39332ad48cdffbc6009fe179b8900325d343ccb60338
```

This is compiler and packaging evidence only. The NRO has **not** been started
on a Switch; display, controller, exit, suspend, memory, and frame timing remain
unverified on hardware. This run did not include a Switch console.

## Smallest next playable milestone

Keep the upstream Rust recreation and PK3 loader. First establish a supported
Rust runtime for the engine (likely `std` via newlib linked into a libnx host),
then implement or adopt a reviewed Miniquad backend over the target's EGL/GLES
path, and bridge Nintendo controller events into the existing input model. Use
explicit writable `sdmc:/switch/looking-glass/` data/save paths. Gate co-op/TLS
and audio separately until Horizon backends exist. The first playability claim
requires device evidence for a player-supplied PK3, a real map, movement/look,
jump, attack, pause, save, exit, relaunch, and load. This report establishes
none of those game behaviors.
