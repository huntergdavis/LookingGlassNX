# LookingGlassNX Horizon host backend

This package directory contains the libnx/EGL host and its Cargo target
configuration. The resulting NRO links the root LookingGlassNX game crate,
whose real entry is `lookingglass_horizon_game_entry` in `src/main.rs`. It uses
the existing asset loader, viewer, title/menu, and `Input` model; it is not a
standalone title-color or input demo.

The Horizon bridge polls libnx once per applet frame. The root
`src/input.rs::read_pad` consumes the same XInput-compatible `RawPad` ABI the
backend fills, including mapped face/system buttons, triggers, and signed stick
axes. No host-side pad keys are synthesized and the host does not consume Plus;
Plus remains the engine Start/Escape action.

On Horizon, the game reads user-supplied assets from
`sdmc:/switch/alice-nx/game_root`. Saves and settings use the separate writable
profile `sdmc:/switch/alice-nx/profile` (saves under its `saves/` child). The
game-root directory is read-only from this path's perspective. The NRO does not
embed user assets.

Miniquad requests GLES3 and preflights the 121 functions present in both its
generated loader and the pinned GLES3 headers, plus the three VAO entry points
used by Miniquad. Four generated names outside GLES 3.0 are not required or
preflighted: `glClearDepth`, `glGetQueryObjectiv`, `glGetQueryObjectui64v`, and
`glPolygonMode`. Hardware GLES3/VAO availability and applet suspend/resume,
focus, and surface recreation remain unverified.

The C host serves Rust `std` through libnx CSRNG and the POSIX `getrandom` ABI.
It validates flags, zero-length and null-buffer requests, bounds the return
length, serializes service access, and reads large requests in bounded chunks.
CSRNG is initialized before the Rust entry and shut down after the game loop;
the root Rust crate supplies getrandom 0.3/0.4's custom-backend hook. The
service call's maximum chunk is not established by a device run.

The host also provides Rust `std`'s two observed `sysconf` queries because
libnx/newlib does not link its `sysconf` implementation here: `_SC_PAGESIZE`
returns 4096 and `_SC_NPROCESSORS_ONLN` returns one worker conservatively.
Compile-time assertions pin the newlib query values from the immutable builder
headers (8 and 10); all other queries return `-1` with `EINVAL`. The shared
mapping/errno helper has a host test, but target ABI and runtime behavior have
not been exercised on hardware.

Run commands from this directory so Cargo discovers `.cargo/config.toml`:

```sh
cargo check --manifest-path ../../Cargo.toml --lib --no-default-features --locked --offline
cargo build --manifest-path ../../Cargo.toml --lib --release --no-default-features --locked --offline
docker run --rm -v "$(pwd)/../..:/src" -w /src/switch/miniquad-horizon-spike local-alice-switch-builder@sha256:885f305c1f942475bf1d37681da8b4aebfa6eea9e16360625474c35b7611d587 make
mkdir -p target/host
rustc --edition=2018 --test ../../vendor/miniquad/src/native/horizon_pad.rs -o target/host/horizon-pad-tests
target/host/horizon-pad-tests
```

The build uses pinned nightly `nightly-2026-11-10` with the locally patched
Horizon `std` source and the immutable devkitPro image
`local-alice-switch-builder@sha256:885f305c1f942475bf1d37681da8b4aebfa6eea9e16360625474c35b7611d587`.
The `--no-default-features` build disables co-op TLS; default desktop TLS stays
enabled, and this build rejects `wss://` co-op invites before opening a socket.

The pure button/trigger mapper has two host-executed tests. They validate the
libnx enum to RawPad conversion but do not validate physical controls on Switch.
No NRO boot, game-data load, gameplay, pause/resume, or save/relaunch behavior
has been verified on hardware. The accepted standalone EGL smoke source and
artifact remain separate and unchanged.
