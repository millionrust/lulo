//! Finder's Quick Actions (FILES-12, FILES-23): Rotate Left for images,
//! from the item menu's Quick Actions submenu and the gallery inspector's
//! button. An item with no action that applies gets no Quick Actions row,
//! rather than an empty one.
//!
//! The image is decoded with its EXIF orientation applied, turned a quarter
//! anticlockwise, encoded in its own format and swapped in atomically, all
//! on the blocking pool.

use super::*;

/// Formats Rotate Left can write back without losing frames.
const ROTATABLE: &[&str] = &["png", "jpg", "jpeg", "bmp", "tif", "tiff", "webp"];

pub(super) fn can_rotate(path: &Path, is_dir: bool) -> bool {
    !is_dir
        && path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| ROTATABLE.contains(&extension.to_ascii_lowercase().as_str()))
}

/// Rotate one image a quarter turn anticlockwise, in place.
pub(super) fn rotate_image_left(path: &Path) -> Result<(), String> {
    use image::ImageDecoder as _;
    use std::io::Write as _;

    let format = image::ImageFormat::from_path(path).map_err(|error| error.to_string())?;
    let mut decoder = image::ImageReader::open(path)
        .map_err(|error| error.to_string())?
        .with_guessed_format()
        .map_err(|error| error.to_string())?
        .into_decoder()
        .map_err(|error| error.to_string())?;
    let orientation = decoder.orientation().map_err(|error| error.to_string())?;
    let mut picture = image::DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
    picture.apply_orientation(orientation);
    let rotated = picture.rotate270();

    let parent = path
        .parent()
        .ok_or_else(|| "the image has no folder".to_owned())?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = parent.join(format!(".{name}.rotate-{}", uuid::Uuid::new_v4()));
    let written = (|| -> Result<(), String> {
        let file = std::fs::File::create(&temporary).map_err(|error| error.to_string())?;
        let mut writer = std::io::BufWriter::new(file);
        match format {
            image::ImageFormat::Jpeg => rotated
                .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
                    &mut writer,
                    95,
                ))
                .map_err(|error| error.to_string())?,
            _ => rotated
                .write_to(&mut writer, format)
                .map_err(|error| error.to_string())?,
        }
        writer.flush().map_err(|error| error.to_string())?;
        let file = writer.into_inner().map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        if let Ok(metadata) = std::fs::metadata(path) {
            let _ = std::fs::set_permissions(&temporary, metadata.permissions());
        }
        std::fs::rename(&temporary, path).map_err(|error| error.to_string())
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

impl FinderView {
    /// Whether Quick Actions has anything for the current selection.
    pub(super) fn selection_quick_actions(&self) -> bool {
        let paths = self.selected_entries();
        !paths.is_empty() && paths.iter().all(|entry| can_rotate(&entry.path, entry.is_dir))
    }

    fn selected_entries(&self) -> Vec<&Entry> {
        if self.trash_view || self.applications_view {
            return Vec::new();
        }
        self.selected
            .iter()
            .filter_map(|index| self.entries.get(*index))
            .collect()
    }

    /// Quick Actions ▸ Rotate Left.
    pub(super) fn rotate_selection_left(&mut self, cx: &mut Context<Self>) {
        self.menu_at = None;
        let paths = self
            .selected_entries()
            .into_iter()
            .filter(|entry| can_rotate(&entry.path, entry.is_dir))
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();
        if paths.is_empty() {
            return;
        }
        cx.spawn(async move |this, cx: &mut gpui::AsyncApp| {
            let failures = blocking::unblock(move || {
                paths
                    .iter()
                    .filter_map(|path| {
                        rotate_image_left(path).err().map(|error| {
                            format!(
                                "“{}” could not be rotated: {error}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            )
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let _ = this.update(cx, |this: &mut FinderView, cx| {
                if let Some(first) = failures.first() {
                    this.operation_error = Some(first.clone().into());
                }
                this.gen_thumbs(cx);
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_still_images_can_be_rotated() {
        assert!(can_rotate(Path::new("/a/photo.JPG"), false));
        assert!(can_rotate(Path::new("/a/shot.png"), false));
        assert!(!can_rotate(Path::new("/a/anim.gif"), false));
        assert!(!can_rotate(Path::new("/a/notes.txt"), false));
        assert!(!can_rotate(Path::new("/a/folder.png"), true));
    }

    #[test]
    fn rotate_left_turns_an_image_a_quarter_anticlockwise() {
        let root = std::env::temp_dir().join(format!("rmac-rotate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("wide.png");
        let mut picture = image::RgbaImage::new(3, 2);
        // The top-right pixel is red.
        picture.put_pixel(2, 0, image::Rgba([255, 0, 0, 255]));
        picture.save(&path).unwrap();

        rotate_image_left(&path).unwrap();

        let rotated = image::open(&path).unwrap().to_rgba8();
        assert_eq!(rotated.dimensions(), (2, 3));
        // Turned anticlockwise, the top-right corner becomes the top-left.
        assert_eq!(rotated.get_pixel(0, 0), &image::Rgba([255, 0, 0, 255]));
        let leftovers = std::fs::read_dir(&root).unwrap().count();
        assert_eq!(leftovers, 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
