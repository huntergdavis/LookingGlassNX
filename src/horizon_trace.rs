//! Opt-in scope tracing for hardware diagnostics.
//!
//! Scopes log `rust:trace:enter:name` on entry and `rust:trace:exit:name` on exit. They
//! only do anything when the `horizon-trace` cargo feature is enabled and the target is
//! Horizon. Otherwise every macro here expands to nothing, so release builds are unchanged.
//!
//! Frame gating: `horizon_trace_frame!()` runs once at the start of each viewer frame. Scopes
//! log only while the frame count is at most FRAME_LIMIT. `horizon_trace_scope!` is for
//! frame-level functions. `horizon_trace_call!` is for per-call functions and also admits at
//! most PER_CALL_PER_FRAME entries per frame, so they cannot flood the card.

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
pub(crate) mod imp {
    use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

    /// Bytes of trace output allowed per process boot. Past this, lines are dropped and one
    /// 'budget exhausted' line is written.
    pub(crate) const BYTE_BUDGET: u64 = 8 * 1024 * 1024;
    static TRACE_BYTES: AtomicU64 = AtomicU64::new(0);
    static BUDGET_HIT: AtomicBool = AtomicBool::new(false);

    /// Every trace line goes through here, so the byte budget covers all of them.
    pub(crate) fn emit(line: &str) {
        let len = line.len() as u64;
        let used = TRACE_BYTES.fetch_add(len, Relaxed);
        if used + len <= BYTE_BUDGET {
            crate::horizon_diag(line);
        } else if !BUDGET_HIT.swap(true, Relaxed) {
            crate::horizon_diag("rust:trace:budget-exhausted");
        }
    }

    pub(crate) fn try_begin(label: &str) {
        emit(&format!("rust:try:begin:{label}"));
    }

    pub(crate) fn try_ok(label: &str) {
        emit(&format!("rust:try:ok:{label}"));
    }

    pub(crate) fn try_err(label: &str, err: &anyhow::Error) {
        emit(&format!(
            "rust:try:err:{label}:depth={}:{err:#}",
            err.chain().count()
        ));
    }


    /// Frames traced from the first viewer frame. Scope logging stops after this.
    pub(crate) const FRAME_LIMIT: u32 = 300;
    /// Per-call scope entries admitted in one frame.
    pub(crate) const PER_CALL_PER_FRAME: u32 = 8;

    static FRAME: AtomicU32 = AtomicU32::new(0);
    static PER_CALL: AtomicU32 = AtomicU32::new(0);

    pub(crate) fn tick_frame() {
        FRAME.fetch_add(1, Relaxed);
        PER_CALL.store(0, Relaxed);
    }

    /// Start a new trace window at each New Game, so the frame count starts there, not at boot.
    pub(crate) fn reset_window() {
        FRAME.store(0, Relaxed);
        PER_CALL.store(0, Relaxed);
    }

    fn in_window() -> bool {
        FRAME.load(Relaxed) <= FRAME_LIMIT
    }

    fn admit_per_call() -> bool {
        in_window() && PER_CALL.fetch_add(1, Relaxed) < PER_CALL_PER_FRAME
    }

    /// Log one fixed marker line.
    pub(crate) fn mark(msg: &str) {
        emit(msg);
    }

    const HEX: &[u8; 16] = b"0123456789abcdef";

    fn push(buf: &mut [u8; 160], n: &mut usize, bytes: &[u8]) {
        for &b in bytes {
            if *n + 1 < buf.len() {
                buf[*n] = b;
                *n += 1;
            }
        }
    }

    /// Log a marker with one numeric payload, built without touching the allocator.
    ///
    /// `format!` and `CString::new` both allocate, so a mark emitted through `emit` can be lost
    /// exactly when the heap is what failed. This one formats into a stack buffer and hands the
    /// NUL-terminated bytes straight to the C shim, so it still reports at the limit.
    pub(crate) fn mark_num(prefix: &str, label: &str, value: u64) {
        let mut buf = [0u8; 160];
        let mut n = 0usize;
        push(&mut buf, &mut n, b"rust:");
        push(&mut buf, &mut n, prefix.as_bytes());
        push(&mut buf, &mut n, b":");
        push(&mut buf, &mut n, label.as_bytes());
        push(&mut buf, &mut n, b":");
        let mut digits = [0u8; 16];
        let mut d = 0usize;
        let mut v = value;
        loop {
            digits[d] = HEX[(v & 0xf) as usize];
            d += 1;
            v >>= 4;
            if v == 0 {
                break;
            }
        }
        push(&mut buf, &mut n, b"0x");
        while d > 0 {
            d -= 1;
            push(&mut buf, &mut n, &digits[d..d + 1]);
        }
        buf[n] = 0;
        let len = (n + 1) as u64;
        let used = TRACE_BYTES.fetch_add(len, Relaxed);
        if used + len > BYTE_BUDGET {
            if !BUDGET_HIT.swap(true, Relaxed) {
                crate::horizon_diag("rust:trace:budget-exhausted");
            }
            return;
        }
        unsafe {
            extern "C" {
                fn lg_horizon_diag(event: *const std::ffi::c_char);
            }
            lg_horizon_diag(buf.as_ptr().cast());
        };
    }

    /// Heap break address from newlib (`_sbrk_r` with `incr = 0` is a pure read).
    ///
    /// The signal is the growth between marks, not the absolute value. `sbrk` and `_sbrk` are
    /// not linked on this target, so this asks `_sbrk_r`, which is the allocator actually in use.
    pub(crate) fn heap_now() -> u64 {
        unsafe {
            extern "C" {
                static _impure_ptr: *mut core::ffi::c_void;
                fn _sbrk_r(reent: *mut core::ffi::c_void, incr: isize) -> *mut core::ffi::c_void;
            }
            _sbrk_r(_impure_ptr, 0) as usize as u64
        }
    }

    /// While set, prop building logs its own internals, so the loot-art prop pair stops being
    /// one opaque step. Only loot art sets it, so other prop loads stay quiet.
    static PROP_TRACE: AtomicBool = AtomicBool::new(false);

    pub(crate) fn set_prop_trace(on: bool) {
        PROP_TRACE.store(on, Relaxed);
    }

    pub(crate) fn prop_trace() -> bool {
        PROP_TRACE.load(Relaxed)
    }

    /// Log an error with its full anyhow chain (`{:#}`) under a name.
    pub(crate) fn error<E: std::fmt::Display + ?Sized>(name: &str, e: &E) {
        emit(&format!("rust:trace:error:{name}:{e:#}"));
    }

    pub(crate) struct TraceScope {
        name: &'static str,
        active: bool,
    }

    impl TraceScope {
        pub(crate) fn frame_level(name: &'static str) -> Self {
            let active = in_window();
            if active {
                emit(&format!("rust:trace:enter:{name}"));
            }
            Self { name, active }
        }

        pub(crate) fn per_call(name: &'static str) -> Self {
            let active = admit_per_call();
            if active {
                emit(&format!("rust:trace:enter:{name}"));
            }
            Self { name, active }
        }
    }

    impl Drop for TraceScope {
        fn drop(&mut self) {
            if self.active {
                emit(&format!("rust:trace:exit:{}", self.name));
            }
        }
    }
}

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_trace_scope {
    ($name:literal) => {
        let _horizon_trace_scope = crate::horizon_trace::imp::TraceScope::frame_level($name);
    };
}

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_trace_call {
    ($name:literal) => {
        let _horizon_trace_call = crate::horizon_trace::imp::TraceScope::per_call($name);
    };
}

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_trace_mark {
    ($msg:literal) => {
        crate::horizon_trace::imp::mark($msg);
    };
}

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_trace_error {
    ($name:literal, $e:expr) => {
        crate::horizon_trace::imp::error($name, &$e)
    };
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
#[allow(unused_macros)]
macro_rules! horizon_trace_mark {
    ($msg:literal) => {};
}

/// Heap watermark straight from `_sbrk(0)`, logged without allocating.
#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_heap_mark {
    ($label:literal) => {
        crate::horizon_trace::imp::mark_num("heap", $label, crate::horizon_trace::imp::heap_now());
    };
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
#[allow(unused_macros)]
macro_rules! horizon_heap_mark {
    ($label:literal) => {};
}

/// `horizon_step_mark!("label", index)` names which item a loop is on, without allocating.
#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_step_mark {
    ($label:literal, $value:expr) => {
        crate::horizon_trace::imp::mark_num("step", $label, $value as u64);
    };
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
#[allow(unused_macros)]
macro_rules! horizon_step_mark {
    ($label:literal, $value:expr) => {{
        let _ = $value;
    }};
}

/// Turn prop-internal logging on or off.
#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_prop_trace {
    ($on:expr) => {
        crate::horizon_trace::imp::set_prop_trace($on);
    };
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
#[allow(unused_macros)]
macro_rules! horizon_prop_trace {
    ($on:expr) => {{
        let _ = $on;
    }};
}

/// Whether prop-internal logging is currently on. Constant `false` when tracing is off.
#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_prop_tracing {
    () => {
        crate::horizon_trace::imp::prop_trace()
    };
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
#[allow(unused_macros)]
macro_rules! horizon_prop_tracing {
    () => {
        false
    };
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
#[allow(unused_macros)]
macro_rules! horizon_trace_error {
    ($name:literal, $e:expr) => {{
        let _ = &$e;
    }};
}

/// `horizon_try!(expr, "label")` yields the Ok value of a fallible expression. On Horizon with
/// the feature it logs begin, then ok, or err with the anyhow chain and depth, and returns the
/// same error (converted with From, as `?` does). Off, it is exactly `expr?`.
#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_try {
    ($e:expr, $label:expr) => {{
        let label = $label;
        crate::horizon_trace::imp::try_begin(label);
        match $e {
            Ok(value) => {
                crate::horizon_trace::imp::try_ok(label);
                value
            }
            Err(err) => {
                crate::horizon_trace::imp::try_err(label, &err);
                return Err(From::from(err));
            }
        }
    }};
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
macro_rules! horizon_try {
    ($e:expr, $label:expr) => {
        $e?
    };
}

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_trace_frame {
    () => {
        crate::horizon_trace::imp::tick_frame();
    };
}

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_trace_load_budget {
    () => {
        crate::horizon_trace::imp::reset_window();
    };
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
macro_rules! horizon_trace_load_budget {
    () => {};
}

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_trace_new_game {
    () => {
        crate::horizon_trace::imp::reset_window();
    };
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
macro_rules! horizon_trace_new_game {
    () => {};
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
macro_rules! horizon_trace_scope {
    ($name:literal) => {};
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
macro_rules! horizon_trace_call {
    ($name:literal) => {};
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
macro_rules! horizon_trace_frame {
    () => {};
}
