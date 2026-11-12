# Switch Rust std/newlib and EGL/Miniquad host spike

**Status: cross-built and linked as a feasibility spike; not device-tested and
not a playable game.** This work is on branch `codex/switch-std-egl-spike`,
based on the accepted probe revision `2053f91821ada269fb005b0eccf79165b66a36d5`.
The original probe worktree and commit remain unchanged.

## What built

`switch/std-egl-spike` combines a minimal Rust `staticlib` with a C++/libnx
application. The C++ side creates a libnx window, EGL display/surface and GLES2
context, then obtains GL function pointers through `eglGetProcAddress`. Rust
owns one Miniquad 0.4.8 `GlContext` for the life of the loop and draws a frame
after each controller poll. Holding A changes the display from blue to green;
Plus exits the applet loop. The Rust library also calls
`std::path::Path::components()` to exercise a `std` API and uses `getrandom`
through the Switch CSRNG service. It does not open or stat a filesystem path.

The C++ `Runtime` guard releases the Miniquad context, current EGL state,
context, surface, display and CSRNG service on normal exit and all early-return
paths after initialization. Multiplayer and the Alice map/engine are not
started by this milestone.

The pinned nightly Rust toolchain cross-built the Rust static library. The
cached local builder image `local-alice-switch-builder@sha256:885f305c1f942475bf1d37681da8b4aebfa6eea9e16360625474c35b7611d587`
then linked it with devkitPro/libnx, switch-mesa EGL, GLES2, and produced:

```text
switch/std-egl-spike/src.nro
size:   6129861 bytes
sha256: c0b475615904e8b85f5d1151307d6f308875e4be25114ca736b3717424e1bb00
```

The current boot/input candidate NRO is:

```text
switch/std-egl-spike/src.nro
size:   6129861 bytes
sha256: fd83eb8c0f78fb7a44d8dd1d9bf0136d74bdda188e6919caceca10bb94538ce5
```

Its ELF symbol table contains `lookingglass_miniquad_init`,
`lookingglass_miniquad_frame`, `lookingglass_miniquad_exit`, and the `getrandom`
shim. This demonstrates a cross-build and link path for the Rust `std` library
and a persistent Miniquad GL context under externally-created EGL. It is not
yet a demonstrated hardware boot: the NRO has not run on a Switch, so display,
controller input, Plus exit, and resource cleanup are still source/link claims
awaiting a device run.

## Rebuilding this spike

Run these commands from `switch/std-egl-spike/`. Start with writable, isolated
copies of Rust nightly `nightly-2026-11-10` plus `rust-src` and Cargo home; do
not patch the shared installed toolchain or Cargo cache. The builder image
identified below must also be available. Patch `rust-src`, fetch the locked
crate sources into the isolated Cargo home, then patch the extracted `libc`
crate before compiling:

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
cargo clean
cargo build --locked --release

docker run --rm -u "$(id -u):$(id -g)" \
  -v "$PWD:/src" -w /src \
  local-alice-switch-builder@sha256:885f305c1f942475bf1d37681da8b4aebfa6eea9e16360625474c35b7611d587 \
  sh -lc 'make clean && make -j4'
```

In this checkout's local validation setup, the isolated paths are
`../../.spike/tooling/rustup/toolchains/nightly-2026-11-10-x86_64-unknown-linux-gnu`
and `../../.spike/cargo`, relative to this directory.

The local builder image is a cached devkitPro-derived image, not an image
published by this project. Rebuilding on a clean host also requires making
that exact image available there. The Makefile declares the Rust archive as
an ELF prerequisite so a rebuilt archive relinks the host without `make clean`.

This is a local feasibility build, not a redistributable NRO. Its static link
includes the Rust standard library and crate closure, libnx, Mesa EGL/GLES and
libdrm, and devkitA64/newlib runtime libraries. Package metadata identifies
some licenses, but a complete package-to-binary inventory and corresponding
notices for this link closure have not been prepared. Audit that closure and
resolve any missing notices before distributing an NRO.

## Compatibility changes needed by this toolchain

The official freestanding target reports `target_os="horizon"`, an empty
`target_env`, and does not declare a Unix target family. For this experiment,
the Cargo target flags add `target_family="unix"`, `target_env="newlib"`,
`unix`, thread-local code generation, and the `getrandom_backend="custom"`
selection. The Rust `std/build.rs` source needed a local Horizon allowlist line
to avoid producing `restricted_std`; after changing `rust-src`, a full
`cargo clean` was required before rebuilding `std`.

That exposed a second gap in the cached `libc 0.2.191`: its Horizon Newlib
module had AArch64 layouts for `stat` but rejected AArch64 when defining
`dev_t`, `ino_t`, and `off_t`. The spike patch adds `short`, `unsigned short`,
and `long`, respectively, matching the devkitA64 Newlib headers in the cached
builder image. The patch is included at
[`switch/std-egl-spike/patches/libc-0.2.191-aarch64-newlib.patch`](../switch/std-egl-spike/patches/libc-0.2.191-aarch64-newlib.patch).
The Rust standard-library patch is beside it. These are local compatibility
patches for this experiment, not upstream fixes or a validated general runtime.

Linking also exposed a reference to C `getrandom` from Rust `std`'s
`HashMap` random-state code. The C++ host supplies that symbol by forwarding to
libnx `csrngGetRandomBytes`; it initializes and exits the CSRNG service around
the smoke app. A production host still needs review of startup order, error
handling, entropy behavior, and all other libc/std dependencies.

## Miniquad boundary and remaining risks

Miniquad 0.4.8's GL renderer can be constructed after the host makes an EGL
context current, as shown by the linked smoke call. Its `start()` function has
no Horizon branch, and the crate does not provide a Switch window, event loop,
or controller backend. The game would need a maintained Horizon host/backend
that drives frame timing and maps libnx input into the existing callbacks.
Macroquad integration is not implemented here.

The `std` call above is `Path::components()` only; filesystem reads, writes,
metadata, and SD path behavior have not been tested. The Switch probe's
Plus-button loop and this smoke host's Plus/B loop are also independent of the
Alice input model.

FlashNX documents a similar Rust `staticlib` + C++/libnx + newlib/EGL/GLES
architecture, but it is a precedent rather than a drop-in: its Rust source
patches, allocator, TLS workarounds, and renderer are project-specific. See
[FlashNX development notes](https://github.com/Jonathan8520/FlashNX/blob/main/DEVELOPMENT.md).

The NRO has not been run on Switch hardware. Display output, pad input, Plus
exit, CSRNG service behavior, `std` filesystem paths, thread-local state,
threads, audio, memory use, and frame timing remain unverified. No Alice code
or user game data is included. This is a boot/input artifact for a device
milestone review, not the first-playable game; that acceptance remains in the
separate device checklist in [`SWITCH_PORT.md`](SWITCH_PORT.md).
