use crate::native::Request;
use std::sync::mpsc::{Receiver, TryRecvError};

pub(crate) fn drain(receiver: &Receiver<Request>) {
    loop {
        match receiver.try_recv() {
            Ok(Request::SetFullscreen(_)) => {
                // The Horizon host owns a fixed fullscreen EGL surface.
            }
            Ok(Request::ScheduleUpdate
                | Request::SetCursorGrab(_)
                | Request::ShowMouse(_)
                | Request::SetMouseCursor(_)
                | Request::SetWindowSize { .. }
                | Request::SetWindowPosition { .. }
                | Request::ShowKeyboard(_)) => {
                // No window manager, mouse, or software keyboard is exposed.
                // Consume these so Miniquad send(...).unwrap() calls are safe.
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn public_display_requests_stay_connected_and_are_drained() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Request::SetFullscreen(true)).unwrap();
        sender.send(Request::SetWindowSize { new_width: 800, new_height: 600 }).unwrap();
        sender.send(Request::ShowMouse(false)).unwrap();
        drain(&receiver);
        assert!(matches!(receiver.try_recv(), Err(TryRecvError::Empty)));
        sender.send(Request::SetCursorGrab(false)).unwrap();
        drain(&receiver);
    }
}
