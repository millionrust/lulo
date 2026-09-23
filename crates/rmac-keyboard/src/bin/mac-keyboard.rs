//! `rmac-mac-keyboard`: Mac keyboard behaviour on PC keyboards.
//!
//! * `apply --shortcuts on|off --swap on|off --caps ID --option-characters on|off`
//!   (root, through pkexec from System Settings)
//! * `regenerate` (root, from the rmac-session postinst)
//! * `follow` (the user's session unit: niri focus → keyd bindings)
//! * `reset` (drop dynamic keyd bindings; the unit's stop hook)
//! * `print-config --swap on|off --caps ID --option-characters on|off`

use std::process::ExitCode;

const USAGE: &str = "usage: rmac-mac-keyboard apply --shortcuts on|off --swap on|off --caps ID --option-characters on|off
       rmac-mac-keyboard print-config --swap on|off --caps ID --option-characters on|off
       rmac-mac-keyboard regenerate | follow | reset";

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rmac-mac-keyboard: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: &[String]) -> Result<(), String> {
    let Some((command, rest)) = arguments.split_first() else {
        return Err(USAGE.into());
    };
    match command.as_str() {
        "print-config" => {
            // Reuse the apply parser with shortcuts on: only the layout matters.
            let mut full = vec!["--shortcuts".to_owned(), "on".to_owned()];
            full.extend_from_slice(rest);
            let target = rmac_keyboard::parse_helper_arguments(&full).ok_or(USAGE)?;
            print!("{}", rmac_keyboard::keyd_config(&target.layout));
            Ok(())
        }
        other => platform(other, rest),
    }
}

#[cfg(target_os = "linux")]
fn platform(command: &str, rest: &[String]) -> Result<(), String> {
    use rmac_keyboard::system;

    match (command, rest) {
        ("apply", rest) => {
            let target = rmac_keyboard::parse_helper_arguments(rest).ok_or(USAGE)?;
            system::apply_as_root(&target).map_err(|error| error.to_string())
        }
        ("regenerate", []) => system::regenerate_as_root()
            .map(|changed| {
                if changed {
                    println!("updated {}", system::KEYD_CONFIG);
                }
            })
            .map_err(|error| error.to_string()),
        ("follow", []) => system::follow().map_err(|error| error.to_string()),
        ("reset", []) => system::reset().map_err(|error| error.to_string()),
        _ => Err(USAGE.into()),
    }
}

#[cfg(not(target_os = "linux"))]
fn platform(_command: &str, _rest: &[String]) -> Result<(), String> {
    Err("keyd and localed are available only on Linux".into())
}
