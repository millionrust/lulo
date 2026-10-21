//! HEIC and AVIF through the system `libheif` (PREV-09), loaded at run time
//! so Preview builds and runs without it: a machine without the library
//! simply cannot open these formats and says so. Ubuntu's `libheif1` and
//! `libheif-plugin-aomdec` (AVIF) / `libheif-plugin-libde265` (HEIC)
//! supply everything; saving needs an encoder plugin (`libheif-plugin-aomenc`
//! for AVIF, `libheif-plugin-x265` for HEIC) and reports plainly when none
//! is installed.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use std::sync::OnceLock;

use image::RgbaImage;
use libloading::Library;

const LIBRARY_NAMES: &[&str] = &[
    "libheif.so.1",
    "libheif.so",
    "libheif.1.dylib",
    "libheif.dylib",
    "heif.dll",
    "libheif.dll",
];

const COLORSPACE_RGB: c_int = 1;
const CHROMA_INTERLEAVED_RGBA: c_int = 11;
const CHANNEL_INTERLEAVED: c_int = 10;
const COMPRESSION_HEVC: c_int = 1;
const COMPRESSION_AV1: c_int = 4;

/// Refuse pictures far larger than any camera makes before allocating.
const MAX_PIXELS: u64 = 400_000_000;

#[repr(C)]
struct HeifError {
    code: c_int,
    subcode: c_int,
    message: *const c_char,
}

type Ptr = *mut c_void;

struct Api {
    _library: Library,
    context_alloc: unsafe extern "C" fn() -> Ptr,
    context_free: unsafe extern "C" fn(Ptr),
    read_from_file: unsafe extern "C" fn(Ptr, *const c_char, *const c_void) -> HeifError,
    primary_handle: unsafe extern "C" fn(Ptr, *mut Ptr) -> HeifError,
    handle_release: unsafe extern "C" fn(Ptr),
    decode_image: unsafe extern "C" fn(Ptr, *mut Ptr, c_int, c_int, *const c_void) -> HeifError,
    plane_readonly: unsafe extern "C" fn(Ptr, c_int, *mut c_int) -> *const u8,
    image_width: unsafe extern "C" fn(Ptr, c_int) -> c_int,
    image_height: unsafe extern "C" fn(Ptr, c_int) -> c_int,
    image_release: unsafe extern "C" fn(Ptr),
    encoder_for_format: unsafe extern "C" fn(Ptr, c_int, *mut Ptr) -> HeifError,
    image_create: unsafe extern "C" fn(c_int, c_int, c_int, c_int, *mut Ptr) -> HeifError,
    image_add_plane: unsafe extern "C" fn(Ptr, c_int, c_int, c_int, c_int) -> HeifError,
    plane: unsafe extern "C" fn(Ptr, c_int, *mut c_int) -> *mut u8,
    set_quality: unsafe extern "C" fn(Ptr, c_int) -> HeifError,
    encode_image: unsafe extern "C" fn(Ptr, Ptr, Ptr, *const c_void, *mut Ptr) -> HeifError,
    encoder_release: unsafe extern "C" fn(Ptr),
    write_to_file: unsafe extern "C" fn(Ptr, *const c_char) -> HeifError,
}

impl Api {
    fn load() -> Option<Self> {
        let library = LIBRARY_NAMES
            .iter()
            // SAFETY: loading libheif runs only its own initialisers.
            .find_map(|name| unsafe { Library::new(name) }.ok())?;
        macro_rules! symbol {
            ($name:literal) => {
                // SAFETY: the signatures match libheif/heif.h (API 1.x).
                *unsafe { library.get(concat!($name, "\0").as_bytes()) }.ok()?
            };
        }
        Some(Self {
            context_alloc: symbol!("heif_context_alloc"),
            context_free: symbol!("heif_context_free"),
            read_from_file: symbol!("heif_context_read_from_file"),
            primary_handle: symbol!("heif_context_get_primary_image_handle"),
            handle_release: symbol!("heif_image_handle_release"),
            decode_image: symbol!("heif_decode_image"),
            plane_readonly: symbol!("heif_image_get_plane_readonly"),
            image_width: symbol!("heif_image_get_width"),
            image_height: symbol!("heif_image_get_height"),
            image_release: symbol!("heif_image_release"),
            encoder_for_format: symbol!("heif_context_get_encoder_for_format"),
            image_create: symbol!("heif_image_create"),
            image_add_plane: symbol!("heif_image_add_plane"),
            plane: symbol!("heif_image_get_plane"),
            set_quality: symbol!("heif_encoder_set_lossy_quality"),
            encode_image: symbol!("heif_context_encode_image"),
            encoder_release: symbol!("heif_encoder_release"),
            write_to_file: symbol!("heif_context_write_to_file"),
            _library: library,
        })
    }
}

fn api() -> Option<&'static Api> {
    static API: OnceLock<Option<Api>> = OnceLock::new();
    API.get_or_init(Api::load).as_ref()
}

/// Whether the system has libheif at all.
pub fn available() -> bool {
    api().is_some()
}

const MISSING: &str = "Lulo needs the libheif library to open this format (install libheif1).";

fn check(error: HeifError) -> Result<(), String> {
    if error.code == 0 {
        return Ok(());
    }
    let message = if error.message.is_null() {
        String::new()
    } else {
        // SAFETY: libheif returns a static NUL-terminated message.
        unsafe { CStr::from_ptr(error.message) }
            .to_string_lossy()
            .into_owned()
    };
    Err(if message.is_empty() {
        format!("libheif error {}", error.code)
    } else {
        message
    })
}

struct Context<'a>(&'a Api, Ptr);

impl Drop for Context<'_> {
    fn drop(&mut self) {
        // SAFETY: the context came from `heif_context_alloc`.
        unsafe { (self.0.context_free)(self.1) }
    }
}

struct Handle<'a>(&'a Api, Ptr);

impl Drop for Handle<'_> {
    fn drop(&mut self) {
        // SAFETY: the handle came from libheif and is released once.
        unsafe { (self.0.handle_release)(self.1) }
    }
}

struct Picture<'a>(&'a Api, Ptr);

impl Drop for Picture<'_> {
    fn drop(&mut self) {
        // SAFETY: the image came from libheif and is released once.
        unsafe { (self.0.image_release)(self.1) }
    }
}

fn c_path(path: &Path) -> Result<CString, String> {
    CString::new(path.as_os_str().as_encoded_bytes().to_vec())
        .map_err(|_| "the file name contains a NUL byte".to_owned())
}

/// Decodes the primary image of a HEIC or AVIF file to RGBA (rotation and
/// mirroring stored in the file are already applied by libheif).
pub fn decode(path: &Path) -> Result<RgbaImage, String> {
    let api = api().ok_or_else(|| MISSING.to_owned())?;
    let name = c_path(path)?;
    // SAFETY: every pointer below is created by libheif and released by the
    // guards above; the output slices are read only while `picture` lives.
    unsafe {
        let context = Context(api, (api.context_alloc)());
        if context.1.is_null() {
            return Err("libheif could not start".to_owned());
        }
        check((api.read_from_file)(
            context.1,
            name.as_ptr(),
            std::ptr::null(),
        ))?;
        let mut handle = std::ptr::null_mut();
        check((api.primary_handle)(context.1, &mut handle))?;
        let handle = Handle(api, handle);
        let mut picture = std::ptr::null_mut();
        check((api.decode_image)(
            handle.1,
            &mut picture,
            COLORSPACE_RGB,
            CHROMA_INTERLEAVED_RGBA,
            std::ptr::null(),
        ))?;
        let picture = Picture(api, picture);
        let width = (api.image_width)(picture.1, CHANNEL_INTERLEAVED);
        let height = (api.image_height)(picture.1, CHANNEL_INTERLEAVED);
        if width <= 0 || height <= 0 || (width as u64) * (height as u64) > MAX_PIXELS {
            return Err("the picture has an unsupported size".to_owned());
        }
        let mut stride: c_int = 0;
        let plane = (api.plane_readonly)(picture.1, CHANNEL_INTERLEAVED, &mut stride);
        if plane.is_null() || stride < width * 4 {
            return Err("libheif returned no pixels".to_owned());
        }
        let (width, height, stride) = (width as usize, height as usize, stride as usize);
        let mut pixels = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            let line = std::slice::from_raw_parts(plane.add(row * stride), width * 4);
            pixels.extend_from_slice(line);
        }
        RgbaImage::from_raw(width as u32, height as u32, pixels)
            .ok_or_else(|| "libheif returned a bad bitmap".to_owned())
    }
}

/// Which container to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Heic,
    Avif,
}

/// Encodes `image` (RGBA) as `format` into `path`. Needs an encoder plugin
/// (`libheif-plugin-x265` for HEIC, `libheif-plugin-aomenc` for AVIF).
pub fn encode(image: &RgbaImage, format: Format, path: &Path) -> Result<(), String> {
    let api = api().ok_or_else(|| MISSING.to_owned())?;
    let name = c_path(path)?;
    let (width, height) = image.dimensions();
    // SAFETY: as in `decode`; the plane is written only within its stride.
    unsafe {
        let context = Context(api, (api.context_alloc)());
        if context.1.is_null() {
            return Err("libheif could not start".to_owned());
        }
        let mut encoder = std::ptr::null_mut();
        let compression = match format {
            Format::Heic => COMPRESSION_HEVC,
            Format::Avif => COMPRESSION_AV1,
        };
        check((api.encoder_for_format)(
            context.1,
            compression,
            &mut encoder,
        ))
        .map_err(|_| {
            format!(
                "Lulo cannot save this format: no {} encoder is installed ({}).",
                if format == Format::Heic {
                    "HEIC"
                } else {
                    "AVIF"
                },
                if format == Format::Heic {
                    "libheif-plugin-x265"
                } else {
                    "libheif-plugin-aomenc"
                }
            )
        })?;
        struct Encoder<'a>(&'a Api, Ptr);
        impl Drop for Encoder<'_> {
            fn drop(&mut self) {
                // SAFETY: released once.
                unsafe { (self.0.encoder_release)(self.1) }
            }
        }
        let encoder = Encoder(api, encoder);
        check((api.set_quality)(encoder.1, 90))?;
        let mut picture = std::ptr::null_mut();
        check((api.image_create)(
            width as c_int,
            height as c_int,
            COLORSPACE_RGB,
            CHROMA_INTERLEAVED_RGBA,
            &mut picture,
        ))?;
        let picture = Picture(api, picture);
        check((api.image_add_plane)(
            picture.1,
            CHANNEL_INTERLEAVED,
            width as c_int,
            height as c_int,
            8,
        ))?;
        let mut stride: c_int = 0;
        let plane = (api.plane)(picture.1, CHANNEL_INTERLEAVED, &mut stride);
        if plane.is_null() || (stride as usize) < width as usize * 4 {
            return Err("libheif gave no pixel plane".to_owned());
        }
        for row in 0..height as usize {
            let source = &image.as_raw()[row * width as usize * 4..(row + 1) * width as usize * 4];
            std::ptr::copy_nonoverlapping(
                source.as_ptr(),
                plane.add(row * stride as usize),
                width as usize * 4,
            );
        }
        let mut handle = std::ptr::null_mut();
        check((api.encode_image)(
            context.1,
            picture.1,
            encoder.1,
            std::ptr::null(),
            &mut handle,
        ))?;
        if !handle.is_null() {
            (api.handle_release)(handle);
        }
        check((api.write_to_file)(context.1, name.as_ptr()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_avif_picture_round_trips_through_libheif_when_it_is_installed() {
        // Skipped where libheif or its AV1 encoder is absent (CI images
        // install both; see the packaging dependencies).
        if !available() {
            return;
        }
        let mut picture = RgbaImage::new(64, 48);
        for (x, y, pixel) in picture.enumerate_pixels_mut() {
            *pixel = image::Rgba([(x * 4) as u8, (y * 5) as u8, 90, 255]);
        }
        let directory =
            std::env::temp_dir().join(format!("rmac-preview-heif-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("picture.avif");
        if encode(&picture, Format::Avif, &path).is_err() {
            let _ = std::fs::remove_dir_all(&directory);
            return;
        }
        let decoded = decode(&path).unwrap();
        assert_eq!(decoded.dimensions(), (64, 48));
        // Lossy: a pixel is near what was written.
        let original = picture.get_pixel(32, 24).0;
        let round = decoded.get_pixel(32, 24).0;
        assert!((i32::from(original[0]) - i32::from(round[0])).abs() < 24);
        assert!((i32::from(original[1]) - i32::from(round[1])).abs() < 24);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_missing_or_garbage_file_is_an_error_not_a_crash() {
        if !available() {
            return;
        }
        assert!(decode(Path::new("/nonexistent/picture.heic")).is_err());
        let path =
            std::env::temp_dir().join(format!("rmac-preview-garbage-{}.heic", std::process::id()));
        std::fs::write(&path, b"not a picture at all").unwrap();
        assert!(decode(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
