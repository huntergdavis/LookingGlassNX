//! Minimal libnx host runner. Native window/EGL lifetime remains in the C++ host.
use crate::{conf::Conf, event::EventHandler, native::NativeDisplayData};
use std::{cell::RefCell, ffi::{c_char, c_void}};

thread_local! {
    // Horizon's libnx loop and Miniquad callbacks run on the same main thread.
    static HANDLER: RefCell<Option<Box<dyn EventHandler>>> = RefCell::new(None);
}
static FUTURE_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static PAD_STATE: std::sync::Mutex<RawPad> = std::sync::Mutex::new(RawPad {
    buttons: 0, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0,
});

pub unsafe fn run<F>(_conf: Conf, f: F)
where F: 'static + FnOnce() -> Box<dyn EventHandler> {
    extern "C" { fn miniquad_horizon_attach(handler: *mut c_void) -> bool; }
    let create_handler: Box<dyn FnOnce() -> Box<dyn EventHandler>> = Box::new(f);
    let raw = Box::into_raw(Box::new(create_handler)) as *mut c_void;
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
struct RawPad { buttons: u16, lt: u8, rt: u8, lx: i16, ly: i16, rx: i16, ry: i16 }
#[repr(C)]
#[derive(Default)]
pub struct RawState { packet: u32, pad: RawPad }

#[no_mangle]
pub extern "C" fn lookingglass_horizon_pad(index: u32, state: *mut RawState) -> bool {
    if index != 0 || state.is_null() { return false; }
    unsafe {
        if !lg_hid_connected(index) { return false; }
        let mut buttons = 0u64;
        if !lg_hid_buttons(index, &mut buttons) { return false; }
        let mut pad = RawPad::default();
        let mut axes = (0i16, 0i16, 0i16, 0i16);
        if !lg_hid_axes(index, &mut axes.0, &mut axes.1, &mut axes.2, &mut axes.3) { return false; }
        pad.buttons = map_horizon_buttons(buttons);
        if buttons & (1 << 8) != 0 { pad.lt = 255; }
        if buttons & (1 << 9) != 0 { pad.rt = 255; }
        pad.lx = axes.0;
        pad.ly = axes.1;
        pad.rx = axes.2;
        pad.ry = axes.3;
        *PAD_STATE.lock().unwrap() = pad;
        (*state).pad = pad;
        true
    }
}

fn map_horizon_buttons(buttons: u64) -> u16 {
    // libnx HidNpadButton bit positions -> the engine's XInput-compatible RawPad.
    let pairs = [
        (0x1000, 0), (0x2000, 1), (0x4000, 2), (0x8000, 3),
        (0x0100, 6), (0x0200, 7), (0x0040, 4), (0x0080, 5),
        (0x0020, 11), // Minus is Back.
        (0x0010, 10), // Plus is Start (host exits on its down edge).
    ];
    let mut out = 0;
    for (dst, src) in pairs {
        if buttons & (1u64 << src) != 0 { out |= dst; }
    }
    if buttons & (1 << 12) != 0 { out |= 4; }
    if buttons & (1 << 13) != 0 { out |= 1; }
    if buttons & (1 << 14) != 0 { out |= 8; }
    if buttons & (1 << 15) != 0 { out |= 2; }
    out
}

#[cfg(test)]
mod horizon_pad_tests {
    use super::map_horizon_buttons;

    #[test]
    fn maps_libnx_buttons_to_engine_rawpad_masks() {
        assert_eq!(map_horizon_buttons(1 << 0), 0x1000); // A/accept
        assert_eq!(map_horizon_buttons(1 << 1), 0x2000); // B/back
        assert_eq!(map_horizon_buttons(1 << 2), 0x4000); // X
        assert_eq!(map_horizon_buttons(1 << 3), 0x8000); // Y
        assert_eq!(map_horizon_buttons(1 << 10), 0x0010); // Plus/Start
        assert_eq!(map_horizon_buttons(1 << 11), 0x0020); // Minus/Back
        assert_eq!(map_horizon_buttons(1 << 12), 0x0004); // D-pad left
        assert_eq!(map_horizon_buttons(1 << 13), 0x0001); // D-pad up
        assert_eq!(map_horizon_buttons(1 << 14), 0x0008); // D-pad right
        assert_eq!(map_horizon_buttons(1 << 15), 0x0002); // D-pad down
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
    crate::set_display(NativeDisplayData::new(
        width,
        height,
        std::sync::mpsc::channel().0,
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
    HANDLER.with(|slot| {
        if let Some(handler) = slot.borrow_mut().as_mut() { handler.update(); handler.draw(); }
    });
}

#[no_mangle]
pub extern "C" fn miniquad_horizon_pad_buttons() -> u16 {
    PAD_STATE.lock().unwrap().buttons
}

#[no_mangle]
pub extern "C" fn miniquad_horizon_pad_down() -> u16 {
    static PREVIOUS: std::sync::Mutex<u16> = std::sync::Mutex::new(0);
    let current = PAD_STATE.lock().unwrap().buttons;
    let mut previous = PREVIOUS.lock().unwrap();
    let down = current & !*previous;
    *previous = current;
    down
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
