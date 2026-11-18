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
    use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

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

    pub(crate) struct TraceScope {
        name: &'static str,
        active: bool,
    }

    impl TraceScope {
        pub(crate) fn frame_level(name: &'static str) -> Self {
            let active = in_window();
            if active {
                crate::horizon_diag(&format!("rust:trace:enter:{name}"));
            }
            Self { name, active }
        }

        pub(crate) fn per_call(name: &'static str) -> Self {
            let active = admit_per_call();
            if active {
                crate::horizon_diag(&format!("rust:trace:enter:{name}"));
            }
            Self { name, active }
        }
    }

    impl Drop for TraceScope {
        fn drop(&mut self) {
            if self.active {
                crate::horizon_diag(&format!("rust:trace:exit:{}", self.name));
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
