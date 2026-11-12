mod event {
    pub trait EventHandler {
        fn update(&mut self);
        fn draw(&mut self);
    }
}

#[path = "../../../vendor/miniquad/src/native/horizon_callback.rs"]
mod horizon_callback;
