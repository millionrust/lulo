//! Photoshop (PSD) documents (PREV-09): the flattened composite image every
//! PSD saved with "Maximise Compatibility" carries, which is what macOS
//! Preview shows too. Layers are not read; a file saved without a
//! composite says so plainly.

use image::RgbaImage;

const MAX_SIDE: u32 = 30_000;
const MAX_PIXELS: u64 = 400_000_000;

pub fn is_psd(bytes: &[u8]) -> bool {
    bytes.starts_with(b"8BPS")
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .at
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| "the PSD file is cut short".to_owned())?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn u16(&mut self) -> Result<u16, String> {
        let bytes = self.take(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn skip_section(&mut self) -> Result<(), String> {
        let length = self.u32()? as usize;
        self.take(length).map(|_| ())
    }
}

/// PackBits-decodes one scanline of `count` bytes.
fn unpack_row(source: &mut Reader<'_>, length: usize, count: usize) -> Result<Vec<u8>, String> {
    let packed = source.take(length)?;
    let mut out = Vec::with_capacity(count);
    let mut i = 0;
    while i < packed.len() && out.len() < count {
        let header = packed[i] as i8;
        i += 1;
        if header >= 0 {
            let run = header as usize + 1;
            let chunk = packed
                .get(i..i + run)
                .ok_or_else(|| "bad PSD compression".to_owned())?;
            out.extend_from_slice(chunk);
            i += run;
        } else if header != -128 {
            let run = 1 - header as isize;
            let value = *packed
                .get(i)
                .ok_or_else(|| "bad PSD compression".to_owned())?;
            out.extend(std::iter::repeat_n(value, run as usize));
            i += 1;
        }
    }
    out.resize(count, 0);
    Ok(out)
}

/// Decodes the composite image of a PSD (8 or 16 bits per channel; RGB,
/// greyscale or CMYK) to RGBA.
pub fn decode(bytes: &[u8]) -> Result<RgbaImage, String> {
    let mut reader = Reader { bytes, at: 0 };
    if reader.take(4)? != b"8BPS" {
        return Err("not a Photoshop document".to_owned());
    }
    let version = reader.u16()?;
    if version != 1 {
        return Err("Large Document Format (PSB) files are not supported".to_owned());
    }
    reader.take(6)?;
    let channels = reader.u16()? as usize;
    let height = reader.u32()?;
    let width = reader.u32()?;
    let depth = reader.u16()?;
    let mode = reader.u16()?;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err("the PSD has an unsupported size".to_owned());
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("the PSD is too large".to_owned());
    }
    if !matches!(depth, 8 | 16) {
        return Err("only 8 and 16 bit Photoshop documents are supported".to_owned());
    }
    // Colour mode: 1 greyscale, 3 RGB, 4 CMYK.
    let colour_channels = match mode {
        1 => 1,
        3 => 3,
        4 => 4,
        _ => return Err("this Photoshop colour mode is not supported".to_owned()),
    };
    if channels < colour_channels {
        return Err("the PSD has too few channels".to_owned());
    }
    reader.skip_section()?; // colour mode data
    reader.skip_section()?; // image resources
    reader.skip_section()?; // layer and mask information
    let compression = reader.u16()?;
    let (width, height) = (width as usize, height as usize);
    let sample = usize::from(depth / 8);
    let row_bytes = width * sample;
    // Planar rows: channel by channel, row by row.
    let used = channels.min(colour_channels + 1);
    let mut planes: Vec<Vec<u8>> = Vec::with_capacity(channels);
    match compression {
        0 => {
            for _ in 0..channels {
                let plane = reader.take(row_bytes * height)?;
                planes.push(plane.to_vec());
            }
        }
        1 => {
            let mut lengths = Vec::with_capacity(channels * height);
            for _ in 0..channels * height {
                lengths.push(reader.u16()? as usize);
            }
            for channel in 0..channels {
                let mut plane = Vec::with_capacity(row_bytes * height);
                for row in 0..height {
                    let length = lengths[channel * height + row];
                    plane.extend(unpack_row(&mut reader, length, row_bytes)?);
                }
                planes.push(plane);
            }
        }
        _ => return Err("this PSD compression is not supported".to_owned()),
    }
    let value = |channel: usize, index: usize| -> u8 { planes[channel][index * sample] };
    let mut pixels = Vec::with_capacity(width * height * 4);
    for index in 0..width * height {
        let rgba = match mode {
            1 => {
                let grey = value(0, index);
                [
                    grey,
                    grey,
                    grey,
                    if used > 1 { value(1, index) } else { 255 },
                ]
            }
            3 => [
                value(0, index),
                value(1, index),
                value(2, index),
                if used > 3 { value(3, index) } else { 255 },
            ],
            _ => {
                // Photoshop stores CMYK inverted (255 = no ink).
                let (c, m, y, k) = (
                    u32::from(value(0, index)),
                    u32::from(value(1, index)),
                    u32::from(value(2, index)),
                    u32::from(value(3, index)),
                );
                [
                    (c * k / 255) as u8,
                    (m * k / 255) as u8,
                    (y * k / 255) as u8,
                    if used > 4 { value(4, index) } else { 255 },
                ]
            }
        };
        pixels.extend_from_slice(&rgba);
    }
    RgbaImage::from_raw(width as u32, height as u32, pixels)
        .ok_or_else(|| "bad PSD bitmap".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: u32, height: u32, channels: u16, depth: u16, mode: u16) -> Vec<u8> {
        let mut file = b"8BPS".to_vec();
        file.extend_from_slice(&1_u16.to_be_bytes());
        file.extend_from_slice(&[0; 6]);
        file.extend_from_slice(&channels.to_be_bytes());
        file.extend_from_slice(&height.to_be_bytes());
        file.extend_from_slice(&width.to_be_bytes());
        file.extend_from_slice(&depth.to_be_bytes());
        file.extend_from_slice(&mode.to_be_bytes());
        for _ in 0..3 {
            file.extend_from_slice(&0_u32.to_be_bytes());
        }
        file
    }

    #[test]
    fn a_raw_rgb_composite_decodes_in_channel_planes() {
        let mut file = header(2, 1, 3, 8, 3);
        file.extend_from_slice(&0_u16.to_be_bytes());
        file.extend_from_slice(&[10, 20]); // red plane
        file.extend_from_slice(&[30, 40]); // green plane
        file.extend_from_slice(&[50, 60]); // blue plane
        let picture = decode(&file).unwrap();
        assert_eq!(picture.dimensions(), (2, 1));
        assert_eq!(picture.get_pixel(0, 0).0, [10, 30, 50, 255]);
        assert_eq!(picture.get_pixel(1, 0).0, [20, 40, 60, 255]);
    }

    #[test]
    fn a_packbits_composite_with_alpha_decodes() {
        let mut file = header(3, 1, 4, 8, 3);
        file.extend_from_slice(&1_u16.to_be_bytes());
        // Four rows of one scanline each; every row is "repeat value 3 times".
        for _ in 0..4 {
            file.extend_from_slice(&2_u16.to_be_bytes());
        }
        for value in [200_u8, 100, 50, 128] {
            file.extend_from_slice(&[(-2_i8) as u8, value]);
        }
        let picture = decode(&file).unwrap();
        assert_eq!(picture.get_pixel(2, 0).0, [200, 100, 50, 128]);
    }

    #[test]
    fn greyscale_and_truncated_files_are_handled() {
        let mut file = header(1, 1, 1, 8, 1);
        file.extend_from_slice(&0_u16.to_be_bytes());
        file.push(77);
        assert_eq!(decode(&file).unwrap().get_pixel(0, 0).0, [77, 77, 77, 255]);
        assert!(decode(&file[..file.len() - 3]).is_err());
        assert!(decode(b"8BPS").is_err());
        assert!(!is_psd(b"GIF89a"));
    }
}
