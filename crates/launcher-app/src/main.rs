//! Session-owned, Spotlight-style launcher surface.

mod assets;
mod intelligence;
mod service;
mod view;

fn main() {
    service::run();
}
