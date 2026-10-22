use crate::photo_metadata::PhotoMetadata;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{DynamicImage, ImageFormat, ImageReader};
use std::io::Cursor;
use std::path::Path;

pub struct PreparedPhoto {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub extension: &'static str,
}

fn encode_under_limit(
    image: DynamicImage,
    limit: usize,
    metadata: &PhotoMetadata,
) -> Result<PreparedPhoto, String> {
    let mut current = image;
    loop {
        let mut png = Vec::new();
        let mut encoder = PngEncoder::new(&mut png);
        metadata.apply(&mut encoder)?;
        current
            .write_with_encoder(encoder)
            .map_err(|error| error.to_string())?;
        metadata.write_png_xmp(&mut png)?;
        if png.len() <= limit {
            return Ok(PreparedPhoto {
                bytes: png,
                mime: "image/png",
                extension: "png",
            });
        }
        if !current.color().has_alpha() && metadata.can_encode_jpeg() {
            let mut jpeg = Vec::new();
            let mut encoder = JpegEncoder::new_with_quality(&mut jpeg, 88);
            metadata.apply(&mut encoder)?;
            encoder
                .encode_image(&current)
                .map_err(|error| error.to_string())?;
            metadata.write_jpeg_xmp(&mut jpeg)?;
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

pub fn prepare_file(path: &Path, preserve_metadata: bool) -> Result<PreparedPhoto, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("Could not open photo: {error}"))?;
    prepare_bytes(bytes, preserve_metadata, 8 * 1024 * 1024)
}

fn prepare_bytes(
    bytes: Vec<u8>,
    preserve_metadata: bool,
    limit: usize,
) -> Result<PreparedPhoto, String> {
    let format = image::guess_format(&bytes).map_err(|error| error.to_string())?;
    if preserve_metadata && bytes.len() <= limit {
        let extension = match format {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpg",
            ImageFormat::Bmp => "bmp",
            ImageFormat::Gif => "gif",
            ImageFormat::WebP => "webp",
            ImageFormat::Tiff => "tiff",
            _ => return Err("Unsupported photo format".into()),
        };
        return Ok(PreparedPhoto {
            bytes,
            mime: format.to_mime_type(),
            extension,
        });
    }
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .map_err(|error| format!("Could not open photo: {error}"))?;
    let metadata = if preserve_metadata {
        PhotoMetadata::read(&mut decoder)?
    } else {
        PhotoMetadata::default()
    };
    let image = DynamicImage::from_decoder(decoder)
        .map_err(|error| format!("Could not open photo: {error}"))?;
    encode_under_limit(image, limit, &metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GenericImageView, ImageBuffer, Rgba};
    use image::{ImageDecoder, ImageEncoder, ImageReader};

    #[test]
    fn exif_larger_than_a_jpeg_segment_is_preserved_in_png() {
        let fixture = include_bytes!("../tests/fixtures/metadata-rgb.png");
        let image = image::load_from_memory(fixture).unwrap();
        let exif = vec![0; 70_000];
        let mut original = Vec::new();
        let mut encoder = PngEncoder::new(&mut original);
        encoder.set_exif_metadata(exif.clone()).unwrap();
        image.write_with_encoder(encoder).unwrap();
        assert!(original.len() > 75_000);

        let result = prepare_bytes(original, true, 75_000).unwrap();
        let mut output = ImageReader::new(Cursor::new(&result.bytes))
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();

        assert_eq!(result.mime, "image/png");
        assert!(result.bytes.len() <= 75_000);
        assert_eq!(output.exif_metadata().unwrap(), Some(exif));
        assert!(image::load_from_memory(&result.bytes).is_ok());
    }

    #[test]
    fn disabled_setting_strips_metadata_before_and_after_compression() {
        let original = include_bytes!("../tests/fixtures/metadata-rgb.png");
        for limit in [original.len(), 8_000] {
            let result = prepare_bytes(original.to_vec(), false, limit).unwrap();
            let mut output = ImageReader::new(Cursor::new(&result.bytes))
                .with_guessed_format()
                .unwrap()
                .into_decoder()
                .unwrap();

            assert!(result.bytes.len() <= limit);
            assert_eq!(output.xmp_metadata().unwrap(), None);
            assert_eq!(output.exif_metadata().unwrap(), None);
            assert_eq!(output.icc_profile().unwrap(), None);
            assert!(image::load_from_memory(&result.bytes).is_ok());
        }
    }

    #[test]
    fn original_at_exact_limit_is_preserved() {
        let original = include_bytes!("../tests/fixtures/metadata-rgba.png");

        let result = prepare_bytes(original.to_vec(), true, original.len()).unwrap();

        assert_eq!(result.bytes, original);
        assert_eq!(result.mime, "image/png");
        assert_eq!(result.extension, "png");
    }

    #[test]
    fn metadata_that_cannot_fit_is_reported_instead_of_removed() {
        let original = include_bytes!("../tests/fixtures/metadata-rgba.png");

        let result = prepare_bytes(original.to_vec(), true, 200);

        assert!(matches!(result, Err(error) if error == "Photo cannot fit Discord upload limit"));
    }

    #[test]
    fn jpeg_conversion_preserves_xmp_exif_and_icc_within_limit() {
        let original = include_bytes!("../tests/fixtures/metadata-rgb.png");
        let result = prepare_bytes(original.to_vec(), true, 10_000).unwrap();
        let mut source = ImageReader::new(Cursor::new(original))
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();
        let mut output = ImageReader::new(Cursor::new(&result.bytes))
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();

        assert_eq!(result.mime, "image/jpeg");
        assert!(result.bytes.len() <= 10_000);
        assert_eq!(&result.bytes[..4], &[0xff, 0xd8, 0xff, 0xe0]);
        assert_eq!(
            output.xmp_metadata().unwrap(),
            source.xmp_metadata().unwrap()
        );
        assert_eq!(
            output.exif_metadata().unwrap(),
            source.exif_metadata().unwrap()
        );
        assert_eq!(output.icc_profile().unwrap(), source.icc_profile().unwrap());
        assert_eq!(
            image::load_from_memory(&result.bytes).unwrap().dimensions(),
            (64, 64)
        );
    }

    #[test]
    fn compressed_png_preserves_xmp_exif_and_icc_within_limit() {
        let original = include_bytes!("../tests/fixtures/metadata-rgba.png");
        let result = prepare_bytes(original.to_vec(), true, 8_000).unwrap();
        let mut source = ImageReader::new(Cursor::new(original))
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();
        let mut output = ImageReader::new(Cursor::new(&result.bytes))
            .with_guessed_format()
            .unwrap()
            .into_decoder()
            .unwrap();

        assert_eq!(result.mime, "image/png");
        assert!(result.bytes.len() <= 8_000);
        assert_eq!(
            output.xmp_metadata().unwrap(),
            source.xmp_metadata().unwrap()
        );
        assert_eq!(
            output.exif_metadata().unwrap(),
            source.exif_metadata().unwrap()
        );
        assert_eq!(output.icc_profile().unwrap(), source.icc_profile().unwrap());
        let decoded = image::load_from_memory(&result.bytes).unwrap();
        assert_eq!(decoded.to_rgba8().get_pixel(0, 0)[3], 128);
    }

    #[test]
    fn preserves_original_jpeg_bytes_when_enabled() {
        let source = DynamicImage::new_rgb8(16, 16);
        let mut original = Vec::new();
        JpegEncoder::new_with_quality(&mut original, 90)
            .encode_image(&source)
            .unwrap();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("vrc-shot-relay-original-{stamp}.jpg"));
        std::fs::write(&path, &original).unwrap();

        let result = prepare_file(&path, true);
        let unchanged = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let result = result.unwrap();

        assert_eq!(result.bytes, original);
        assert_eq!(result.mime, "image/jpeg");
        assert_eq!(result.extension, "jpg");
        assert_eq!(unchanged, original);
    }

    #[test]
    fn keeps_transparency_and_stays_under_limit() {
        let source = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(64, 64, Rgba([1, 2, 3, 0])));

        let result = encode_under_limit(source, 10_000, &PhotoMetadata::default()).unwrap();
        let decoded = image::load_from_memory(&result.bytes).unwrap();

        assert_eq!(result.mime, "image/png");
        assert!(result.bytes.len() <= 10_000);
        assert_eq!(decoded.dimensions(), (64, 64));
        assert_eq!(decoded.to_rgba8().get_pixel(0, 0)[3], 0);
    }

    #[test]
    fn reports_when_even_smallest_image_cannot_fit() {
        let source = DynamicImage::ImageRgba8(ImageBuffer::from_pixel(1, 1, Rgba([1, 2, 3, 0])));

        let result = encode_under_limit(source, 1, &PhotoMetadata::default());

        assert!(result.is_err());
    }
}
