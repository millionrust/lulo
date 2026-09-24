//! A tiny standalone Quick Look host: opens exactly one Quick Look panel on
//! the paths given on the command line and exits once it closes.
//!
//! `rmac_quick_look::open` already draws and drives the whole panel (it is
//! the same code Files and the Open/Save panel embed in-process); this
//! binary exists purely so a surface that wants Quick Look for a single
//! item — the desktop's context menu (DESK-01) — can spawn it instead of
//! linking `rmac-quick-look`, and with it `gpui-component` and every asset
//! and preview backend that comes with it, into a process that must stay
//! lean (the wallpaper renderer is resident on every session, including
//! low-end PCs).

use std::borrow::Cow;
use std::path::PathBuf;

use gpui::{App, AssetSource, Result, SharedString};

// The panel draws entirely through its own embedded SVGs (content.rs,
// metrics.rs) and gpui-component's window-chrome wrapper only — no
// gpui-component widget here needs the bundled icon/font assets those
// widgets normally read through gpui-component-assets, so this asset
// source stays scoped to what Quick Look itself ships.
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(rmac_quick_look::asset(path))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(rmac_quick_look::asset_paths(path)
            .into_iter()
            .map(SharedString::from)
            .collect())
    }
}

fn main() {
    let paths = std::env::args()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    if paths.is_empty() {
        eprintln!("usage: rmac-quick-look PATH…");
        std::process::exit(2);
    }
    rmac_ui::application()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            let options = rmac_quick_look::Options { uncompress: false };
            match rmac_quick_look::open(paths.clone(), 0, options, cx) {
                Some((_, panel)) => {
                    cx.observe_release(&panel, |_, _, cx| cx.quit()).detach();
                    cx.activate(true);
                }
                None => {
                    eprintln!("Quick Look could not open its window");
                    cx.quit();
                }
            }
        });
}
