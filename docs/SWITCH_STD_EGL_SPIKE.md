# Switch native Rust/GLES boot and input probe

**Status: cross-built and linked; not run on Switch hardware; not a playable
game.** This isolated probe uses a Rust static library and a small C++/libnx
host. It does not include Alice engine code, user game data, multiplayer, or a
map.

## Current candidate

The C++ host opens the libnx applet window, initializes EGL with an OpenGL ES 2
context, polls the standard controller through libnx, and calls Rust once per
frame. Rust calls GLES2 `glClearColor` and `glClear` directly from the linked
GLES2 library while EGL is current. Holding A selects green; the idle screen
is blue. Plus exits `appletMainLoop`. A C++ scope guard tears down the EGL
context, surface and display on normal exit and early returns after EGL
initialization.

This candidate deliberately does not instantiate Miniquad or Macroquad. The
prior Miniquad `GlContext::begin_default_pass` approach was reviewed and found
to require Miniquad's uninitialized `NATIVE_DISPLAY`; it is not a supported
host path here. Miniquad's GL context also requested vertex-array procedures
without evidence they are available from the GLES2 context. A future renderer
needs a Switch-aware backend that owns the native display and GLES version
contract.

The current frame path does not call `Path::components`, `getrandom`, CSRNG,
or a filesystem API. They are not part of this boot probe.

## Build evidence

Built with Rust nightly dated `nightly-2026-11-10` plus `rust-src` (reported
binary version `rustc 1.102.0-nightly (ac8310ba6 2026-11-09)`) and linked by
the cached devkitPro-derived image
`local-alice-switch-builder@sha256:885f305c1f942475bf1d37681da8b4aebfa6eea9e16360625474c35b7611d587`.
The Rust cross-build and native NRO link completed successfully on candidate
`codex/switch-boot-input-correction` at
`47c392842f016b53158084f5c8f11d9debf8e47c`. The NRO is:

```text
switch/std-egl-spike/src.nro
size:   5798085 bytes
sha256: 3ac7ac3f69651430ab207b0f9eabec42566fd5be6765cabae5e0cf4bcfbfd4cd
```

The ELF exports `lookingglass_render_frame` and contains linked `glClear` and
`glClearColor` implementations. The old `lookingglass_miniquad_*` and
`lookingglass_path_components_smoke` helpers are absent.

The static link includes Rust `std`, libnx, Mesa EGL/GLES, libdrm, and
devkitA64/Newlib runtime libraries. The artifact is for local review only: a
complete package-to-binary license inventory and corresponding notices for the
link closure have not been prepared. Do not distribute it until that audit is
complete.

## Rebuild

Run from `switch/std-egl-spike/` with an isolated patched nightly toolchain,
`rust-src`, and Cargo home. The checked-in patches are for this experiment;
they are not upstream fixes or a validated general Horizon runtime.

```sh
export SPIKE_TC=/path/to/isolated/nightly-2026-11-10-x86_64-unknown-linux-gnu
export CARGO_HOME=/path/to/isolated/cargo-home
export PATH="$SPIKE_TC/bin:$PATH"
export RUSTC="$SPIKE_TC/bin/rustc"
export RUSTC_BOOTSTRAP=1

patch -p1 -d "$SPIKE_TC/lib/rustlib/src/rust" \
  -i "$PWD/patches/rust-std-horizon.patch"
cargo fetch --locked
patch -p1 \
  -d "$CARGO_HOME/registry/src/index.crates.io-1949cf8c6b5b557f/libc-0.2.191" \
  -i "$PWD/patches/libc-0.2.191-aarch64-newlib.patch"
cargo build --locked --release

docker run --rm -u "$(id -u):$(id -g)" \
  -v "$PWD:/src" -w /src \
  local-alice-switch-builder@sha256:885f305c1f942475bf1d37681da8b4aebfa6eea9e16360625474c35b7611d587 \
  sh -lc 'export DEVKITPRO=/opt/devkitpro PATH=/opt/devkitpro/devkitA64/bin:/opt/devkitpro/tools/bin:/usr/bin:/bin; make -j4'
```

The installed toolchain and builder image in the original workspace are local
validation infrastructure, not project downloads. The Switch NRO has not been
run on hardware. Blue output, A-button color change, Plus exit, cleanup,
controller availability, frame pacing, memory use, and service behavior remain
unverified until a device run. No user game assets or proprietary data are
included.
