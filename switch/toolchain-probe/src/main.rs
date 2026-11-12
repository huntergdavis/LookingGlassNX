#![no_std]
#![no_main]

#[macro_use]
extern crate nx;

extern crate alloc;

use nx::result::*;
use nx::util;
use nx::svc;
use nx::diag::abort;
use nx::diag::log;
use nx::{applet, console, gpu, input, sync::RwLock};
use nx::service::hid;
use alloc::sync::Arc;
use core::num::NonZeroU16;

use core::panic;

#[no_mangle]
pub fn initialize_heap(hbl_heap: util::PointerAndSize) -> util::PointerAndSize {
    if hbl_heap.is_valid() {
        hbl_heap
    }
    else {
        let heap_size: usize = 0x10000000;
        let heap_address = svc::set_heap_size(heap_size).unwrap();
        util::PointerAndSize::new(heap_address, heap_size)
    }
}

#[no_mangle]
pub fn main() -> Result<()> {
    diag_log!(log::lm::LmLogger { log::LogSeverity::Trace, false } => "Hello world!");

    applet::initialize()?;
    let supported = hid::NpadStyleTag::FullKey()
        | hid::NpadStyleTag::Handheld()
        | hid::NpadStyleTag::JoyDual()
        | hid::NpadStyleTag::JoyLeft()
        | hid::NpadStyleTag::JoyRight();
    let input = input::Context::new(supported, 1)?;
    let mut player = input.get_player(hid::NpadIdType::No1);

    let graphics = Arc::new(RwLock::new(gpu::Context::new(
        gpu::NvDrvServiceKind::Application,
        gpu::ViServiceKind::Application,
        0x1000000,
    )?));
    let mut screen = console::scrollback::ScrollbackConsole::new(
        graphics,
        8,
        NonZeroU16::new(72).unwrap(),
        false,
        None,
        2,
    )?;
    screen.write("Looking Glass Horizon probe\n\
        Rust no_std + cargo-nx NRO\n\
        Display path: nx GPU canvas\n\
        Input path: HID controller\n\
        Press + to exit\n");

    loop {
        if player.get_buttons().contains(hid::NpadButton::Plus()) {
            break;
        }
        screen.draw()?;
        screen.wait_vsync_event(None)?;
    }

    Ok(())
}

#[panic_handler]
fn panic_handler(info: &panic::PanicInfo) -> ! {
    util::simple_panic_handler::<log::lm::LmLogger>(info, abort::AbortLevel::FatalThrow())
}
