//! Opt-in scope tracing for hardware diagnostics.
//!
//! `horizon_trace_scope!("name")` logs `rust:trace:enter:name` on entry and
//! `rust:trace:exit:name` when the enclosing scope ends. It only does anything when the
//! `horizon-trace` cargo feature is enabled and the target is Horizon. Otherwise it expands
//! to nothing, so release builds are unchanged.

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
pub(crate) struct TraceScope(&'static str);

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
impl TraceScope {
    pub(crate) fn enter(name: &'static str) -> Self {
        crate::horizon_diag(&format!("rust:trace:enter:{name}"));
        Self(name)
    }
}

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
impl Drop for TraceScope {
    fn drop(&mut self) {
        crate::horizon_diag(&format!("rust:trace:exit:{}", self.0));
    }
}

#[cfg(all(feature = "horizon-trace", target_os = "horizon"))]
macro_rules! horizon_trace_scope {
    ($name:literal) => {
        let _horizon_trace_scope = crate::horizon_trace::TraceScope::enter($name);
    };
}

#[cfg(not(all(feature = "horizon-trace", target_os = "horizon")))]
macro_rules! horizon_trace_scope {
    ($name:literal) => {};
}
