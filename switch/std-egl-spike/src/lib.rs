/// Fill the RGBA clear color from the controller state supplied by the host.
///
/// # Safety
/// `rgba` must point to four writable `f32` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lookingglass_boot_color(buttons: u32, rgba: *mut f32) -> bool {
    if rgba.is_null() {
        return false;
    }
    let color = if buttons & 1 != 0 {
        (0.05, 0.48, 0.18, 1.0)
    } else {
        (0.08, 0.18, 0.42, 1.0)
    };
    unsafe { rgba.copy_from_nonoverlapping([color.0, color.1, color.2, color.3].as_ptr(), 4); }
    true
}
