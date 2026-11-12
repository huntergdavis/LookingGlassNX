//! Minimal libnx host runner. Native window/EGL lifetime remains in the C++ host.
use crate::{conf::Conf, event::EventHandler, native::NativeDisplayData};
use super::horizon_pad::{map_horizon_buttons, map_horizon_triggers};
use std::{cell::RefCell, ffi::{c_char, c_void}};
use std::sync::mpsc::{Receiver, TryRecvError};

thread_local! {
    // Horizon's libnx loop and Miniquad callbacks run on the same main thread.
    static HANDLER: RefCell<Option<Box<dyn EventHandler>>> = RefCell::new(None);
}
static FUTURE_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PAD_CONNECTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PAD_STATE: std::sync::Mutex<RawPad> = std::sync::Mutex::new(RawPad {
    buttons: 0, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0,
});
// Keep the receiver alive for the lifetime of the display. Miniquad's public
// window APIs send through NativeDisplayData::native_requests and unwrap.
static NATIVE_REQUESTS: std::sync::Mutex<Option<Receiver<crate::native::Request>>> = std::sync::Mutex::new(None);

pub unsafe fn run<F>(_conf: Conf, f: F)
where F: 'static + FnOnce() -> Box<dyn EventHandler> {
    extern "C" { fn miniquad_horizon_attach(handler: *mut c_void) -> bool; }
    let raw = Box::into_raw(Box::new(f)) as *mut c_void;
    assert!(miniquad_horizon_attach(raw), "Horizon host rejected Miniquad callback");
}

extern "C" {
    fn lg_egl_proc(name: *const c_char) -> *mut c_void;
    fn lg_set_gl_viewport(width: i32, height: i32);
    fn lg_hid_buttons(index: u32, buttons: *mut u64) -> bool;
    fn lg_hid_axes(index: u32, lx: *mut i16, ly: *mut i16, rx: *mut i16, ry: *mut i16) -> bool;
    fn lg_hid_connected(index: u32) -> bool;
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct RawPad { pub buttons: u16, pub lt: u8, pub rt: u8, pub lx: i16, pub ly: i16, pub rx: i16, pub ry: i16 }
#[repr(C)]
#[derive(Default)]
pub struct RawState { pub packet: u32, pub pad: RawPad }

#[no_mangle]
pub extern "C" fn lookingglass_horizon_pad(index: u32, state: *mut RawState) -> bool {
    if index != 0 || state.is_null() { return false; }
    unsafe {
        if !lg_hid_connected(index) {
            PAD_CONNECTED.store(false, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        let mut buttons = 0u64;
        if !lg_hid_buttons(index, &mut buttons) {
            PAD_CONNECTED.store(false, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        let mut pad = RawPad::default();
        let mut axes = (0i16, 0i16, 0i16, 0i16);
        if !lg_hid_axes(index, &mut axes.0, &mut axes.1, &mut axes.2, &mut axes.3) {
            PAD_CONNECTED.store(false, std::sync::atomic::Ordering::Relaxed);
            return false;
        }
        pad.buttons = map_horizon_buttons(buttons);
        (pad.lt, pad.rt) = map_horizon_triggers(buttons);
        pad.lx = axes.0;
        pad.ly = axes.1;
        pad.rx = axes.2;
        pad.ry = axes.3;
        *PAD_STATE.lock().unwrap() = pad;
        PAD_CONNECTED.store(true, std::sync::atomic::Ordering::Relaxed);
        (*state).pad = pad;
        true
    }
}

#[no_mangle]
pub extern "C" fn miniquad_horizon_start(width: i32, height: i32) -> bool {
    if width <= 0 || height <= 0 { return false; }
    unsafe {
        let load = |name: &str| {
            let name = std::ffi::CString::new(name).unwrap();
            let raw = lg_egl_proc(name.as_ptr());
            if raw.is_null() { None } else { Some(std::mem::transmute::<*mut c_void, unsafe extern "C" fn()>(raw)) }
        };
        // Reject a partial GLES implementation before Miniquad stores null procs.
        for required in GLES3_CORE_FUNCTIONS {
            if load(required).is_none() { return false; }
        }
        crate::native::gl::load_gl_funcs(load);
        lg_set_gl_viewport(width, height);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    *NATIVE_REQUESTS.lock().unwrap() = Some(rx);
    crate::set_display(NativeDisplayData::new(
        width,
        height,
        tx,
        Box::new(NullClipboard),
    ));
    true
}

#[no_mangle]
pub extern "C" fn miniquad_horizon_attach(handler: *mut c_void) -> bool {
    if handler.is_null() { return false; }
    type Callback = Box<dyn FnOnce() -> Box<dyn EventHandler>>;
    let callback = unsafe { Box::from_raw(handler as *mut Callback) };
    HANDLER.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_some() { return false; }
        *slot = Some(callback());
        drain_native_requests();
        true
    })
}

#[no_mangle]
pub extern "C" fn lookingglass_horizon_exit() {
    if let Some(display) = crate::NATIVE_DISPLAY.get() { display.lock().unwrap().quit_ordered = true; }
}

#[no_mangle]
pub extern "C" fn miniquad_horizon_should_quit() -> bool {
    FUTURE_DONE.load(std::sync::atomic::Ordering::Relaxed)
}

#[no_mangle]
pub extern "C" fn miniquad_horizon_frame() {
    if FUTURE_DONE.load(std::sync::atomic::Ordering::Relaxed) { return; }
    // C host updates HID once before entering this frame.
    let mut state = RawState::default();
    if !lookingglass_horizon_pad(0, &mut state) { state.pad = RawPad::default(); }
    *PAD_STATE.lock().unwrap() = state.pad;
    if crate::native_display().lock().unwrap().quit_ordered {
        FUTURE_DONE.store(true, std::sync::atomic::Ordering::Relaxed);
        return;
    }
    drain_native_requests();
    HANDLER.with(|slot| {
        if let Some(handler) = slot.borrow_mut().as_mut() {
            handler.update();
            drain_native_requests();
            handler.draw();
            drain_native_requests();
        }
    });
}

fn drain_native_requests() {
    let requests = NATIVE_REQUESTS.lock().unwrap();
    let Some(receiver) = requests.as_ref() else { return; };
    consume_native_requests(receiver);
}

fn consume_native_requests(receiver: &Receiver<crate::native::Request>) {
    loop {
        match receiver.try_recv() {
            Ok(crate::native::Request::SetFullscreen(_)) => {},
            Ok(crate::native::Request::ScheduleUpdate
                | crate::native::Request::SetCursorGrab(_)
                | crate::native::Request::ShowMouse(_)
                | crate::native::Request::SetMouseCursor(_)
                | crate::native::Request::SetWindowSize { .. }
                | crate::native::Request::SetWindowPosition { .. }
                | crate::native::Request::ShowKeyboard(_)) => {},
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod native_request_tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn display_requests_remain_sendable_and_are_drained() {
        let (tx, rx) = mpsc::channel();
        tx.send(crate::native::Request::SetFullscreen(true)).unwrap();
        tx.send(crate::native::Request::SetWindowSize { new_width: 800, new_height: 600 }).unwrap();
        tx.send(crate::native::Request::ShowMouse(false)).unwrap();
        consume_native_requests(&rx);
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
        tx.send(crate::native::Request::SetCursorGrab(false)).unwrap();
        consume_native_requests(&rx);
    }
}

#[no_mangle]
pub extern "C" fn miniquad_horizon_readpad(state: *mut RawState) -> bool {
    if state.is_null() { return false; }
    let pad = *PAD_STATE.lock().unwrap();
    unsafe { (*state).pad = pad; }
    PAD_CONNECTED.load(std::sync::atomic::Ordering::Relaxed)
}

#[no_mangle]
pub extern "C" fn miniquad_horizon_stop() {
    HANDLER.with(|slot| { slot.borrow_mut().take(); });
    if let Some(display) = crate::NATIVE_DISPLAY.get() {
        display.lock().unwrap().quit_ordered = true;
    }
}

struct NullClipboard;
impl crate::native::Clipboard for NullClipboard {
    fn get(&mut self) -> Option<String> { None }
    fn set(&mut self, _: &str) {}
}

// This is the GLES 3.0 core set from the pinned devkitPro headers. The generated
// Miniquad loader also contains desktop-only / ES 3.2 timing entries; those are
// not required by this backend and are never used by the title/menu path.
const GLES3_CORE_FUNCTIONS: &[&str] = &[
    "glGetStringi",
    "glGetString",
    "glFramebufferTextureLayer",
    "glGenFramebuffers",
    "glBindFramebuffer",
    "glBindRenderbuffer",
    "glClearBufferfi",
    "glClearBufferfv",
    "glClearBufferuiv",
    "glDeleteRenderbuffers",
    "glUniform4fv",
    "glUniform3fv",
    "glUniform2fv",
    "glUniform1fv",
    "glUniform1iv",
    "glUniform2iv",
    "glUniform3iv",
    "glUniform4iv",
    "glUniform1i",
    "glUniform2i",
    "glUniform3i",
    "glUniform4i",
    "glUniform1f",
    "glUniform2f",
    "glUniform3f",
    "glUniform4f",
    "glUseProgram",
    "glShaderSource",
    "glLinkProgram",
    "glPixelStorei",
    "glGetUniformLocation",
    "glGetShaderiv",
    "glGetProgramInfoLog",
    "glGetAttribLocation",
    "glDisableVertexAttribArray",
    "glDeleteShader",
    "glDeleteProgram",
    "glCompileShader",
    "glStencilFuncSeparate",
    "glStencilOpSeparate",
    "glRenderbufferStorageMultisample",
    "glDrawBuffers",
    "glVertexAttribDivisor",
    "glBufferSubData",
    "glGenBuffers",
    "glCheckFramebufferStatus",
    "glFramebufferRenderbuffer",
    "glCompressedTexImage2D",
    "glCompressedTexImage3D",
    "glActiveTexture",
    "glTexSubImage3D",
    "glUniformMatrix2fv",
    "glUniformMatrix3fv",
    "glUniformMatrix4fv",
    "glRenderbufferStorage",
    "glPolygonOffset",
    "glDrawElements",
    "glDeleteFramebuffers",
    "glBlendEquationSeparate",
    "glDeleteTextures",
    "glGetProgramiv",
    "glBindTexture",
    "glTexImage3D",
    "glCreateShader",
    "glTexSubImage2D",
    "glCopyTexImage2D",
    "glClearDepthf",
    "glFramebufferTexture2D",
    "glCreateProgram",
    "glViewport",
    "glDeleteBuffers",
    "glDrawArrays",
    "glDrawElementsInstanced",
    "glVertexAttribPointer",
    "glVertexAttribIPointer",
    "glDisable",
    "glColorMask",
    "glBindBuffer",
    "glBindVertexArray",
    "glDeleteVertexArrays",
    "glDepthMask",
    "glDrawArraysInstanced",
    "glClearStencil",
    "glScissor",
    "glGenRenderbuffers",
    "glBufferData",
    "glBlendFuncSeparate",
    "glGenerateMipmap",
    "glTexParameteri",
    "glGetIntegerv",
    "glEnable",
    "glBlitFramebuffer",
    "glStencilMask",
    "glStencilMaskSeparate",
    "glAttachShader",
    "glDetachShader",
    "glGetError",
    "glClearColor",
    "glBlendColor",
    "glTexParameterf",
    "glTexParameterfv",
    "glGetShaderInfoLog",
    "glDepthFunc",
    "glStencilOp",
    "glStencilFunc",
    "glEnableVertexAttribArray",
    "glBlendFunc",
    "glReadBuffer",
    "glClear",
    "glTexImage2D",
    "glGenVertexArrays",
    "glFrontFace",
    "glCullFace",
    "glGenTextures",
    "glReadPixels",
    "glBeginQuery",
    "glDeleteQueries",
    "glEndQuery",
    "glGenQueries",
    "glFlush",
    "glFinish",
];
