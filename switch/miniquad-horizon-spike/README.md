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

On Horizon, set `LOOKING_GLASS_DATA_DIR` to a folder containing the original
2011 PC Alice `.pk3` archives; the default is
`sdmc:/switch/alice-nx/game-data`. The supplied `game_root` was found to contain
an Xbox 360 Alice: Madness Returns layout, which this loader cannot use and
does not modify. If the data folder cannot be opened, the NRO shows its path,
the required format, and the configurable override. Saves and settings use the
separate writable profile `sdmc:/switch/alice-nx/profile` (saves under its `saves/`
child). The NRO does not embed user assets.

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

Startup diagnostics are written best-effort to the first unused
`sdmc:/switch/alice-nx/startup-diagnostic-NNN.log` path (created with
exclusive-create; earlier logs are never truncated). Each event is flushed.
For the diagnostic build, creating the empty regular file
`sdmc:/switch/alice-nx/diagnostic-no-audio` before launch selects no-audio
startup mode. The selector persists across launches until manually removed;
audio remains enabled by default when it is absent. This path selector is
unverified on hardware.
The trace records the C host stages, EGL errors, first missing GL function,
Miniquad/Rust entry and callback stages, `GL_VERSION`, first frame/swap, and
the no-data screen's first submitted draw. The host clears and swaps once
before Miniquad initialization, then exits with code 12 if `GL_VERSION` is
still null. It requires a bounded printable-ASCII `OpenGL ES 3.*` version
string and exits with code 13 otherwise; it does not guess a version. Failure
to open or write the log does not change startup control flow. If no file is
present, that can mean startup never reached `main` or diagnostic-file creation
failed. Hardware startup diagnostic 003 for the reviewed `18245ed` NRO confirms
that the path was writable and markers through `rust:game-entry-entered` were
stored on one launch. It ends there; later events and storage behavior are not
verified. The exact diagnostic SHA256 is
`5f1bd427264619d1cf0a5021b97bc7d9f12b37bcad4454db32dba5692443d116`.

The first diagnostic run reached EGL setup and took the surface-size failure
path with logged values `0x0` and error `EGL_SUCCESS`; that older combined
check does not identify each query's individual result. The host now logs each
EGL dimension query and uses the same `NWindow`'s
`nwindowGetDimensions` result only when either EGL dimension is unavailable or
nonpositive. It validates the libnx result, positive dimensions, and `EGLint`
and `int` range after EGL surface creation, then checks the selected size
against `GL_MAX_VIEWPORT_DIMS`. Invalid limits, GL errors during the bootstrap
clear, or dimensions beyond the viewport limit are logged and fail startup.
This follows the Switch reference's native-window dimension query. This source
change has not yet been built or device-verified.

Horizon audio now has a source-only libnx `audout` path. It keeps the existing
44.1 kHz stereo mixer and capture monitor, linearly resamples in the audio
worker to DeviceOut's 48 kHz stereo Int16 format, and submits 10 ms blocks
through eight 0x1000-aligned buffers (80 ms total queue capacity). A
dedicated Rust worker owns audout and polls released buffers every 5 ms, so
render-thread stalls do not stop audio production. Startup reports ready only
after three blocks are submitted and audout starts, with a 2 second channel
timeout. Playback drop requests stop and polls for 50 short intervals before detaching a stuck
worker; scheduler delays can extend the elapsed time. A process-wide atomic
reservation is acquired before thread creation and released only after a
successful join and confirmed audout shutdown. It remains held after init
failure, timeout, panic, or detach for the rest of the process, so no later
worker can reuse a possibly live service or static slot.
Shutdown stops and flushes audout, checks that every descriptor has returned,
and leaves the service and static storage intact if ownership cannot be
confirmed. Scheduling delays longer than the bounded queue can still cause a
gap. The worker uses Rust's pthread-key TLS path validated by the earlier
standalone Switch TLS probe; worker lifecycle and audio output have not been
built or verified on the Switch. Physical audio output and underrun behavior
remain open device checks.

The current hardware diagnostic 003 ends at `rust:game-entry-entered`; it has
no game-entry accepted/error marker. The source now has an unbuilt candidate
that adds Horizon-only fixed boundaries after environment/arguments/CLI,
around archive discovery/open/index, map enumeration, and viewer-window attach.
Archive progress reports only capped ordinal and entry counts (first eight
packs, at most eight progress records per pack) without paths or filenames. A
missing marker only bounds the failure interval because SD logging is
best-effort. This candidate has not been built or reviewed; do not use it to
replace the installed NRO.

The host also provides Rust `std`'s two observed `sysconf` queries because
libnx/newlib does not link its `sysconf` implementation here: `_SC_PAGESIZE`
returns 4096 and `_SC_NPROCESSORS_ONLN` returns one worker conservatively.
Compile-time assertions pin the newlib query values from the immutable builder
headers (8 and 10); all other queries return `-1` with `EINVAL`. The shared
mapping/errno helper has a host test, but target ABI and runtime behavior have
not been exercised on hardware.

Horizon rejects co-op launch options, the co-op probe, the direct co-op
diagnostic options, and registered co-op level checks before the co-op
diagnostic pre-parser runs. Values consumed by ordinary options are skipped
when classifying flags. The viewer rejects co-op options at entry, its menu
action is inert, and the preview runner has a defensive guard. These gates
avoid entering co-op paths while native networking worker shutdown is not
proven to quiesce before EGL and CSRNG teardown. Desktop co-op behavior remains
enabled. This does not establish that every worker lifecycle is isolated or
safe to tear down.

Run commands from this directory so Cargo discovers `.cargo/config.toml`:

```sh
cargo check --manifest-path ../../Cargo.toml --lib --no-default-features --locked --offline
cargo build --manifest-path ../../Cargo.toml --lib --release --no-default-features --locked --offline
docker run --rm -v "$(pwd)/../..:/work" -w /work local-alice-switch-builder@sha256:885f305c1f942475bf1d37681da8b4aebfa6eea9e16360625474c35b7611d587 make -C /work/switch/miniquad-horizon-spike
```

Diagnostic (horizon-trace) builds add two things to that recipe, and both are easy to lose:

- build the Rust half with `--features horizon-trace`, or the markers vanish while the
  `rust:build:N` label still claims otherwise;
- pass `LG_STDERR_TO_LOG=1` to `make`, after deleting `build/`: `make` does not track `CFLAGS`,
  so a stale `build/main.o` silently produces a release-shaped NRO.

```sh
rm -rf build
cargo build --manifest-path ../../Cargo.toml --lib --release --no-default-features --features horizon-trace --locked --offline
docker run --rm -v "$(pwd)/../..:/work" -w /work local-alice-switch-builder@sha256:885f305c1f942475bf1d37681da8b4aebfa6eea9e16360625474c35b7611d587 make -C /work/switch/miniquad-horizon-spike LG_STDERR_TO_LOG=1
readelf -s switch/miniquad-horizon-spike/miniquad-horizon-spike.elf | grep -w dup2   # must be non-empty
```

The `dup2` check is what proves the define reached the C++ compile: release NROs have no `dup2`,
so an empty result means the stderr redirect, and the whole point of that build, is missing.

Host-side checks (from this directory, after the block above):

```sh
mkdir -p target/host
rustc --edition=2018 --test ../../vendor/miniquad/src/native/horizon_pad.rs -o target/host/horizon-pad-tests
target/host/horizon-pad-tests
g++ -std=c++17 -Wall -Wextra -Werror tests/sysconf_test.cpp -o target/host/sysconf-test
target/host/sysconf-test
rustc --edition=2021 --test tests/horizon_callback_harness.rs -o target/host/horizon-callback-tests
target/host/horizon-callback-tests
rustc --edition=2021 --test tests/horizon_requests_harness.rs -o target/host/horizon-request-tests
target/host/horizon-request-tests
```

The build uses pinned nightly `nightly-2026-11-10` with the locally patched
Horizon `std` source and the immutable devkitPro image
`local-alice-switch-builder@sha256:885f305c1f942475bf1d37681da8b4aebfa6eea9e16360625474c35b7611d587`.
The `--no-default-features` build disables co-op TLS; default desktop TLS stays
enabled, and this build rejects `wss://` co-op invites before opening a socket.

The pure button/trigger mapper has two host-executed tests. They validate the
libnx enum to RawPad conversion but do not validate physical controls on Switch.
The reviewed `18245ed` NRO was installed and launched on Switch. Diagnostic
003 records EGL setup, the native 1280x720 surface-size fallback, Miniquad
startup, and `rust:game-entry-entered`, then stops. The owner reported a blank
screen/crash. This does not establish that Rust `run()` returned, assets were
mounted, the menu rendered on hardware, or input/gameplay worked. Separately,
one Suyu emulator run showed a visible title/menu but sent no input; it is not
hardware evidence. Pause/resume and save/relaunch also remain unverified. The
accepted standalone EGL smoke source and artifact remain separate and unchanged.

## NRO application icon

The NRO uses the project's original AI-generated portal illustration as its application icon. The source image and derivative hashes are recorded in [`ICON_PROVENANCE.md`](ICON_PROVENANCE.md); `Makefile` pins the 256x256 JPEG with `ICON := icon.jpg`, which the devkitPro NRO rule passes to `--icon`. The source PNG is retained for provenance, not packed as game data or ROMFS. The icon has no third-party game characters or logos and is not an official game logo.
