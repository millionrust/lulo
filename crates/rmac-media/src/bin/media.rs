//! `rmac-media <play-pause|play|pause|stop|next|previous|status>` for the
//! media keys and diagnostics.

use std::process::ExitCode;

fn main() -> ExitCode {
    let argument = std::env::args().nth(1).unwrap_or_default();
    if argument == "status" {
        return match rmac_media::active_player() {
            Ok(Some(player)) => {
                println!(
                    "{} {:?}: {} — {}",
                    player.identity,
                    player.status,
                    player.title.as_deref().unwrap_or("(no title)"),
                    player.artist.as_deref().unwrap_or("(no artist)"),
                );
                ExitCode::SUCCESS
            }
            Ok(None) => {
                println!("no active player");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        };
    }
    let Some(command) = rmac_media::Command::parse(&argument) else {
        eprintln!("usage: rmac-media <play-pause|play|pause|stop|next|previous|status>");
        return ExitCode::from(2);
    };
    match rmac_media::send(command) {
        Ok(_) => ExitCode::SUCCESS,
        // Pressing a media key with nothing to control is not an error.
        Err(error) if error.kind == rmac_media::ErrorKind::NoPlayer => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
