use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat};
use std::io::Cursor;
use std::path::Path;

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

pub fn prepare_file(path: &Path) -> Result<PreparedPhoto, String> {
    let image = image::open(path).map_err(|error| format!("Could not open photo: {error}"))?;
    encode_under_limit(image, 8 * 1024 * 1024)
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
    fn reports_when_even_smallest_image_cannot_fit() {
        let source = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(1, 1, Rgba([1, 2, 3, 0])));

        let result = encode_under_limit(source, 1);

        assert!(result.is_err());
    }
}
