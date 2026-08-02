use crate::{HealthSnapshot, Snapshot, SourceHealth};

#[derive(Default)]
pub struct Coordinator {
    compositor: rmac_compositor::State,
    settings: rmac_shell_settings::WallpaperSettings,
    health: HealthSnapshot,
}

impl Coordinator {
    pub fn ready(&self) -> bool {
        self.health.compositor != SourceHealth::Starting
            && self.health.settings != SourceHealth::Starting
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            plan: rmac_wallpaper::plan(&self.settings, &self.compositor.snapshot().outputs),
            health: self.health.clone(),
        }
    }

    pub fn apply_compositor(&mut self, event: rmac_compositor::Event) -> bool {
        let before = self.snapshot();
        match &event {
            rmac_compositor::Event::ConnectionChanged { state } => {
                self.health.compositor = match state {
                    rmac_compositor::ConnectionState::Connected => SourceHealth::Healthy,
                    rmac_compositor::ConnectionState::Connecting => SourceHealth::Starting,
                    rmac_compositor::ConnectionState::Disconnected => SourceHealth::Unavailable {
                        detail: "niri wallpaper output events are disconnected".into(),
                    },
                    rmac_compositor::ConnectionState::Reconnecting => SourceHealth::Unavailable {
                        detail: "niri wallpaper output events are reconnecting".into(),
                    },
                };
            }
            _ => self.health.compositor = SourceHealth::Healthy,
        }
        self.compositor.apply(event);
        before != self.snapshot()
    }

    pub fn apply_settings(
        &mut self,
        result: Result<rmac_shell_settings::ShellSettings, String>,
    ) -> bool {
        let before = self.snapshot();
        match result {
            Ok(settings) => {
                self.settings = settings.wallpaper;
                self.health.settings = SourceHealth::Healthy;
            }
            Err(detail) => self.health.settings = SourceHealth::Unavailable { detail },
        }
        before != self.snapshot()
    }

    pub fn apply_file_health(&mut self, health: SourceHealth) -> bool {
        if self.health.files == health {
            return false;
        }
        self.health.files = health;
        true
    }
}
