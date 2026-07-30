mod appearance;
mod connectivity;
mod controller;
mod displays;
mod focus;
mod input;
mod navigation;
mod notifications;
mod power;
mod service_updates;
mod shell_settings;
mod sound;
mod system_environment;

fn main() {
    controller::run();
}
