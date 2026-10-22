use image::{ImageDecoder, ImageEncoder};

const JPEG_SEGMENT_PAYLOAD_LIMIT: usize = 65_533;
const JPEG_XMP_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

pub(crate) fn move_png_xmp_before_idat(bytes: &mut [u8]) -> Result<(), String> {
    let mut offset = 8;
    let mut insertion = None;
    while offset < bytes.len() {
        let remaining = &bytes[offset..];
        let header = remaining.get(..8).ok_or("Truncated PNG chunk header")?;
        let length = usize::try_from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]))
        .map_err(|error| error.to_string())?;
        let chunk_length = length.checked_add(12).ok_or("PNG chunk is too large")?;
        let chunk = remaining.get(..chunk_length).ok_or("Truncated PNG chunk")?;
        let kind = &header[4..8];
        if kind == b"IEND" {
            break;
        }
        if kind == b"IDAT" && insertion.is_none() {
            insertion = Some(offset);
        }
        if kind == b"iTXt"
            && chunk[8..chunk_length - 4].starts_with(b"XML:com.adobe.xmp\0")
            && let Some(start) = insertion
        {
            // The image decoder only reads metadata before the first IDAT.
            bytes[start..offset + chunk_length].rotate_right(chunk_length);
            insertion = Some(start + chunk_length);
        }
        offset += chunk_length;
    }
    Ok(())
}

#[derive(Default)]
pub(crate) struct PhotoMetadata {
    exif: Option<Vec<u8>>,
    icc: Option<Vec<u8>>,
    xmp: Option<Vec<u8>>,
}

impl PhotoMetadata {
    pub(crate) fn can_encode_jpeg(&self) -> bool {
        self.exif
            .as_ref()
            .is_none_or(|exif| exif.len() <= JPEG_SEGMENT_PAYLOAD_LIMIT - b"Exif\0\0".len())
            && self
                .xmp
                .as_ref()
                .is_none_or(|xmp| xmp.len() <= JPEG_SEGMENT_PAYLOAD_LIMIT - JPEG_XMP_HEADER.len())
            && self
                .icc
                .as_ref()
                .is_none_or(|icc| icc.len() <= (JPEG_SEGMENT_PAYLOAD_LIMIT - 14) * 255)
    }

    pub(crate) fn read(decoder: &mut impl ImageDecoder) -> Result<Self, String> {
        Ok(Self {
            exif: decoder.exif_metadata().map_err(|error| error.to_string())?,
            icc: decoder.icc_profile().map_err(|error| error.to_string())?,
            xmp: decoder.xmp_metadata().map_err(|error| error.to_string())?,
        })
    }

    pub(crate) fn apply(&self, encoder: &mut impl ImageEncoder) -> Result<(), String> {
        if let Some(exif) = &self.exif {
            encoder
                .set_exif_metadata(exif.clone())
                .map_err(|error| error.to_string())?;
        }
        if let Some(icc) = &self.icc {
            encoder
                .set_icc_profile(icc.clone())
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(crate) fn write_png_xmp(&self, png: &mut Vec<u8>) -> Result<(), String> {
        let Some(xmp) = &self.xmp else {
            return Ok(());
        };
        let mut chunk = b"iTXtXML:com.adobe.xmp\0\0\0\0\0".to_vec();
        chunk.extend_from_slice(xmp);
        let length = u32::try_from(chunk.len() - 4).map_err(|error| error.to_string())?;
        let checksum = crc32(&chunk);
        chunk.extend_from_slice(&checksum.to_be_bytes());
        chunk.splice(..0, length.to_be_bytes());
        // Place XMP after IHDR and before IDAT so header-only metadata readers can find it.
        png.splice(33..33, chunk);
        Ok(())
    }

    pub(crate) fn write_jpeg_xmp(&self, jpeg: &mut Vec<u8>) -> Result<(), String> {
        let Some(xmp) = &self.xmp else {
            return Ok(());
        };
        let mut segment = JPEG_XMP_HEADER.to_vec();
        segment.extend_from_slice(xmp);
        let length = u16::try_from(segment.len() + 2)
            .map_err(|_| "XMP metadata is too large for JPEG".to_string())?;
        segment.splice(..0, length.to_be_bytes());
        segment.splice(..0, [0xff, 0xe1]);
        let jfif_end = 4 + usize::from(u16::from_be_bytes([jpeg[4], jpeg[5]]));
        jpeg.splice(jfif_end..jfif_end, segment);
        Ok(())
    }
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}
