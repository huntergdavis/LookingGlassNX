# Switch native Rust/GLES boot and input probe

**Status: cross-built and linked; not run on Switch hardware; not a playable
game.** This isolated probe uses a Rust static library and a small C++/libnx
host. It does not include Alice engine code, user game data, multiplayer, or a
map.

## Current candidate

The C++ host opens the libnx applet window, initializes EGL with an OpenGL ES 2
context, queries the EGL surface size and sets the GLES2 viewport, then polls
the standard controller through libnx. Each frame it asks a Rust helper to
select blue or green RGBA, performs GLES2 `glClearColor`/`glClear` in the C++
host, checks GL errors, and swaps with `eglSwapBuffers`. Holding A selects
green; releasing A returns to blue. Plus exits `appletMainLoop`. A C++ scope
guard tears down the EGL context, surface and display on normal exit and early
returns after EGL initialization.

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
`8c8407b31127d08d0c0d63fabf6fdcaa165cefef`. The NRO is:

```text
switch/std-egl-spike/src.nro
size:   5798085 bytes
sha256: 597f95c6230003119d39a98f393c21bc9cc630f3f56db08446f7c4c798dc0a88
```

The ELF exports `lookingglass_boot_color` and contains linked `eglQuerySurface`,
`eglSwapBuffers`, `glViewport`, `glClearColor`, `glClear`, and `glGetError`
implementations. The old `lookingglass_miniquad_*`,
`lookingglass_path_components_smoke`, and `getrandom` helpers are absent.

The static link uses Rust `std`, libnx, Mesa EGL/GLES, and devkitA64/Newlib
runtime libraries. The link command also supplies glapi and Nouveau DRM
archives; the exact retained archive closure has not been inventoried. The
artifact is for local review only: a complete package-to-binary license
inventory and corresponding notices have not been prepared. Do not distribute
it until that audit is complete.

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
# The app lockfile has no crate dependencies. Fetch build-std's locked sources
# from the Rust library workspace so libc 0.2.191 is present for the patch.
cargo fetch --manifest-path "$SPIKE_TC/lib/rustlib/src/rust/library/Cargo.toml" --locked
patch -p1 \
  -d "$CARGO_HOME/registry/src/index.crates.io-1949cf8c6b5b557f/libc-0.2.191" \
  -i "$PWD/patches/libc-0.2.191-aarch64-newlib.patch"
cargo build --locked --release

docker run --rm -u "$(id -u):$(id -g)" \
  -v "$PWD:/src" -w /src \
  local-alice-switch-builder@sha256:885f305c1f942475bf1d37681da8b4aebfa6eea9e16360625474c35b7611d587 \
  sh -lc 'export DEVKITPRO=/opt/devkitpro PATH=/opt/devkitpro/devkitA64/bin:/opt/devkitpro/tools/bin:/usr/bin:/bin; make -j4'
```

That is the release recipe. A diagnostic build (`--features horizon-trace` on the cargo side)
must also pass `LG_STDERR_TO_LOG=1` to `make` and delete `build/` first, since `make` does not
track `CFLAGS` and a stale `build/main.o` yields a release-shaped NRO. Confirm with
`readelf -s miniquad-horizon-spike.elf | grep -w dup2`; a release NRO has no `dup2`, so an empty
result means stderr was not redirected and the runtime's own allocation message will be lost.

The installed toolchain and builder image in the original workspace are local
validation infrastructure, not project downloads. The Switch NRO has not been
run on hardware. Blue output, A-button color change, Plus exit, cleanup,
controller availability, frame pacing, memory use, and service behavior remain
unverified until a device run. No user game assets or proprietary data are
included.
