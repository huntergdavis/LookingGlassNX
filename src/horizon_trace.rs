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
macro_rules! horizon_trace_mark {
    ($msg:literal) => {};
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
