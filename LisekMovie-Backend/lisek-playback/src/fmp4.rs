//! Just enough ISO-BMFF to cut FFmpeg's fragmented MP4 output into HLS
//! segments: top-level boxes from a stream, track timescales and codec
//! strings from `moov`, and each fragment's first video timestamp from
//! `moof` (`tfdt` + the first sample's composition offset).

use std::io;
use tokio::io::{AsyncRead, AsyncReadExt};

/// One top-level box, header included.
#[derive(Debug, Clone)]
pub struct Mp4Box {
    pub kind: [u8; 4],
    pub bytes: Vec<u8>,
}

/// Largest box accepted from FFmpeg (a fragment's `mdat` holds one GOP;
/// 4K remuxes at 100 Mbit/s with 10 s GOPs stay well below this).
const MAX_BOX: u64 = 1 << 30;

/// Reads the next top-level box; `None` at a clean end of stream.
pub async fn read_box<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Option<Mp4Box>> {
    let mut header = [0u8; 8];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let size32 = u32::from_be_bytes(header[0..4].try_into().unwrap()) as u64;
    let kind: [u8; 4] = header[4..8].try_into().unwrap();
    let (size, header_len) = match size32 {
        1 => {
            let mut large = [0u8; 8];
            reader.read_exact(&mut large).await?;
            (u64::from_be_bytes(large), 16)
        }
        0 => return Err(io::Error::new(io::ErrorKind::InvalidData, "box extends to end of stream")),
        n => (n, 8),
    };
    if size < header_len as u64 || size > MAX_BOX {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("bad box size {size}")));
    }
    let mut bytes = Vec::with_capacity(size as usize);
    bytes.extend_from_slice(&header);
    if header_len == 16 {
        bytes.extend_from_slice(&size.to_be_bytes());
    }
    let body = size - header_len as u64;
    let read = reader.take(body).read_to_end(&mut bytes).await?;
    if read as u64 != body {
        return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated box"));
    }
    Ok(Some(Mp4Box { kind, bytes }))
}

/// Children of a box body: `(type, payload)`.
fn children(body: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut rest = body;
    std::iter::from_fn(move || {
        if rest.len() < 8 {
            return None;
        }
        let size = u32::from_be_bytes(rest[0..4].try_into().ok()?) as usize;
        let kind: [u8; 4] = rest[4..8].try_into().ok()?;
        let (size, header) = match size {
            1 if rest.len() >= 16 => (u64::from_be_bytes(rest[8..16].try_into().ok()?) as usize, 16),
            0 => (rest.len(), 8),
            n => (n, 8),
        };
        if size < header || size > rest.len() {
            return None;
        }
        let payload = &rest[header..size];
        rest = &rest[size..];
        Some((kind, payload))
    })
}

fn child<'a>(body: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(body).find(|(k, _)| k == kind).map(|(_, p)| p)
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}
fn be_u64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: u32,
    pub timescale: u32,
    /// `vide`, `soun` ...
    pub handler: [u8; 4],
    /// RFC 6381 codec string (`avc1.640028`, `mp4a.40.2`), when known.
    pub codec: Option<String>,
}

/// Tracks of an init segment (`ftyp` + `moov`).
pub fn parse_init(moov_box: &[u8]) -> Vec<Track> {
    let Some(moov) = moov_box.get(8..) else { return vec![] };
    children(moov)
        .filter(|(k, _)| k == b"trak")
        .filter_map(|(_, trak)| {
            let tkhd = child(trak, b"tkhd")?;
            let id = if tkhd.first() == Some(&1) { be_u32(tkhd, 20)? } else { be_u32(tkhd, 12)? };
            let mdia = child(trak, b"mdia")?;
            let mdhd = child(mdia, b"mdhd")?;
            let timescale = if mdhd.first() == Some(&1) { be_u32(mdhd, 20)? } else { be_u32(mdhd, 12)? };
            let handler: [u8; 4] = child(mdia, b"hdlr")?.get(8..12)?.try_into().ok()?;
            let stsd = child(child(child(mdia, b"minf")?, b"stbl")?, b"stsd")?;
            Some(Track { id, timescale, handler, codec: codec_string(stsd.get(8..)?) })
        })
        .collect()
}

/// The codec string of the first sample entry in an `stsd` body.
fn codec_string(entries: &[u8]) -> Option<String> {
    let (kind, entry) = children(entries).next()?;
    let visual = |entry: &[u8]| entry.get(78..).map(|b| b.to_vec());
    let audio = |entry: &[u8]| entry.get(28..).map(|b| b.to_vec());
    match &kind {
        b"avc1" | b"avc3" => {
            let config = visual(entry)?;
            let avcc = child(&config, b"avcC")?;
            Some(format!(
                "{}.{:02x}{:02x}{:02x}",
                std::str::from_utf8(&kind).ok()?,
                avcc.get(1)?,
                avcc.get(2)?,
                avcc.get(3)?
            ))
        }
        b"hvc1" | b"hev1" => {
            let config = visual(entry)?;
            let hvcc = child(&config, b"hvcC")?;
            let b1 = *hvcc.get(1)?;
            let space = ["", "A", "B", "C"][(b1 >> 6) as usize];
            let tier = if b1 & 0x20 != 0 { 'H' } else { 'L' };
            let profile = b1 & 0x1f;
            let compat = be_u32(hvcc, 2)?.reverse_bits();
            let constraints = hvcc.get(6..12)?;
            let level = *hvcc.get(12)?;
            let mut s = format!(
                "{}.{space}{profile}.{compat:X}.{tier}{level}",
                std::str::from_utf8(&kind).ok()?
            );
            let used = constraints.iter().rposition(|b| *b != 0).map_or(0, |i| i + 1);
            for byte in &constraints[..used] {
                s.push_str(&format!(".{byte:X}"));
            }
            Some(s)
        }
        b"vp09" => {
            let config = visual(entry)?;
            let vpcc = child(&config, b"vpcC")?;
            // version/flags (4), profile, level, bitDepth(4 bits) ...
            Some(format!(
                "vp09.{:02}.{:02}.{:02}",
                vpcc.get(4)?,
                vpcc.get(5)?,
                vpcc.get(6)? >> 4
            ))
        }
        b"av01" => {
            let config = visual(entry)?;
            let av1c = child(&config, b"av1C")?;
            let profile = av1c.get(1)? >> 5;
            let level = av1c.get(1)? & 0x1f;
            let flags = av1c.get(2)?;
            let tier = if flags & 0x80 != 0 { 'H' } else { 'M' };
            let depth = if flags & 0x40 != 0 { if flags & 0x20 != 0 { 12 } else { 10 } } else { 8 };
            Some(format!("av01.{profile}.{level:02}{tier}.{depth:02}"))
        }
        b"mp4a" => {
            let config = audio(entry)?;
            let esds = child(&config, b"esds")?;
            mp4a_codec(esds.get(4..)?)
        }
        b"ac-3" => Some("ac-3".into()),
        b"ec-3" => Some("ec-3".into()),
        b"Opus" => Some("opus".into()),
        b"fLaC" => Some("fLaC".into()),
        b"alac" => Some("alac".into()),
        _ => None,
    }
}

/// `mp4a.40.2` from an ES descriptor.
fn mp4a_codec(descriptors: &[u8]) -> Option<String> {
    // Walk ES_Descriptor (0x03) -> DecoderConfigDescriptor (0x04) ->
    // DecoderSpecificInfo (0x05).
    fn descriptor(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
        let tag = *bytes.first()?;
        let mut len = 0usize;
        let mut at = 1;
        for _ in 0..4 {
            let b = *bytes.get(at)?;
            at += 1;
            len = (len << 7) | (b & 0x7f) as usize;
            if b & 0x80 == 0 {
                break;
            }
        }
        let body = bytes.get(at..at + len)?;
        Some((tag, body, &bytes[at + len..]))
    }
    let (tag, es, _) = descriptor(descriptors)?;
    if tag != 0x03 {
        return None;
    }
    let flags = *es.get(2)?;
    let mut at = 3;
    if flags & 0x80 != 0 {
        at += 2;
    }
    if flags & 0x40 != 0 {
        at += 1 + *es.get(at)? as usize;
    }
    if flags & 0x20 != 0 {
        at += 2;
    }
    let (tag, config, _) = descriptor(es.get(at..)?)?;
    if tag != 0x04 {
        return None;
    }
    let object_type = *config.first()?;
    if object_type != 0x40 {
        return Some(format!("mp4a.{object_type:02X}"));
    }
    let (tag, specific, _) = descriptor(config.get(13..)?)?;
    if tag != 0x05 {
        return Some("mp4a.40.2".into());
    }
    let mut aot = specific.first()? >> 3;
    if aot == 31 {
        aot = 32 + (((specific.first()? & 0x07) << 3) | (specific.get(1)? >> 5));
    }
    Some(format!("mp4a.40.{aot}"))
}

/// When the samples of `track` in a `moof` box start (presentation time
/// of the first sample, seconds, before any offset) and how long they
/// last; `None` when the fragment has no sample of that track.
pub fn fragment_timing(moof_box: &[u8], track: &Track) -> Option<(f64, f64)> {
    let moof = moof_box.get(8..)?;
    children(moof)
        .filter(|(k, _)| k == b"traf")
        .find_map(|(_, traf)| {
            let tfhd = child(traf, b"tfhd")?;
            if be_u32(tfhd, 4)? != track.id {
                return None;
            }
            let tfhd_flags = be_u32(tfhd, 0)? & 0x00ff_ffff;
            let mut at = 8;
            if tfhd_flags & 0x1 != 0 {
                at += 8; // base data offset
            }
            if tfhd_flags & 0x2 != 0 {
                at += 4; // sample description index
            }
            let default_duration = if tfhd_flags & 0x8 != 0 { be_u32(tfhd, at)? } else { 0 };
            let tfdt = child(traf, b"tfdt")?;
            let decode = if tfdt.first() == Some(&1) { be_u64(tfdt, 4)? } else { be_u32(tfdt, 4)? as u64 };
            let trun = child(traf, b"trun")?;
            let version = *trun.first()?;
            let flags = be_u32(trun, 0)? & 0x00ff_ffff;
            let count = be_u32(trun, 4)? as usize;
            if count == 0 {
                return None;
            }
            let mut at = 8;
            if flags & 0x1 != 0 {
                at += 4; // data offset
            }
            if flags & 0x4 != 0 {
                at += 4; // first sample flags
            }
            let fields = [0x100, 0x200, 0x400, 0x800].iter().filter(|f| flags & *f != 0).count();
            let mut total: u64 = 0;
            let mut first_offset = 0i64;
            for sample in 0..count.min(1 << 20) {
                let mut field = at + sample * fields * 4;
                if flags & 0x100 != 0 {
                    total += be_u32(trun, field)? as u64;
                    field += 4;
                } else {
                    total += default_duration as u64;
                }
                if sample == 0 && flags & 0x800 != 0 {
                    let skip = [0x200, 0x400].iter().filter(|f| flags & *f != 0).count() * 4;
                    let raw = be_u32(trun, field + skip)?;
                    first_offset = if version == 1 { raw as i32 as i64 } else { raw as i64 };
                }
            }
            let scale = track.timescale.max(1) as f64;
            Some(((decode as i64 + first_offset) as f64 / scale, total as f64 / scale))
        })
}

/// The first presentation time of `track` in a fragment (see
/// [`fragment_timing`]).
pub fn fragment_start(moof_box: &[u8], track: &Track) -> Option<f64> {
    fragment_timing(moof_box, track).map(|(start, _)| start)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    #[test]
    fn fragment_start_adds_the_composition_offset() {
        let tfhd = boxed(b"tfhd", &[0, 0, 0, 0, 0, 0, 0, 1]);
        let mut tfdt_body = vec![1, 0, 0, 0];
        tfdt_body.extend_from_slice(&16000u64.to_be_bytes()); // 1.0 s at 16 kHz
        let tfdt = boxed(b"tfdt", &tfdt_body);
        // version 1, flags: data offset + duration + size + flags + cto
        let mut trun_body = vec![1, 0, 0x0f, 0x01];
        trun_body.extend_from_slice(&1u32.to_be_bytes()); // one sample
        trun_body.extend_from_slice(&0u32.to_be_bytes()); // data offset
        trun_body.extend_from_slice(&667u32.to_be_bytes()); // duration
        trun_body.extend_from_slice(&1000u32.to_be_bytes()); // size
        trun_body.extend_from_slice(&0u32.to_be_bytes()); // flags
        trun_body.extend_from_slice(&1334i32.to_be_bytes()); // cto
        let traf = boxed(b"traf", &[tfhd, tfdt, boxed(b"trun", &trun_body)].concat());
        let moof = boxed(b"moof", &traf);
        let track = Track { id: 1, timescale: 16000, handler: *b"vide", codec: None };
        let start = fragment_start(&moof, &track).unwrap();
        assert!((start - (16000.0 + 1334.0) / 16000.0).abs() < 1e-9);
        let (_, duration) = fragment_timing(&moof, &track).unwrap();
        assert!((duration - 667.0 / 16000.0).abs() < 1e-9);
        let other = Track { id: 2, ..track };
        assert_eq!(fragment_start(&moof, &other), None);
    }

    #[test]
    fn aac_codec_string_from_esds() {
        // ES_Descriptor with DecoderConfig (0x40, AAC) and AudioSpecificConfig LC.
        let esds = [
            0x03, 0x19, 0x00, 0x01, 0x00, 0x04, 0x11, 0x40, 0x15, 0x00, 0x00, 0x00, 0x00, 0x01, 0xf4,
            0x00, 0x00, 0x01, 0xf4, 0x00, 0x05, 0x02, 0x11, 0x90, 0x06, 0x01, 0x02,
        ];
        assert_eq!(mp4a_codec(&esds).as_deref(), Some("mp4a.40.2"));
    }

    #[tokio::test]
    async fn reads_boxes_until_the_stream_ends() {
        let data = [boxed(b"ftyp", b"isom"), boxed(b"free", b"")].concat();
        let mut reader = &data[..];
        assert_eq!(&read_box(&mut reader).await.unwrap().unwrap().kind, b"ftyp");
        assert_eq!(read_box(&mut reader).await.unwrap().unwrap().bytes.len(), 8);
        assert!(read_box(&mut reader).await.unwrap().is_none());
        let mut truncated = &data[..10];
        assert!(read_box(&mut truncated).await.is_err());
    }
}
