use crate::event::EventHandler;
use std::ffi::c_void;

pub(crate) type HorizonCallback = Box<dyn FnOnce() -> Box<dyn EventHandler>>;

pub(crate) fn into_opaque(callback: HorizonCallback) -> *mut c_void {
    // The outer Box gives the C ABI a thin pointer. The inner value is the
    // fat-pointer trait object and must be reconstructed as exactly this type.
    Box::into_raw(Box::new(callback)) as *mut c_void
}

pub(crate) unsafe fn from_opaque(raw: *mut c_void) -> HorizonCallback {
    *Box::from_raw(raw as *mut HorizonCallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct Handler;
    impl EventHandler for Handler {
        fn update(&mut self) {}
        fn draw(&mut self) {}
    }

    #[test]
    fn opaque_roundtrip_invokes_captured_callback() {
        let invoked = Rc::new(Cell::new(false));
        let captured = invoked.clone();
        let callback: HorizonCallback = Box::new(move || {
            captured.set(true);
            Box::new(Handler)
        });
        let raw = into_opaque(callback);
        let restored = unsafe { from_opaque(raw) };
        let _handler = restored();
        assert!(invoked.get());
    }
}
