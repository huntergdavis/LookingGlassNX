unsafe extern "C" {
    fn glClearColor(red: f32, green: f32, blue: f32, alpha: f32);
    fn glClear(mask: u32);
}

/// Draw one GLES2 clear frame. The libnx host owns and serializes the EGL
/// context and calls this only while that context is current.
#[unsafe(no_mangle)]
pub extern "C" fn lookingglass_render_frame(buttons: u32) {
    let color = if buttons & 1 != 0 {
        (0.05, 0.48, 0.18, 1.0)
    } else {
        (0.08, 0.18, 0.42, 1.0)
    };
    unsafe {
        glClearColor(color.0, color.1, color.2, color.3);
        glClear(0x0000_4000); // GL_COLOR_BUFFER_BIT
    }
}
