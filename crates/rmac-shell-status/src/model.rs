use rmac_compositor::{OutputId, WindowId, WorkspaceId};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FocusedContext {
    pub output: Option<OutputId>,
    pub workspace_id: Option<WorkspaceId>,
    pub workspace_label: Option<String>,
    pub window_id: Option<WindowId>,
    pub app_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputContext {
    pub id: OutputId,
    pub logical_size: rmac_compositor::LogicalSize,
    pub scale: f64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NetworkState {
    Unavailable,
    Disconnected,
    Portal,
    Limited,
    Connected,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NetworkIndicator {
    pub state: NetworkState,
    pub connection_name: Option<String>,
    pub wifi_strength: Option<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VpnIndicator {
    pub active_names: Vec<String>,
    pub transitioning: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BluetoothIndicator {
    pub available: bool,
    pub powered: bool,
    pub connected_devices: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SoundIndicator {
    pub available: bool,
    pub volume: u8,
    pub muted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BatteryIndicator {
    pub percentage: u8,
    pub state: rmac_power::BatteryState,
    pub on_battery: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FocusIndicator {
    pub enabled: bool,
    pub mode: Option<String>,
    pub ends_at_unix_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NotificationIndicator {
    pub unread_count: u32,
    pub has_urgent: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// Enabled outputs with valid logical geometry, stripped of hardware
    /// serials and other diagnostic-only compositor metadata.
    pub outputs: Vec<OutputContext>,
    pub focused: FocusedContext,
    pub network: Option<NetworkIndicator>,
    pub vpn: Option<VpnIndicator>,
    pub bluetooth: Option<BluetoothIndicator>,
    pub sound: Option<SoundIndicator>,
    pub battery: Option<BatteryIndicator>,
    pub show_battery_percentage: bool,
    pub focus: Option<FocusIndicator>,
    pub notifications: Option<NotificationIndicator>,
    pub clock: rmac_shell_settings::ClockSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Compositor(rmac_compositor::Event),
    Network(rmac_network::NetworkSnapshot),
    Wifi(rmac_network::WifiSnapshot),
    Vpn(rmac_network::VpnSnapshot),
    Bluetooth(rmac_bluetooth::Snapshot),
    Audio(rmac_audio::Snapshot),
    Power(rmac_power::Snapshot),
    Settings(rmac_shell_settings::ShellSettings),
    Notifications(NotificationIndicator),
    Focus(Option<FocusIndicator>),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Change {
    /// True only when a consumer-visible projection changed.
    pub visible: bool,
}
