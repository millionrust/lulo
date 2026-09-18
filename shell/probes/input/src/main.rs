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
    protocol::{wl_pointer, wl_registry},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::{self, ZwlrVirtualPointerManagerV1},
    zwlr_virtual_pointer_v1::{self, ZwlrVirtualPointerV1},
};

const OUTPUT_WIDTH: u32 = 1920;
const OUTPUT_HEIGHT: u32 = 1080;
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;

#[derive(Default)]
struct State;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
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
        _: &mut Self,
        _: &ZwlrVirtualPointerV1,
        _: zwlr_virtual_pointer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
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
    let command = args.get(1).map(String::as_str).unwrap_or("");
    let (x, y) = match (args.get(2), args.get(3)) {
        (Some(x), Some(y)) => (x.parse::<u32>()?, y.parse::<u32>()?),
        _ => {
            eprintln!(
                "usage: input-probe click|rclick|press|move <x> <y> | drag <x1> <y1> <x2> <y2>"
            );
            std::process::exit(2);
        }
    };
    let target = if command == "drag" {
        Some((args[4].parse::<u32>()?, args[5].parse::<u32>()?))
    } else {
        None
    };

    let connection = Connection::connect_to_env()?;
    let (globals, queue) = registry_queue_init::<State>(&connection)?;
    let qh = queue.handle();
    let manager = globals.bind::<ZwlrVirtualPointerManagerV1, State, ()>(&qh, 1..=2, ())?;
    // Passing no seat lets the compositor choose its default seat.
    let pointer = manager.create_virtual_pointer(None, &qh, ());
    queue.flush()?;

    pointer.motion_absolute(now_millis(), x, y, OUTPUT_WIDTH, OUTPUT_HEIGHT);
    pointer.frame();
    queue.flush()?;
    std::thread::sleep(std::time::Duration::from_millis(80));

    match command {
        "click" | "rclick" => {
            let button = if command == "rclick" {
                BTN_RIGHT
            } else {
                BTN_LEFT
            };
            pointer.button(now_millis(), button, wl_pointer::ButtonState::Pressed);
            pointer.frame();
            queue.flush()?;
            std::thread::sleep(std::time::Duration::from_millis(60));
            pointer.button(now_millis(), button, wl_pointer::ButtonState::Released);
            pointer.frame();
        }
        "press" => {
            pointer.button(now_millis(), BTN_LEFT, wl_pointer::ButtonState::Pressed);
            pointer.frame();
        }
        "drag" => {
            let (x2, y2) = target.ok_or("drag needs an end point")?;
            pointer.button(now_millis(), BTN_LEFT, wl_pointer::ButtonState::Pressed);
            pointer.frame();
            queue.flush()?;
            std::thread::sleep(std::time::Duration::from_millis(150));
            for step in 1..=10i64 {
                let nx = (i64::from(x) + (i64::from(x2) - i64::from(x)) * step / 10) as u32;
                let ny = (i64::from(y) + (i64::from(y2) - i64::from(y)) * step / 10) as u32;
                pointer.motion_absolute(now_millis(), nx, ny, OUTPUT_WIDTH, OUTPUT_HEIGHT);
                pointer.frame();
                queue.flush()?;
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
            pointer.button(now_millis(), BTN_LEFT, wl_pointer::ButtonState::Released);
            pointer.frame();
        }
        "move" => {}
        _ => {
            eprintln!("unknown command: {command}");
            std::process::exit(2);
        }
    }
    queue.flush()?;
    std::thread::sleep(std::time::Duration::from_millis(120));
    Ok(())
}
