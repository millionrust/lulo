//! Development-only input injector for the rmac reference session.
//!
//! niri advertises `zwlr_virtual_pointer_manager_v1`, so this probe can move
//! and click the pointer for interactive verification without a physical
//! device. It is a probe, never part of the product packages.
//!
//! Usage: input-probe click <x> <y> | move <x> <y> | press <x> <y>

use std::time::{SystemTime, UNIX_EPOCH};

use wayland_client::{
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_pointer, wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::{self, ZwlrVirtualPointerManagerV1},
    zwlr_virtual_pointer_v1::{self, ZwlrVirtualPointerV1},
};

const OUTPUT_WIDTH: u32 = 1920;
const OUTPUT_HEIGHT: u32 = 1080;
const BTN_LEFT: u32 = 0x110;

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwlrVirtualPointerManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "zwlr_virtual_pointer_manager_v1" => {
                    state.manager =
                        Some(registry.bind::<ZwlrVirtualPointerManagerV1, _, _>(name, 2, qh, ()));
                }
                "wl_seat" => {
                    state.seat =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(9), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrVirtualPointerManagerV1,
        _: zwlr_virtual_pointer_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrVirtualPointerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrVirtualPointerV1,
        event: zwlr_virtual_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let _ = (state, event);
    }
}

fn now_millis() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u32)
        .unwrap_or(0)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (command, x, y) = match args.get(1).map(String::as_str) {
        Some("click") if args.len() == 4 => {
            ("click", args[2].parse::<u32>()?, args[3].parse::<u32>()?)
        }
        Some("press") if args.len() == 4 => {
            ("press", args[2].parse::<u32>()?, args[3].parse::<u32>()?)
        }
        Some("move") if args.len() == 4 => {
            ("move", args[2].parse::<u32>()?, args[3].parse::<u32>()?)
        }
        _ => {
            eprintln!("usage: input-probe click|press|move <x> <y>");
            std::process::exit(2);
        }
    };

    let connection = Connection::connect_to_env()?;
    let (globals, mut queue) = registry_queue_init::<State>(&connection)?;
    let qh = queue.handle();
    let mut state = State::default();
    // Dispatches the global events into `state`.
    let _ = &globals;
    queue.roundtrip(&mut state)?;

    let manager = state
        .manager
        .clone()
        .ok_or("niri did not advertise zwlr_virtual_pointer_manager_v1")?;
    let pointer = manager.create_virtual_pointer(state.seat.as_ref(), &qh, ());
    queue.roundtrip(&mut state)?;

    let time = now_millis();
    pointer.motion_absolute(time, x, y, OUTPUT_WIDTH, OUTPUT_HEIGHT);
    pointer.frame();
    queue.flush()?;
    std::thread::sleep(std::time::Duration::from_millis(80));

    match command {
        "click" => {
            pointer.button(now_millis(), BTN_LEFT, wl_pointer::ButtonState::Pressed);
            pointer.frame();
            queue.flush()?;
            std::thread::sleep(std::time::Duration::from_millis(60));
            pointer.button(now_millis(), BTN_LEFT, wl_pointer::ButtonState::Released);
            pointer.frame();
        }
        "press" => {
            pointer.button(now_millis(), BTN_LEFT, wl_pointer::ButtonState::Pressed);
            pointer.frame();
        }
        "move" => {}
        _ => unreachable!(),
    }
    queue.flush()?;
    std::thread::sleep(std::time::Duration::from_millis(120));
    Ok(())
}
