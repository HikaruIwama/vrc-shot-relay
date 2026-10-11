use image::ImageDecoder;
use image::codecs::{jpeg::JpegDecoder, png::PngDecoder};
use std::fs::{self, File};
use std::io::BufReader;
use std::path::Path;
use time::{Date, Month, OffsetDateTime, Time, UtcOffset};

#[derive(Debug, PartialEq, Eq)]
pub struct WorldInfo {
    pub name: String,
    pub id: String,
}

pub fn parse_world_xmp(xml: &str) -> Option<WorldInfo> {
    let document = roxmltree::Document::parse(xml.trim_matches('\0')).ok()?;
    let namespace = "http://ns.vrchat.com/vrc/1.0/";
    let value = |name| {
        document
            .descendants()
            .find(|node| {
                node.tag_name().name() == name && node.tag_name().namespace() == Some(namespace)
            })
            .and_then(|node| node.text())
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    let name = value("WorldDisplayName")?;
    let id = value("WorldID")?;
    if !valid_world_id(id) {
        return None;
    }
    Some(WorldInfo {
        name: name.into(),
        id: id.into(),
    })
}

pub fn capture_time_from_name(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let mut parts = name.strip_prefix("VRChat_")?.split('_');
    let date = parts.next()?;
    let clock = parts.next()?;
    let date_parts: Vec<_> = date.split('-').collect();
    let clock_parts: Vec<_> = clock.split('-').collect();
    if date_parts.len() != 3 || clock_parts.len() != 3 {
        return None;
    }
    let year = date_parts[0].parse().ok()?;
    let month = Month::try_from(date_parts[1].parse::<u8>().ok()?).ok()?;
    let day = date_parts[2].parse().ok()?;
    Date::from_calendar_date(year, month, day).ok()?;
    let hour = clock_parts[0].parse().ok()?;
    let minute = clock_parts[1].parse().ok()?;
    let second = clock_parts[2].split('.').next()?.parse().ok()?;
    Time::from_hms(hour, minute, second).ok()?;
    Some(format!("{date} {hour:02}:{minute:02}:{second:02}"))
}

pub fn capture_time(path: &Path) -> Option<String> {
    capture_time_from_name(path).or_else(|| {
        let created = fs::metadata(path).ok()?.created().ok()?;
        let local = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        let time = OffsetDateTime::from(created).to_offset(local);
        Some(format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            time.year(),
            u8::from(time.month()),
            time.day(),
            time.hour(),
            time.minute(),
            time.second()
        ))
    })
}

pub fn read_world(path: &Path) -> Option<WorldInfo> {
    let file = File::open(path).ok()?;
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let xmp = match extension.as_str() {
        "png" => PngDecoder::new(BufReader::new(file))
            .ok()?
            .xmp_metadata()
            .ok()?,
        "jpg" | "jpeg" => JpegDecoder::new(BufReader::new(file))
            .ok()?
            .xmp_metadata()
            .ok()?,
        _ => None,
    }?;
    parse_world_xmp(&String::from_utf8(xmp).ok()?)
}

fn valid_world_id(id: &str) -> bool {
    let Some(uuid) = id.strip_prefix("wrld_") else {
        return false;
    };
    let segments: Vec<_> = uuid.split('-').collect();
    segments.len() == 5
        && segments.iter().zip([8, 4, 4, 4, 12]).all(|(segment, len)| {
            segment.len() == len && segment.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_world_without_exposing_author() {
        let xml = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:vrc="http://ns.vrchat.com/vrc/1.0/"><vrc:AuthorID>usr_private</vrc:AuthorID><vrc:WorldID>wrld_f100c62e-3121-4380-9452-d5601269f49a</vrc:WorldID><vrc:WorldDisplayName>Test World</vrc:WorldDisplayName></rdf:Description></rdf:RDF></x:xmpmeta>"#;

        let result = parse_world_xmp(xml);

        assert_eq!(
            result,
            Some(WorldInfo {
                name: "Test World".into(),
                id: "wrld_f100c62e-3121-4380-9452-d5601269f49a".into(),
            })
        );
    }

    #[test]
    fn rejects_invalid_world_id() {
        let xml = r#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:vrc="http://ns.vrchat.com/vrc/1.0/"><vrc:WorldID>https://evil.invalid/</vrc:WorldID><vrc:WorldDisplayName>World</vrc:WorldDisplayName></rdf:Description>"#;

        let result = parse_world_xmp(xml);

        assert_eq!(result, None);
    }

    #[test]
    fn reads_camera_timestamp_with_layer_suffix() {
        let path = Path::new("VRChat_2026-10-11_12-34-56.123_3840x2160_BACKGROUND.png");

        let result = capture_time_from_name(path);

        assert_eq!(result, Some("2026-10-11 12:34:56".into()));
    }
}
