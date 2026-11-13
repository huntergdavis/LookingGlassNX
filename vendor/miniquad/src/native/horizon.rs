//! Minimal libnx host runner. Native window/EGL lifetime remains in the C++ host.
use crate::{conf::Conf, event::EventHandler, native::NativeDisplayData};
use super::horizon_pad::{map_horizon_buttons, map_horizon_triggers};
use std::{cell::RefCell, ffi::{c_char, c_void}};

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
static NATIVE_REQUESTS: std::sync::Mutex<Option<std::sync::mpsc::Receiver<crate::native::Request>>> = std::sync::Mutex::new(None);

pub unsafe fn run<F>(_conf: Conf, f: F)
where F: 'static + FnOnce() -> Box<dyn EventHandler> {
    extern "C" { fn miniquad_horizon_attach(handler: *mut c_void) -> bool; }
    let callback: super::horizon_callback::HorizonCallback = Box::new(f);
    let raw = super::horizon_callback::into_opaque(callback);
    diag_static(b"rust:attach-call-entered\0");
    if !miniquad_horizon_attach(raw) {
        diag_static(b"rust:attach-call-rejected\0");
        panic!("Horizon host rejected Miniquad callback");
    }
    diag_static(b"rust:attach-call-returned\0");
}

extern "C" {
    fn lg_egl_proc(name: *const c_char) -> *mut c_void;
    fn lg_horizon_diag(event: *const c_char);
    fn lg_set_gl_viewport(width: i32, height: i32);
    fn lg_hid_buttons(index: u32, buttons: *mut u64) -> bool;
    fn lg_hid_axes(index: u32, lx: *mut i16, ly: *mut i16, rx: *mut i16, ry: *mut i16) -> bool;
    fn lg_hid_connected(index: u32) -> bool;
}

fn diag(event: &str) {
    if let Ok(event) = std::ffi::CString::new(event) {
        unsafe { lg_horizon_diag(event.as_ptr()); }
    }
}

pub(crate) fn diag_static(event: &'static [u8]) {
    if event.last() == Some(&0) {
        unsafe { lg_horizon_diag(event.as_ptr().cast()); }
    }
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
    diag("rust:miniquad-start-entered");
    if width <= 0 || height <= 0 { return false; }
    unsafe {
        let load = |name: &str| {
            let name = std::ffi::CString::new(name).unwrap();
            let raw = lg_egl_proc(name.as_ptr());
            if raw.is_null() { None } else { Some(std::mem::transmute::<*mut c_void, unsafe extern "C" fn()>(raw)) }
        };
        // Reject a partial GLES implementation before Miniquad stores null procs.
        for required in GLES3_CORE_FUNCTIONS {
            if load(required).is_none() {
                diag(&format!("exit:missing-gl-proc:{required}"));
                return false;
            }
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
    diag_static(b"rust:attach-entered\0");
    if handler.is_null() {
        diag_static(b"attach:handler-pointer-null\0");
        return false;
    }
    diag_static(b"attach:handler-pointer-valid\0");
    let callback = unsafe { super::horizon_callback::from_opaque(handler) };
    diag_static(b"attach:opaque-callback-reconstructed\0");
    HANDLER.with(|slot| {
        diag_static(b"attach:handler-tls-entered\0");
        let mut slot = slot.borrow_mut();
        diag_static(b"attach:handler-slot-borrowed\0");
        if slot.is_some() {
            diag_static(b"attach:handler-slot-already-set\0");
            return false;
        }
        diag_static(b"attach:before-callback-invoke\0");
        let handler = callback();
        diag_static(b"attach:callback-returned\0");
        *slot = Some(handler);
        diag_static(b"attach:handler-stored\0");
        drain_native_requests();
        diag_static(b"attach:native-requests-drained\0");
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
            static FIRST_FRAME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
            if FIRST_FRAME.swap(false, std::sync::atomic::Ordering::Relaxed) {
                diag("rust:first-frame-entered");
            }
            static HANDLER_FRAME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let handler_frame = HANDLER_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            diag_handler_frame(handler_frame, b"frame:2:handler:entered\0", b"frame:3:handler:entered\0");
            diag_handler_frame(handler_frame, b"frame:2:handler:update:before\0", b"frame:3:handler:update:before\0");
            handler.update();
            diag_handler_frame(handler_frame, b"frame:2:handler:update:returned\0", b"frame:3:handler:update:returned\0");
            drain_native_requests();
            diag_handler_frame(handler_frame, b"frame:2:handler:draw:before\0", b"frame:3:handler:draw:before\0");
            handler.draw();
            diag_handler_frame(handler_frame, b"frame:2:handler:draw:returned\0", b"frame:3:handler:draw:returned\0");
            drain_native_requests();
            diag_handler_frame(handler_frame, b"frame:2:handler:returned\0", b"frame:3:handler:returned\0");
        }
    });
}

fn drain_native_requests() {
    let requests = NATIVE_REQUESTS.lock().unwrap();
    let Some(receiver) = requests.as_ref() else { return; };
    crate::native::horizon_requests::drain(receiver);
}

fn diag_handler_frame(frame: usize, frame2: &'static [u8], frame3: &'static [u8]) {
    match frame {
        2 => diag_static(frame2),
        3 => diag_static(frame3),
        _ => {}
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
