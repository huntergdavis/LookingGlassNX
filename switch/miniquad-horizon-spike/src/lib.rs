use macroquad::prelude::*;
use macroquad::Window;

fn run_title() {
    Window::from_config(
        Conf {
            window_title: "Looking GlassNX title/menu input spike".into(),
            ..Default::default()
        },
        async {
            let mut selection = 0usize;
            let mut show_controls = false;
            let mut start_gated = false;
            loop {
                clear_background(Color::from_rgba(14, 18, 28, 255));
                draw_text("Looking GlassNX", 48., 80., 36., WHITE);
                draw_text("Title and controller input path", 48., 116., 22., LIGHTGRAY);
                let down = horizon_pad_down();
                if show_controls {
                    draw_text("A: accept    B: back    +: exit", 48., 190., 22., WHITE);
                    if down & 0x2000 != 0 {
                        show_controls = false;
                    }
                } else if start_gated {
                    draw_text("Start is gated on user game data.", 32., 180., 22., WHITE);
                    draw_text("Press B to return.", 32., 220., 22., LIGHTGRAY);
                    if down & 0x2000 != 0 {
                        start_gated = false;
                    }
                } else {
                    draw_text(
                        if selection == 0 {
                            "> Start (placeholder)"
                        } else {
                            "  Start (placeholder)"
                        },
                        48.,
                        190.,
                        24.,
                        if selection == 0 { YELLOW } else { WHITE },
                    );
                    draw_text(
                        if selection == 1 {
                            "> Controller controls"
                        } else {
                            "  Controller controls"
                        },
                        48.,
                        230.,
                        24.,
                        if selection == 1 { YELLOW } else { WHITE },
                    );
                    if down & (1 | 2) != 0 {
                        selection = if down & 1 != 0 { 0 } else { 1 };
                    }
                    if down & 0x1000 != 0 {
                        if selection == 0 {
                            start_gated = true;
                        } else {
                            show_controls = true;
                        }
                    }
                }
                next_frame().await;
            }
        },
    );
}

extern "C" {
    fn miniquad_horizon_pad_down() -> u16;
}

fn horizon_pad_down() -> u16 {
    unsafe { miniquad_horizon_pad_down() }
}

#[no_mangle]
pub extern "C" fn lookingglass_horizon_entry() {
    run_title();
}
