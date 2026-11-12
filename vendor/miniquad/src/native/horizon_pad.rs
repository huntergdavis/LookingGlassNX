//! Platform-independent libnx button mapping and its engine RawPad masks.

pub(crate) fn map_horizon_buttons(buttons: u64) -> u16 {
    // libnx HidNpadButton bit positions -> the engine's XInput-compatible RawPad.
    let pairs = [
        (0x1000, 0), (0x2000, 1), (0x4000, 2), (0x8000, 3),
        (0x0100, 6), (0x0200, 7), (0x0040, 4), (0x0080, 5),
        (0x0020, 11), // Minus is Back.
        (0x0010, 10), // Plus is Start.
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

pub(crate) fn map_horizon_triggers(buttons: u64) -> (u8, u8) {
    (if buttons & (1 << 8) != 0 { 255 } else { 0 },
     if buttons & (1 << 9) != 0 { 255 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::{map_horizon_buttons, map_horizon_triggers};

    #[test]
    fn maps_libnx_face_buttons_system_buttons_and_dpad_to_rawpad() {
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

    #[test]
    fn maps_zl_zr_to_full_scale_xinput_triggers() {
        assert_eq!(map_horizon_triggers(0), (0, 0));
        assert_eq!(map_horizon_triggers((1 << 8) | (1 << 9)), (255, 255));
    }
}
