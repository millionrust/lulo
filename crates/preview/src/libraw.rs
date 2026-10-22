//! Camera RAW files (CR2, CR3, NEF, ARW, DNG, ORF, RW2, RAF and more)
//! through the system `libraw`, loaded at run time (PREV-09) so Preview
//! builds and runs without it. A large embedded preview is used when the
//! file has one (fast on a low-spec PC and what the camera showed); smaller
//! previews fall back to developing the sensor data.

use std::ffi::{c_char, c_int, c_uint, c_void, CString};
use std::path::Path;
use std::sync::OnceLock;

use image::RgbaImage;
use libloading::Library;

const LIBRARY_NAMES: &[&str] = &[
    "libraw.so.25",
    "libraw.so.24",
    "libraw.so.23",
    "libraw.so.22",
    "libraw.so",
    "libraw.dylib",
    "libraw.dll",
];

/// An embedded preview at least this big on its long side stands in for the
/// developed image.
const PREVIEW_LONG_SIDE: u32 = 1600;
const MAX_PIXELS: u64 = 400_000_000;
const FORMAT_JPEG: c_int = 1;
const FORMAT_BITMAP: c_int = 2;

type Ptr = *mut c_void;

#[repr(C)]
struct Processed {
    kind: c_int,
    height: u16,
    width: u16,
    colors: u16,
    bits: u16,
    data_size: c_uint,
    data: [u8; 1],
}

struct Api {
    _library: Library,
    init: unsafe extern "C" fn(c_uint) -> Ptr,
    open_file: unsafe extern "C" fn(Ptr, *const c_char) -> c_int,
    unpack: unsafe extern "C" fn(Ptr) -> c_int,
    unpack_thumb: unsafe extern "C" fn(Ptr) -> c_int,
    process: unsafe extern "C" fn(Ptr) -> c_int,
    make_image: unsafe extern "C" fn(Ptr, *mut c_int) -> *mut Processed,
    make_thumb: unsafe extern "C" fn(Ptr, *mut c_int) -> *mut Processed,
    clear_mem: unsafe extern "C" fn(*mut Processed),
    close: unsafe extern "C" fn(Ptr),
}

impl Api {
    fn load() -> Option<Self> {
        let library = LIBRARY_NAMES
            .iter()
            // SAFETY: loading libraw runs only its own initialisers.
            .find_map(|name| unsafe { Library::new(name) }.ok())?;
        macro_rules! symbol {
            ($name:literal) => {
                // SAFETY: the signatures match libraw/libraw.h (C API).
                *unsafe { library.get(concat!($name, "\0").as_bytes()) }.ok()?
            };
        }
        Some(Self {
            init: symbol!("libraw_init"),
            open_file: symbol!("libraw_open_file"),
            unpack: symbol!("libraw_unpack"),
            unpack_thumb: symbol!("libraw_unpack_thumb"),
            process: symbol!("libraw_dcraw_process"),
            make_image: symbol!("libraw_dcraw_make_mem_image"),
            make_thumb: symbol!("libraw_dcraw_make_mem_thumb"),
            clear_mem: symbol!("libraw_dcraw_clear_mem"),
            close: symbol!("libraw_close"),
            _library: library,
        })
    }
}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(Api::load).as_ref()
}

pub fn available() -> bool {
    api().is_some()
}

/// File name extensions of the RAW formats libraw reads.
pub fn has_raw_extension(name: &str) -> bool {
    let Some((_, extension)) = name.rsplit_once('.') else {
        return false;
    };
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "cr2"
            | "cr3"
            | "crw"
            | "nef"
            | "nrw"
            | "arw"
            | "srf"
            | "sr2"
            | "dng"
            | "orf"
            | "rw2"
            | "raf"
            | "pef"
            | "srw"
            | "3fr"
            | "erf"
            | "kdc"
            | "mrw"
            | "raw"
    )
}

struct Session(&'static Api, Ptr);

impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: from `libraw_init`, closed once.
        unsafe { (self.0.close)(self.1) }
    }
}

struct Memory(&'static Api, *mut Processed);

impl Drop for Memory {
    fn drop(&mut self) {
        // SAFETY: from libraw's make_mem_*, freed once.
        unsafe { (self.0.clear_mem)(self.1) }
    }
}

fn rgba_from(processed: &Memory) -> Result<RgbaImage, String> {
    // SAFETY: libraw allocates `data_size` bytes after the header.
    unsafe {
        let header = &*processed.1;
        let (width, height) = (u32::from(header.width), u32::from(header.height));
        let size = header.data_size as usize;
        let bytes = std::slice::from_raw_parts(header.data.as_ptr(), size);
        match header.kind {
            FORMAT_JPEG => image::load_from_memory_with_format(bytes, image::ImageFormat::Jpeg)
                .map(|picture| picture.into_rgba8())
                .map_err(|error| error.to_string()),
            FORMAT_BITMAP => {
                if u64::from(width) * u64::from(height) > MAX_PIXELS || header.colors != 3 {
                    return Err("unsupported RAW bitmap".to_owned());
                }
                let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
                match header.bits {
                    8 if size >= width as usize * height as usize * 3 => {
                        for rgb in bytes.chunks_exact(3).take(width as usize * height as usize) {
                            pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                        }
                    }
                    16 if size >= width as usize * height as usize * 6 => {
                        for rgb in bytes.chunks_exact(6).take(width as usize * height as usize) {
                            // Little-endian 16-bit samples: keep the high byte.
                            pixels.extend_from_slice(&[rgb[1], rgb[3], rgb[5], 255]);
                        }
                    }
                    _ => return Err("unsupported RAW bit depth".to_owned()),
                }
                RgbaImage::from_raw(width, height, pixels)
                    .ok_or_else(|| "bad RAW bitmap".to_owned())
            }
            _ => Err("unsupported RAW preview".to_owned()),
        }
    }
}

/// Decodes a RAW file to RGBA.
pub fn decode(path: &Path) -> Result<RgbaImage, String> {
    let api = api().ok_or_else(|| {
        "Lulo needs the libraw library to open camera RAW files (install libraw).".to_owned()
    })?;
    let name = CString::new(path.as_os_str().as_encoded_bytes().to_vec())
        .map_err(|_| "the file name contains a NUL byte".to_owned())?;
    // SAFETY: pointers come from libraw and are released by the guards.
    unsafe {
        let session = Session(api, (api.init)(0));
        if session.1.is_null() {
            return Err("libraw could not start".to_owned());
        }
        if (api.open_file)(session.1, name.as_ptr()) != 0 {
            return Err("this RAW file cannot be read".to_owned());
        }
        if (api.unpack_thumb)(session.1) == 0 {
            let mut error: c_int = 0;
            let thumb = (api.make_thumb)(session.1, &mut error);
            if !thumb.is_null() {
                let thumb = Memory(api, thumb);
                if let Ok(picture) = rgba_from(&thumb) {
                    if picture.width().max(picture.height()) >= PREVIEW_LONG_SIDE {
                        return Ok(picture);
                    }
                }
            }
        }
        if (api.unpack)(session.1) != 0 {
            return Err("this RAW file cannot be read".to_owned());
        }
        if (api.process)(session.1) != 0 {
            return Err("this RAW file cannot be developed".to_owned());
        }
        let mut error: c_int = 0;
        let developed = (api.make_image)(session.1, &mut error);
        if developed.is_null() {
            return Err("this RAW file cannot be developed".to_owned());
        }
        rgba_from(&Memory(api, developed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_extensions_are_recognised_case_insensitively() {
        assert!(has_raw_extension("IMG_0001.CR2"));
        assert!(has_raw_extension("dsc.nef"));
        assert!(has_raw_extension("photo.DNG"));
        assert!(!has_raw_extension("photo.jpg"));
        assert!(!has_raw_extension("noextension"));
    }

    #[test]
    fn a_file_that_is_not_raw_is_an_error_not_a_crash() {
        if !available() {
            return;
        }
        let path =
            std::env::temp_dir().join(format!("rmac-preview-notraw-{}.cr2", std::process::id()));
        std::fs::write(&path, b"definitely not a camera file").unwrap();
        assert!(decode(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
