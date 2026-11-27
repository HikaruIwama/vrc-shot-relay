use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;
use std::path::Path;

/// Largest size, in bytes, that one prepared photo may have when uploaded to Discord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadLimit(usize);

impl UploadLimit {
    /// Accepts 1 to 100 MB, the range Discord allows for webhook uploads.
    pub fn from_megabytes(megabytes: u32) -> Result<Self, String> {
        if !(1..=100).contains(&megabytes) {
            return Err(format!(
                "max_upload_mb must be between 1 and 100, but was {megabytes}"
            ));
        }
        let megabytes = usize::try_from(megabytes).map_err(|error| error.to_string())?;
        Ok(Self(megabytes * 1024 * 1024))
    }

    pub fn bytes(self) -> usize {
        self.0
    }
}

pub struct PreparedPhoto {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub extension: &'static str,
}

pub fn encode_under_limit(image: DynamicImage, limit: usize) -> Result<PreparedPhoto, String> {
    let mut current = image;
    loop {
        let mut png = Cursor::new(Vec::new());
        current
            .write_to(&mut png, ImageFormat::Png)
            .map_err(|error| error.to_string())?;
        if png.get_ref().len() <= limit {
            return Ok(PreparedPhoto {
                bytes: png.into_inner(),
                mime: "image/png",
                extension: "png",
            });
        }
        if !current.color().has_alpha() {
            let mut jpeg = Vec::new();
            JpegEncoder::new_with_quality(&mut jpeg, 88)
                .encode_image(&current)
                .map_err(|error| error.to_string())?;
            if jpeg.len() <= limit {
                return Ok(PreparedPhoto {
                    bytes: jpeg,
                    mime: "image/jpeg",
                    extension: "jpg",
                });
            }
        }
        let width = (current.width() * 3 / 4).max(1);
        let height = (current.height() * 3 / 4).max(1);
        if width == current.width() && height == current.height() {
            return Err("Photo cannot fit Discord upload limit".into());
        }
        current = current.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
    }
}

pub fn prepare_file(path: &Path, limit: UploadLimit) -> Result<PreparedPhoto, String> {
    let image = image::open(path).map_err(|error| format!("Could not open photo: {error}"))?;
    encode_under_limit(image, limit.bytes())
}

pub fn capture_key(path: &Path) -> std::path::PathBuf {
    let Some(stem) = path.file_stem().and_then(|name| name.to_str()) else {
        return path.to_owned();
    };
    let parts: Vec<_> = stem.split('_').collect();
    if parts.len() < 4
        || parts[0] != "VRChat"
        || crate::metadata::capture_time_from_name(path).is_none()
    {
        return path.to_owned();
    }
    let Some((width, height)) = parts[3].split_once('x') else {
        return path.to_owned();
    };
    if width.parse::<u32>().is_err() || height.parse::<u32>().is_err() {
        return path.to_owned();
    }
    path.with_file_name(parts[..4].join("_"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, ImageBuffer, Rgba};

    #[test]
    fn keeps_transparency_and_stays_under_limit() {
        let source = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(64, 64, Rgba([1, 2, 3, 0])));

        let result = encode_under_limit(source, 10_000).unwrap();
        let decoded = image::load_from_memory(&result.bytes).unwrap();

        assert_eq!(result.mime, "image/png");
        assert!(result.bytes.len() <= 10_000);
        assert_eq!(decoded.dimensions(), (64, 64));
        assert_eq!(decoded.to_rgba8().get_pixel(0, 0)[3], 0);
    }

    #[test]
    fn upload_limit_accepts_1_to_100_megabytes() {
        let smallest = UploadLimit::from_megabytes(1).unwrap();
        let largest = UploadLimit::from_megabytes(100).unwrap();

        assert_eq!(smallest.bytes(), 1024 * 1024);
        assert_eq!(largest.bytes(), 100 * 1024 * 1024);
    }

    #[test]
    fn upload_limit_rejects_values_outside_discord_range() {
        let zero = UploadLimit::from_megabytes(0);
        let too_large = UploadLimit::from_megabytes(101);

        assert!(zero.is_err());
        assert!(too_large.is_err());
    }

    #[test]
    fn reports_when_even_smallest_image_cannot_fit() {
        let source = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(1, 1, Rgba([1, 2, 3, 0])));

        let result = encode_under_limit(source, 1);

        assert!(result.is_err());
    }
}
