use std::{
    ffi::{c_char, c_void, CString},
    path::Path,
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};
use miniquad::RenderingBackend;

unsafe extern "C" {
    fn lookingglass_get_proc_address(name: *const c_char) -> *const c_void;
    fn lookingglass_fill_random(dest: *mut u8, len: usize) -> i32;
}

#[unsafe(no_mangle)]
pub unsafe extern "Rust" fn __getrandom_v03_custom(
    dest: *mut u8,
    len: usize,
) -> Result<(), getrandom::Error> {
    if unsafe { lookingglass_fill_random(dest, len) } == 0 {
        Ok(())
    } else {
        Err(getrandom::Error::UNSUPPORTED)
    }
}

fn load_gl_proc(name: &str) -> Option<unsafe extern "C" fn()> {
    let name = CString::new(name).ok()?;
    let address = unsafe { lookingglass_get_proc_address(name.as_ptr()) };
    if address.is_null() {
        None
    } else {
        Some(unsafe { std::mem::transmute(address) })
    }
}

static CONTEXT: AtomicPtr<miniquad::GlContext> = AtomicPtr::new(ptr::null_mut());

#[unsafe(no_mangle)]
pub extern "C" fn lookingglass_path_components_smoke() -> u32 {
    let path = Path::new("sdmc:/switch/looking-glass/");
    path.components().count() as u32
}

#[unsafe(no_mangle)]
/// Initialize the single-threaded EGL/Miniquad boot renderer.
pub extern "C" fn lookingglass_miniquad_init() -> bool {
    if !CONTEXT.load(Ordering::Acquire).is_null() {
        return false;
    }
    miniquad::gl::load_gl_funcs(load_gl_proc);
    let context = Box::into_raw(Box::new(miniquad::GlContext::new()));
    if CONTEXT
        .compare_exchange(ptr::null_mut(), context, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        unsafe { drop(Box::from_raw(context)); }
        return false;
    }
    true
}

#[unsafe(no_mangle)]
/// Draw one frame; the C/libnx host serializes this with init and exit.
pub extern "C" fn lookingglass_miniquad_frame(buttons: u64) -> bool {
    let context = CONTEXT.load(Ordering::Acquire);
    let Some(context) = (unsafe { context.as_mut() }) else {
        return false;
    };
    // Holding A makes controller input visible on screen; the normal blue
    // screen means the applet and EGL frame loop are still advancing.
    let color = if buttons & 1 != 0 {
        (0.05, 0.48, 0.18, 1.0)
    } else {
        (0.08, 0.18, 0.42, 1.0)
    };
    context.begin_default_pass(miniquad::PassAction::clear_color(color.0, color.1, color.2, color.3));
    context.end_render_pass();
    context.commit_frame();
    true
}

#[unsafe(no_mangle)]
/// Release the renderer while the host EGL context is still current.
pub extern "C" fn lookingglass_miniquad_exit() {
    let context = CONTEXT.swap(ptr::null_mut(), Ordering::AcqRel);
    if !context.is_null() {
        unsafe { drop(Box::from_raw(context)); }
    }
}
