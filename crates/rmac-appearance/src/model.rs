#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorScheme {
    #[default]
    NoPreference,
    PreferDark,
    PreferLight,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResolvedColorScheme {
    #[default]
    Light,
    Dark,
}

impl ColorScheme {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoPreference => "Automatic",
            Self::PreferDark => "Dark",
            Self::PreferLight => "Light",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Contrast {
    #[default]
    Normal,
    Higher,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MotionPreference {
    #[default]
    Full,
    Reduced,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextScale {
    #[default]
    Standard,
    Large,
    ExtraLarge,
}

impl TextScale {
    pub const fn factor(self) -> f32 {
        match self {
            Self::Standard => 1.0,
            Self::Large => 1.15,
            Self::ExtraLarge => 1.3,
        }
    }
}

/// An sRGB accent color normalized to the inclusive 0–1 range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AccentColor {
    red: f64,
    green: f64,
    blue: f64,
}

impl AccentColor {
    pub fn new(red: f64, green: f64, blue: f64) -> Option<Self> {
        [red, green, blue]
            .into_iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
            .then_some(Self { red, green, blue })
    }

    pub fn red(self) -> f64 {
        self.red
    }

    pub fn green(self) -> f64 {
        self.green
    }

    pub fn blue(self) -> f64 {
        self.blue
    }

    pub fn components(self) -> (f64, f64, f64) {
        (self.red, self.green, self.blue)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub color_scheme: bool,
    pub accent_color: bool,
    pub contrast: bool,
    pub reduced_motion: bool,
}

impl Capabilities {
    pub fn any(self) -> bool {
        self.color_scheme || self.accent_color || self.contrast || self.reduced_motion
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub available: bool,
    pub color_scheme: ColorScheme,
    pub accent_color: Option<AccentColor>,
    pub contrast: Contrast,
    pub motion: MotionPreference,
    pub capabilities: Capabilities,
    pub detail: Option<String>,
}

impl Snapshot {
    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self {
            detail: Some(detail.into()),
            ..Self::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedAppearance {
    pub color_scheme: ResolvedColorScheme,
    pub accent_color: AccentColor,
    pub contrast: Contrast,
    pub motion: MotionPreference,
    pub text_scale: TextScale,
}
