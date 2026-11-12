mod native {
    #[derive(Debug)]
    pub enum Request {
        SetFullscreen(bool),
        ScheduleUpdate,
        SetCursorGrab(bool),
        ShowMouse(bool),
        SetMouseCursor(bool),
        SetWindowSize { new_width: u32, new_height: u32 },
        SetWindowPosition { new_x: u32, new_y: u32 },
        ShowKeyboard(bool),
    }

}

#[path = "../../../vendor/miniquad/src/native/horizon_requests.rs"]
mod horizon_requests;
