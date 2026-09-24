//! The World Clock's day/night map, rasterised on the CPU.
//!
//! Geometry was fitted to the Mac's map (design-lab/apps-clock-weather-player
//! .html): equirectangular, 1024/360 pt per degree of longitude but 3.02 pt
//! per degree of latitude, the equator 258 pt below the map's top, in a
//! 1024 × 500 band. Everything scales with the map's width.

use crate::solar::Subsolar;

/// Map band at the measured 1024 pt width.
pub const REFERENCE_WIDTH: f32 = 1024.0;
pub const REFERENCE_HEIGHT: f32 = 500.0;
const EQUATOR_Y: f32 = 258.0;
const POINTS_PER_LATITUDE: f32 = 3.02;

/// Measured colours (0xRRGGBB).
pub const OCEAN: u32 = 0x000000;
pub const NIGHT_LAND: u32 = 0x191919;
pub const DAY_LAND: u32 = 0x3F3F3F;
pub const TERMINATOR: u32 = 0x8A8A8A;
pub const MERIDIAN: u32 = 0x303030;
/// Meridian lines split the map into twelve bands.
pub const MERIDIAN_BANDS: u32 = 12;

/// Map height for a given width.
pub fn height_for(width: f32) -> f32 {
    width * REFERENCE_HEIGHT / REFERENCE_WIDTH
}

/// Position (points from the map's top-left) of a longitude/latitude.
pub fn project(width: f32, longitude: f64, latitude: f64) -> (f32, f32) {
    let scale = width / REFERENCE_WIDTH;
    let x = ((longitude + 180.0) / 360.0) as f32 * width;
    let y = (EQUATOR_Y - POINTS_PER_LATITUDE * latitude as f32) * scale;
    (x, y)
}

fn latitude_at(width: f32, y: f32) -> f64 {
    let scale = width / REFERENCE_WIDTH;
    f64::from((EQUATOR_Y * scale - y) / (POINTS_PER_LATITUDE * scale))
}

/// Land outlines as rings of (longitude, latitude) degrees.
#[derive(Clone, Debug, Default)]
pub struct Land {
    rings: Vec<Vec<(f64, f64)>>,
}

impl Land {
    /// Parse the bundled `world-land.svg`: one path of absolute `M x y L x y
    /// … Z` rings with x = (lon + 180) · 10 and y = (90 − lat) · 10.
    pub fn parse(svg: &str) -> Option<Self> {
        let start = svg.find(" d=\"")? + 4;
        let data = &svg[start..];
        let data = &data[..data.find('"')?];
        let mut rings = Vec::new();
        let mut ring: Vec<(f64, f64)> = Vec::new();
        let mut numbers = Vec::with_capacity(2);
        let mut token = String::new();
        let flush_number = |token: &mut String, numbers: &mut Vec<f64>| -> Option<()> {
            if !token.is_empty() {
                numbers.push(token.parse().ok()?);
                token.clear();
            }
            Some(())
        };
        for character in data.chars() {
            match character {
                '0'..='9' | '.' | '-' => token.push(character),
                'M' | 'L' | 'Z' | ' ' | '\n' | ',' => {
                    flush_number(&mut token, &mut numbers)?;
                    if numbers.len() == 2 {
                        ring.push((numbers[0] / 10.0 - 180.0, 90.0 - numbers[1] / 10.0));
                        numbers.clear();
                    }
                    if character == 'M' || character == 'Z' {
                        if ring.len() >= 3 {
                            rings.push(std::mem::take(&mut ring));
                        }
                        ring.clear();
                    }
                }
                _ => return None,
            }
        }
        flush_number(&mut token, &mut numbers)?;
        if ring.len() >= 3 {
            rings.push(ring);
        }
        (!rings.is_empty()).then_some(Self { rings })
    }

    pub fn ring_count(&self) -> usize {
        self.rings.len()
    }

    /// Even-odd point-in-land test.
    pub fn contains(&self, longitude: f64, latitude: f64) -> bool {
        let mut inside = false;
        for ring in &self.rings {
            let mut previous = ring[ring.len() - 1];
            for &point in ring {
                if (point.1 > latitude) != (previous.1 > latitude) {
                    let x = previous.0
                        + (latitude - previous.1) * (point.0 - previous.0) / (point.1 - previous.1);
                    if longitude < x {
                        inside = !inside;
                    }
                }
                previous = point;
            }
        }
        inside
    }

    /// Rasterise the land into a `width` × `height` pixel mask for a map
    /// `map_width` points wide (pixels = points × `scale`).
    pub fn mask(&self, map_width: f32, width: usize, height: usize, scale: f32) -> Vec<bool> {
        let mut mask = vec![false; width * height];
        let mut crossings = Vec::new();
        for row in 0..height {
            let latitude = latitude_at(map_width, (row as f32 + 0.5) / scale);
            crossings.clear();
            for ring in &self.rings {
                let mut previous = ring[ring.len() - 1];
                for &point in ring {
                    if (point.1 > latitude) != (previous.1 > latitude) {
                        let longitude = previous.0
                            + (latitude - previous.1) * (point.0 - previous.0)
                                / (point.1 - previous.1);
                        crossings.push(((longitude + 180.0) / 360.0) * width as f64);
                    }
                    previous = point;
                }
            }
            crossings.sort_by(|a, b| a.total_cmp(b));
            for pair in crossings.chunks_exact(2) {
                let from = (pair[0] - 0.5).ceil().max(0.0) as usize;
                let to = ((pair[1] - 0.5).ceil().max(0.0) as usize).min(width);
                for column in from..to {
                    mask[row * width + column] = true;
                }
            }
        }
        mask
    }
}

/// Paint the map as BGRA pixels (GPUI's image layout), opaque.
pub fn paint(
    land: &[bool],
    sun: &Subsolar,
    map_width: f32,
    width: usize,
    height: usize,
    scale: f32,
) -> Vec<u8> {
    let mut day = vec![false; width * height];
    let columns: Vec<f64> = (0..width)
        .map(|column| {
            let longitude = (column as f64 + 0.5) / width as f64 * 360.0 - 180.0;
            (longitude - sun.longitude).to_radians().cos()
        })
        .collect();
    let (sin_decl, cos_decl) = sun.declination.sin_cos();
    for row in 0..height {
        let latitude = latitude_at(map_width, (row as f32 + 0.5) / scale).to_radians();
        let (sin_lat, cos_lat) = latitude.sin_cos();
        let base = sin_lat * sin_decl;
        let factor = cos_lat * cos_decl;
        for (column, cosine) in columns.iter().enumerate() {
            day[row * width + column] = base + factor * cosine > 0.0;
        }
    }
    let band = width as f32 / MERIDIAN_BANDS as f32;
    let line = scale.round().max(1.0) as usize;
    let is_meridian = |column: usize| {
        (1..MERIDIAN_BANDS).any(|k| {
            let at = (k as f32 * band).round() as usize;
            column >= at && column < at + line
        })
    };
    let meridian_columns: Vec<bool> = (0..width).map(is_meridian).collect();
    let mut pixels = vec![0u8; width * height * 4];
    for row in 0..height {
        for (column, &meridian) in meridian_columns.iter().enumerate() {
            let index = row * width + column;
            let lit = day[index];
            let edge = (column + 1 < width && day[index + 1] != lit)
                || (column > 0 && day[index - 1] != lit)
                || (row + 1 < height && day[index + width] != lit)
                || (row > 0 && day[index - width] != lit);
            let colour = if edge {
                TERMINATOR
            } else if land[index] {
                if lit {
                    DAY_LAND
                } else {
                    NIGHT_LAND
                }
            } else if meridian {
                MERIDIAN
            } else {
                OCEAN
            };
            let out = &mut pixels[index * 4..index * 4 + 4];
            out[0] = colour as u8;
            out[1] = (colour >> 8) as u8;
            out[2] = (colour >> 16) as u8;
            out[3] = 0xFF;
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solar::subsolar;

    const LAND: &str = include_str!("../assets/world-land.svg");

    #[test]
    fn projection_matches_the_fitted_mac_map() {
        // New Delhi's pin sat at (731, 226) on the Mac's 1024-wide map band.
        let (x, y) = project(1024.0, 77.21, 28.61);
        assert!((x - 731.6).abs() < 1.0, "{x}");
        assert!((y - 171.6).abs() < 1.0, "{y}");
        assert_eq!(height_for(1024.0), 500.0);
        assert!((latitude_at(1024.0, 258.0)).abs() < 1e-6);
        let (_, half) = project(512.0, 0.0, 0.0);
        assert_eq!(half, 129.0);
    }

    #[test]
    fn bundled_land_parses_and_knows_continents() {
        let land = Land::parse(LAND).expect("land outlines");
        assert!(land.ring_count() > 100);
        assert!(land.contains(77.21, 28.61)); // New Delhi
        assert!(land.contains(2.35, 48.86)); // Paris
        assert!(land.contains(-60.0, -10.0)); // Amazon
        assert!(land.contains(134.0, -25.0)); // central Australia
        assert!(!land.contains(-30.0, 30.0)); // mid-Atlantic
        assert!(!land.contains(-150.0, 0.0)); // Pacific
        assert!(!land.contains(80.0, -20.0)); // Indian Ocean
    }

    #[test]
    fn malformed_paths_are_rejected() {
        assert!(Land::parse("<svg/>").is_none());
        assert!(Land::parse("<path d=\"M1 2Lx 3Z\"/>").is_none());
        assert!(Land::parse("<path d=\"M1 2L3 4L5 0Z\"/>").is_some());
    }

    #[test]
    fn painted_map_lights_the_day_side() {
        let land = Land::parse(LAND).unwrap();
        let (width, height, scale) = (256usize, 125usize, 0.25f32);
        let mask = land.mask(1024.0, width, height, scale);
        // Noon in New Delhi, 2026-09-23 06:45 UTC.
        let sun = subsolar(20_719.0 * 86_400.0 + 6.75 * 3600.0);
        let pixels = paint(&mask, &sun, 1024.0, width, height, scale);
        let at = |lon: f64, lat: f64| {
            let (x, y) = project(1024.0, lon, lat);
            let (column, row) = ((x * scale) as usize, (y * scale) as usize);
            let index = (row * width + column) * 4;
            u32::from(pixels[index + 2]) << 16
                | u32::from(pixels[index + 1]) << 8
                | u32::from(pixels[index])
        };
        assert_eq!(at(77.21, 23.0), DAY_LAND); // central India, noon
        assert_eq!(at(-100.0, 40.0), NIGHT_LAND); // central USA, night
        assert_eq!(at(-140.0, 10.0), OCEAN); // Pacific
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 0xFF));
    }
}
