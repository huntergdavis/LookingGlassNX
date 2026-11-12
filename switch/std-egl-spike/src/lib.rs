use std::{
    ffi::{c_char, c_void, CString},
    path::Path,
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

#[unsafe(no_mangle)]
pub extern "C" fn lookingglass_std_smoke() -> u32 {
    let path = Path::new("sdmc:/switch/looking-glass/");
    path.components().count() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn lookingglass_miniquad_smoke() {
    miniquad::gl::load_gl_funcs(load_gl_proc);
    let mut context = miniquad::GlContext::new();
    context.begin_default_pass(miniquad::PassAction::clear_color(0.08, 0.18, 0.38, 1.0));
    context.end_render_pass();
    context.commit_frame();
}
