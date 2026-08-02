#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Layout {
    pub destination: Rect,
    pub tiled: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LayoutError {
    InvalidImage,
    InvalidViewport,
    InvalidScale,
}

pub fn layout(
    fit: rmac_shell_settings::WallpaperFit,
    image: rmac_compositor::PhysicalSize,
    viewport: rmac_compositor::LogicalSize,
    output_scale: f64,
) -> Result<Layout, LayoutError> {
    if image.width == 0 || image.height == 0 {
        return Err(LayoutError::InvalidImage);
    }
    if !viewport.is_valid() || viewport.width <= 0.0 || viewport.height <= 0.0 {
        return Err(LayoutError::InvalidViewport);
    }
    if !output_scale.is_finite() || output_scale <= 0.0 {
        return Err(LayoutError::InvalidScale);
    }
    let image_width = f64::from(image.width) / output_scale;
    let image_height = f64::from(image.height) / output_scale;
    let (width, height, tiled) = match fit {
        rmac_shell_settings::WallpaperFit::Fill => {
            let scale = (viewport.width / image_width).max(viewport.height / image_height);
            (image_width * scale, image_height * scale, false)
        }
        rmac_shell_settings::WallpaperFit::Fit => {
            let scale = (viewport.width / image_width).min(viewport.height / image_height);
            (image_width * scale, image_height * scale, false)
        }
        rmac_shell_settings::WallpaperFit::Stretch => (viewport.width, viewport.height, false),
        rmac_shell_settings::WallpaperFit::Center => (image_width, image_height, false),
        rmac_shell_settings::WallpaperFit::Tile => (image_width, image_height, true),
    };
    Ok(Layout {
        destination: Rect {
            x: (viewport.width - width) / 2.0,
            y: (viewport.height - height) / 2.0,
            width,
            height,
        },
        tiled,
    })
}
