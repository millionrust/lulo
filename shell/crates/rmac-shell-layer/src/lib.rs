//! Layer-surface helpers shared by the rmac shell hosts: per-output
//! reconciliation, compositor geometry projection, and the first-frame marker
//! used by the nested-Wayland smoke harness.

use std::collections::BTreeMap;
use std::env;
use std::fs;

use gpui::Window;
use uuid::Uuid;

const READY_FILE_ENV: &str = "RMAC_SMOKE_READY_FILE";
pub const WAYLAND_OUTPUT_RESTART_EXIT_CODE: i32 = 75;

/// Detect a full Wayland registry restart by watching output reappearance.
pub fn output_reappeared(
    previous: &std::collections::BTreeSet<Uuid>,
    current: &std::collections::BTreeSet<Uuid>,
    removed: &mut std::collections::BTreeSet<Uuid>,
) -> bool {
    removed.extend(previous.difference(current).copied());
    current.iter().any(|output| removed.contains(output))
}

/// Stable per-output menu-bar policy derived from niri's authoritative
/// workspace/window geometry. Niri does not publish a separate fullscreen
/// boolean: a fullscreen tile is the one active tile whose logical extent
/// exactly covers its complete output instead of the compositor work area.
pub fn top_bar_output_policies(snapshot: &rmac_compositor::Snapshot) -> BTreeMap<Uuid, bool> {
    snapshot
        .outputs
        .iter()
        .filter_map(|output| {
            let logical = output.logical.as_ref()?;
            if !output.enabled() || !logical.size.is_valid() {
                return None;
            }
            let fullscreen = !snapshot.overview_visible
                && snapshot
                    .workspaces
                    .iter()
                    .find(|workspace| {
                        workspace.active && workspace.output.as_ref() == Some(&output.id)
                    })
                    .and_then(|workspace| workspace.active_window)
                    .and_then(|window_id| {
                        snapshot
                            .windows
                            .iter()
                            .find(|window| window.id == window_id && window.workspace.is_some())
                    })
                    .is_some_and(|window| {
                        nearly_equal(window.layout.tile_size.width, logical.size.width)
                            && nearly_equal(window.layout.tile_size.height, logical.size.height)
                    });
            Some((stable_output_uuid(&output.id), fullscreen))
        })
        .collect()
}

fn nearly_equal(left: f64, right: f64) -> bool {
    left.is_finite() && right.is_finite() && (left - right).abs() <= 0.5
}

pub fn stable_output_uuid(output: &rmac_compositor::OutputId) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_DNS, output.0.as_bytes())
}

#[cfg(all(target_os = "linux", feature = "wayland"))]
pub mod output_surfaces {
    use std::collections::{BTreeMap, BTreeSet};
    use std::rc::Rc;

    use gpui::{AnyWindowHandle, App, PlatformDisplay};
    use uuid::Uuid;

    pub fn newest_displays(cx: &App) -> BTreeMap<Uuid, Rc<dyn PlatformDisplay>> {
        let mut displays: BTreeMap<Uuid, Rc<dyn PlatformDisplay>> = BTreeMap::new();
        for display in cx.displays() {
            let Ok(uuid) = display.uuid() else {
                continue;
            };
            let replace = displays
                .get(&uuid)
                .is_some_and(|current| u64::from(display.id()) > u64::from(current.id()));
            if replace || !displays.contains_key(&uuid) {
                displays.insert(uuid, display);
            }
        }
        displays
    }

    #[derive(Default)]
    pub struct Tracker {
        windows: BTreeMap<Uuid, AnyWindowHandle>,
    }

    impl Tracker {
        pub fn len(&self) -> usize {
            self.windows.len()
        }

        pub fn is_empty(&self) -> bool {
            self.windows.is_empty()
        }

        pub fn reconcile(
            &mut self,
            desired: Option<&BTreeSet<Uuid>>,
            cx: &mut App,
            mut open: impl FnMut(Rc<dyn PlatformDisplay>, &mut App) -> AnyWindowHandle,
        ) {
            let displays = newest_displays(cx);
            let available = displays.keys().copied().collect::<BTreeSet<_>>();
            let active = desired
                .map(|desired| {
                    desired
                        .intersection(&available)
                        .copied()
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or(available);

            let removed = self
                .windows
                .keys()
                .filter(|uuid| !active.contains(uuid))
                .copied()
                .collect::<Vec<_>>();
            for uuid in removed {
                if let Some(handle) = self.windows.remove(&uuid) {
                    let _ = handle.update(cx, |_, window, _| window.remove_window());
                }
            }

            for (uuid, display) in displays {
                if active.contains(&uuid) && !self.windows.contains_key(&uuid) {
                    self.windows.insert(uuid, open(display, cx));
                }
            }
        }
    }

    pub async fn watch_enabled(
        sender: async_channel::Sender<BTreeSet<Uuid>>,
    ) -> Result<(), String> {
        let (event_tx, event_rx) = async_channel::bounded(64);
        let watcher = async {
            rmac_compositor_niri::watch(event_tx)
                .await
                .map_err(|error| error.to_string())
        };
        let consumer = async {
            let mut state = rmac_compositor::State::default();
            let mut published = BTreeSet::new();
            while let Ok(event) = event_rx.recv().await {
                state.apply(event);
                let next = state
                    .snapshot()
                    .outputs
                    .into_iter()
                    .filter(|output| output.enabled())
                    .map(|output| crate::stable_output_uuid(&output.id))
                    .collect::<BTreeSet<_>>();
                if next != published {
                    published = next.clone();
                    if sender.send(next).await.is_err() {
                        return Ok(());
                    }
                }
            }
            Ok(())
        };
        futures_util::try_join!(watcher, consumer)?;
        Ok(())
    }

    pub async fn watch_top_bar(
        sender: async_channel::Sender<BTreeMap<Uuid, bool>>,
    ) -> Result<(), String> {
        let (event_tx, event_rx) = async_channel::bounded(64);
        let watcher = async {
            rmac_compositor_niri::watch(event_tx)
                .await
                .map_err(|error| error.to_string())
        };
        let consumer = async {
            let mut state = rmac_compositor::State::default();
            let mut published = BTreeMap::new();
            while let Ok(event) = event_rx.recv().await {
                state.apply(event);
                let next = crate::top_bar_output_policies(&state.snapshot());
                if next != published {
                    published = next.clone();
                    if sender.send(next).await.is_err() {
                        return Ok(());
                    }
                }
            }
            Ok(())
        };
        futures_util::try_join!(watcher, consumer)?;
        Ok(())
    }
}

/// Writes an opt-in marker after GPUI finishes the window's first frame.
///
/// The nested-Wayland smoke harness uses this to distinguish a rendered
/// surface from a process that merely reached `open_window`. Normal runs do
/// not set the environment variable and perform no filesystem I/O.
pub fn mark_first_frame(window: &Window, probe: &'static str) {
    let Some(path) = env::var_os(READY_FILE_ENV) else {
        return;
    };

    window.on_next_frame(move |_, _| {
        fs::write(&path, format!("{probe}\n"))
            .unwrap_or_else(|error| panic!("write first-frame marker {path:?}: {error}"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compositor_snapshot(
        tile_height: f64,
        active_window: Option<u64>,
    ) -> rmac_compositor::Snapshot {
        rmac_compositor::Snapshot {
            outputs: vec![rmac_compositor::Output {
                id: rmac_compositor::OutputId::from("eDP-1"),
                make: String::new(),
                model: String::new(),
                serial: None,
                physical_size_mm: None,
                modes: vec![rmac_compositor::OutputMode {
                    physical_size: rmac_compositor::PhysicalSize {
                        width: 1920,
                        height: 1080,
                    },
                    refresh_millihz: 60_000,
                    preferred: true,
                }],
                current_mode: Some(0),
                custom_mode: false,
                vrr_supported: false,
                vrr_enabled: false,
                logical: Some(rmac_compositor::LogicalOutput {
                    position: rmac_compositor::LogicalPoint::default(),
                    size: rmac_compositor::LogicalSize {
                        width: 1536.0,
                        height: 864.0,
                    },
                    scale: 1.25,
                    transform: "normal".into(),
                }),
            }],
            workspaces: vec![rmac_compositor::Workspace {
                id: rmac_compositor::WorkspaceId(1),
                index: 1,
                name: None,
                output: Some(rmac_compositor::OutputId::from("eDP-1")),
                urgent: false,
                active: true,
                focused: true,
                active_window: active_window.map(rmac_compositor::WindowId),
            }],
            windows: active_window
                .map(|id| rmac_compositor::Window {
                    id: rmac_compositor::WindowId(id),
                    title: None,
                    app_id: Some("org.rmac.Test".into()),
                    pid: None,
                    workspace: Some(rmac_compositor::WorkspaceId(1)),
                    focused: true,
                    floating: false,
                    urgent: false,
                    focus_timestamp: None,
                    layout: rmac_compositor::WindowLayout {
                        tile_size: rmac_compositor::LogicalSize {
                            width: 1536.0,
                            height: tile_height,
                        },
                        ..Default::default()
                    },
                })
                .into_iter()
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn top_bar_fullscreen_policy_uses_complete_output_geometry() {
        let uuid = stable_output_uuid(&rmac_compositor::OutputId::from("eDP-1"));
        assert_eq!(
            top_bar_output_policies(&compositor_snapshot(836.0, Some(7))).get(&uuid),
            Some(&false)
        );
        assert_eq!(
            top_bar_output_policies(&compositor_snapshot(864.0, Some(7))).get(&uuid),
            Some(&true)
        );
        assert_eq!(
            top_bar_output_policies(&compositor_snapshot(864.0, None)).get(&uuid),
            Some(&false)
        );

        let mut overview = compositor_snapshot(864.0, Some(7));
        overview.overview_visible = true;
        assert_eq!(top_bar_output_policies(&overview).get(&uuid), Some(&false));
    }

    #[test]
    fn output_reappearance_requires_a_fresh_wayland_registry() {
        let first = Uuid::new_v5(&Uuid::NAMESPACE_DNS, b"first");
        let second = Uuid::new_v5(&Uuid::NAMESPACE_DNS, b"second");
        let mut removed = std::collections::BTreeSet::new();
        let both = [first, second].into_iter().collect();
        let first_only = [first].into_iter().collect();

        assert!(!output_reappeared(&both, &first_only, &mut removed));
        assert!(output_reappeared(&first_only, &both, &mut removed));
    }
}
