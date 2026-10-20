//! Keyframe positions and the HLS segment map built from them.
//!
//! A copied video can only be cut at its own keyframes, and a VOD playlist
//! has to list every segment's length before the first one exists. Matroska
//! files store keyframe times in their `Cues` (an index of a few hundred
//! KB, usually at the end of the file), read here directly; other
//! containers fall back to [`scan_with_ffprobe`], which reads the whole
//! file once and is cached in the database.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;

use lisek_core::playback_settings::PlaybackSettings;

// Matroska element IDs (with their length marker bits, as written).
const EBML: u32 = 0x1A45_DFA3;
const SEGMENT: u32 = 0x1853_8067;
const SEEK_HEAD: u32 = 0x114D_9B74;
const SEEK: u32 = 0x4DBB;
const SEEK_ID: u32 = 0x53AB;
const SEEK_POSITION: u32 = 0x53AC;
const INFO: u32 = 0x1549_A966;
const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
const TRACKS: u32 = 0x1654_AE6B;
const TRACK_ENTRY: u32 = 0xAE;
const TRACK_NUMBER: u32 = 0xD7;
const TRACK_TYPE: u32 = 0x83;
const CUES: u32 = 0x1C53_BB6B;
const CUE_POINT: u32 = 0xBB;
const CUE_TIME: u32 = 0xB3;
const CUE_TRACK_POSITIONS: u32 = 0xB7;
const CUE_TRACK: u32 = 0xF7;
const CLUSTER: u32 = 0x1F43_B675;

/// Upper bounds for the metadata elements read into memory.
const MAX_ELEMENT: u64 = 64 << 20;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// EBML variable-length integer. `id`: keep the length marker (element IDs).
/// Returns (value, length, all-ones = unknown size).
fn vint(bytes: &[u8], id: bool) -> Option<(u64, usize, bool)> {
    let first = *bytes.first()?;
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || bytes.len() < len {
        return None;
    }
    let mut value = if id { first as u64 } else { (first as u64) & (0xFF >> len) };
    for b in &bytes[1..len] {
        value = (value << 8) | *b as u64;
    }
    let unknown = !id && value == (1u64 << (7 * len)) - 1;
    Some((value, len, unknown))
}

/// Element header at the start of `bytes`: (id, data size, header length).
fn header(bytes: &[u8]) -> Option<(u32, Option<u64>, usize)> {
    let (id, id_len, _) = vint(bytes, true)?;
    if id_len > 4 {
        return None;
    }
    let (size, size_len, unknown) = vint(&bytes[id_len..], false)?;
    Some((id as u32, (!unknown).then_some(size), id_len + size_len))
}

/// Child elements of an in-memory element body.
fn elements(body: &[u8]) -> impl Iterator<Item = (u32, &[u8])> {
    let mut rest = body;
    std::iter::from_fn(move || {
        let (id, size, header_len) = header(rest)?;
        let size = size? as usize;
        let data = rest.get(header_len..header_len + size)?;
        rest = &rest[header_len + size..];
        Some((id, data))
    })
}

fn uint(data: &[u8]) -> u64 {
    data.iter().take(8).fold(0, |v, b| (v << 8) | *b as u64)
}

fn read_header(file: &mut File) -> io::Result<Option<(u32, Option<u64>, u64)>> {
    let mut buf = [0u8; 12];
    let start = file.stream_position()?;
    let read = file.read(&mut buf)?;
    if read == 0 {
        return Ok(None);
    }
    let (id, size, len) = header(&buf[..read]).ok_or_else(|| invalid("bad element header"))?;
    file.seek(SeekFrom::Start(start + len as u64))?;
    Ok(Some((id, size, start + len as u64)))
}

fn read_body(file: &mut File, at: u64, size: u64) -> io::Result<Vec<u8>> {
    if size > MAX_ELEMENT {
        return Err(invalid("element too large"));
    }
    let mut body = vec![0u8; size as usize];
    file.seek(SeekFrom::Start(at))?;
    file.read_exact(&mut body)?;
    Ok(body)
}

/// Keyframe timestamps (seconds) of the first video track of a Matroska
/// file, from its Cues. `Ok(None)` when the file is not Matroska or has no
/// usable Cues. Blocking: reads a few small regions of the file.
pub fn matroska_keyframes(path: &Path) -> io::Result<Option<Vec<f64>>> {
    let mut file = File::open(path)?;
    let Some((EBML, Some(size), at)) = read_header(&mut file)? else {
        return Ok(None);
    };
    file.seek(SeekFrom::Start(at + size))?;
    let Some((SEGMENT, _, segment_start)) = read_header(&mut file)? else {
        return Ok(None);
    };

    let mut timestamp_scale = 1_000_000u64;
    let mut video_track = None;
    let mut cues_at = None;
    let mut seek_heads = vec![];
    let mut cues: Option<Vec<u8>> = None;

    let read_info = |body: &[u8], scale: &mut u64| {
        for (id, data) in elements(body) {
            if id == TIMESTAMP_SCALE {
                *scale = uint(data).max(1);
            }
        }
    };
    let find_video = |body: &[u8]| {
        elements(body).filter(|(id, _)| *id == TRACK_ENTRY).find_map(|(_, entry)| {
            let mut number = None;
            let mut kind = None;
            for (id, data) in elements(entry) {
                match id {
                    TRACK_NUMBER => number = Some(uint(data)),
                    TRACK_TYPE => kind = Some(uint(data)),
                    _ => {}
                }
            }
            (kind == Some(1)).then_some(number).flatten()
        })
    };
    let read_seek_head = |body: &[u8], cues_at: &mut Option<u64>, heads: &mut Vec<u64>| {
        for (_, seek) in elements(body).filter(|(id, _)| *id == SEEK) {
            let mut target = None;
            let mut position = None;
            for (id, data) in elements(seek) {
                match id {
                    SEEK_ID => target = Some(uint(data) as u32),
                    SEEK_POSITION => position = Some(uint(data)),
                    _ => {}
                }
            }
            match (target, position) {
                (Some(CUES), Some(p)) => *cues_at = Some(segment_start + p),
                (Some(SEEK_HEAD), Some(p)) => heads.push(segment_start + p),
                _ => {}
            }
        }
    };

    // Top-level elements up to the first cluster.
    file.seek(SeekFrom::Start(segment_start))?;
    while let Some((id, size, at)) = read_header(&mut file)? {
        if id == CLUSTER {
            break;
        }
        let Some(size) = size else { break };
        match id {
            SEEK_HEAD => read_seek_head(&read_body(&mut file, at, size)?, &mut cues_at, &mut seek_heads),
            INFO => read_info(&read_body(&mut file, at, size)?, &mut timestamp_scale),
            TRACKS => video_track = find_video(&read_body(&mut file, at, size)?),
            CUES => cues = Some(read_body(&mut file, at, size)?),
            _ => {}
        }
        file.seek(SeekFrom::Start(at + size))?;
    }
    // A second SeekHead (usually at the end) may hold the Cues position.
    for head in seek_heads {
        if cues_at.is_some() {
            break;
        }
        file.seek(SeekFrom::Start(head))?;
        if let Some((SEEK_HEAD, Some(size), at)) = read_header(&mut file)? {
            let mut ignored = vec![];
            read_seek_head(&read_body(&mut file, at, size)?, &mut cues_at, &mut ignored);
        }
    }
    if cues.is_none() {
        let Some(position) = cues_at else { return Ok(None) };
        file.seek(SeekFrom::Start(position))?;
        match read_header(&mut file)? {
            Some((CUES, Some(size), at)) => cues = Some(read_body(&mut file, at, size)?),
            _ => return Ok(None),
        }
    }
    let (Some(track), Some(cues)) = (video_track, cues) else {
        return Ok(None);
    };

    let mut times: Vec<f64> = elements(&cues)
        .filter(|(id, _)| *id == CUE_POINT)
        .filter_map(|(_, point)| {
            let mut time = None;
            let mut for_video = false;
            for (id, data) in elements(point) {
                match id {
                    CUE_TIME => time = Some(uint(data)),
                    CUE_TRACK_POSITIONS => {
                        for_video |= elements(data).any(|(id, d)| id == CUE_TRACK && uint(d) == track)
                    }
                    _ => {}
                }
            }
            time.filter(|_| for_video)
                .map(|t| t as f64 * timestamp_scale as f64 / 1e9)
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    Ok((times.len() >= 2).then_some(times))
}

/// Keyframe timestamps of video stream `stream_index` read by ffprobe from
/// every packet of the file: I/O bound, minutes for a large remux. Used
/// once per file for containers without an index; the result is stored.
pub async fn scan_with_ffprobe(
    path: &Path,
    stream_index: i32,
    config: &PlaybackSettings,
) -> Result<Vec<f64>, String> {
    let mut cmd = crate::transcode::command("ffprobe", config);
    cmd.args([
        "-v",
        "error",
        "-select_streams",
        &stream_index.to_string(),
        "-show_entries",
        "packet=pts_time,flags",
        "-of",
        "csv=p=0",
    ])
    .arg(path);
    let output = tokio::time::timeout(Duration::from_secs(3600), cmd.output())
        .await
        .map_err(|_| "keyframe_scan_timed_out")?
        .map_err(|_| "ffprobe_unavailable")?;
    if !output.status.success() {
        return Err("keyframe_scan_failed".into());
    }
    let mut times: Vec<f64> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let (pts, flags) = line.split_once(',')?;
            flags.starts_with('K').then(|| pts.parse::<f64>().ok()).flatten()
        })
        .filter(|t| t.is_finite())
        .collect();
    times.sort_by(f64::total_cmp);
    times.dedup();
    if times.len() < 2 {
        return Err("keyframe_scan_failed".into());
    }
    Ok(times)
}

/// HLS segments on the playlist timeline (seconds from the file's start).
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentMap {
    /// Start of each segment; `starts[0] == 0`.
    starts: Vec<f64>,
    duration: f64,
}

impl SegmentMap {
    /// Equal segments of `length` seconds (re-encoded video, keyframes
    /// forced on the grid).
    pub fn fixed(duration: f64, length: f64) -> Self {
        let count = (duration / length).ceil().max(1.0) as usize;
        let mut starts: Vec<f64> = (0..count).map(|i| i as f64 * length).collect();
        // A last sliver under a second joins the previous segment.
        if starts.len() > 1 && duration - starts[starts.len() - 1] < 1.0 {
            starts.pop();
        }
        Self { starts, duration }
    }

    /// Segments cut at keyframes, each at least `target` seconds long.
    /// `keyframes` are file timestamps; `origin` is the file's start time.
    /// `None` when the index is too sparse to stream from (a gap over a
    /// minute means it is not a real keyframe list).
    pub fn from_keyframes(keyframes: &[f64], origin: f64, duration: f64, target: f64) -> Option<Self> {
        let mut starts = vec![0.0];
        for &keyframe in keyframes {
            let t = keyframe - origin;
            if t <= 0.0 || t >= duration {
                continue;
            }
            if t - starts[starts.len() - 1] >= target - 1e-3 {
                starts.push(t);
            }
        }
        if starts.len() > 1 && duration - starts[starts.len() - 1] < 1.0 {
            starts.pop();
        }
        let map = Self { starts, duration };
        (map.longest() <= 60.0 && keyframes.len() >= 2).then_some(map)
    }

    pub fn len(&self) -> usize {
        self.starts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.starts.is_empty()
    }

    pub fn duration(&self) -> f64 {
        self.duration
    }

    pub fn start(&self, index: usize) -> f64 {
        self.starts.get(index).copied().unwrap_or(self.duration)
    }

    pub fn end(&self, index: usize) -> f64 {
        self.starts.get(index + 1).copied().unwrap_or(self.duration)
    }

    pub fn length(&self, index: usize) -> f64 {
        self.end(index) - self.start(index)
    }

    pub fn longest(&self) -> f64 {
        (0..self.len()).map(|i| self.length(i)).fold(0.0, f64::max)
    }

    /// The segment a fragment starting at `time` belongs to. A fragment
    /// within 50 ms before a boundary counts as starting at it (rounding
    /// between the index and the stream's own timestamps).
    pub fn segment_at(&self, time: f64) -> usize {
        let probe = time + 0.05;
        self.starts.partition_point(|s| *s <= probe).saturating_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vints_and_headers() {
        assert_eq!(vint(&[0x81], false), Some((1, 1, false)));
        assert_eq!(vint(&[0x40, 0x02], false), Some((2, 2, false)));
        assert_eq!(vint(&[0xFF], false), Some((127, 1, true)), "unknown size");
        assert_eq!(vint(&[0x1A, 0x45, 0xDF, 0xA3], true), Some((EBML as u64, 4, false)));
        assert_eq!(header(&[0x1F, 0x43, 0xB6, 0x75, 0x01, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]), Some((CLUSTER, None, 12)));
    }

    #[test]
    fn keyframe_segments_respect_the_target_and_merge_slivers() {
        let keyframes: Vec<f64> = (0..30).map(|i| i as f64 * 1.001).collect();
        let map = SegmentMap::from_keyframes(&keyframes, 0.0, 30.0, 4.0).unwrap();
        assert_eq!(map.start(1), 4.004);
        assert!(map.length(0) >= 4.0 && map.longest() < 5.1);
        assert_eq!(map.end(map.len() - 1), 30.0);
        assert!(map.length(map.len() - 1) >= 1.0);
        // The file's own start time is playlist zero.
        let shifted: Vec<f64> = keyframes.iter().map(|k| k + 600.0).collect();
        let from_shifted = SegmentMap::from_keyframes(&shifted, 600.0, 30.0, 4.0).unwrap();
        assert_eq!(from_shifted.len(), map.len());
        assert!((0..map.len()).all(|i| (from_shifted.start(i) - map.start(i)).abs() < 1e-9));
        // A single keyframe every two minutes is not an index to stream from.
        assert_eq!(SegmentMap::from_keyframes(&[0.0, 120.0], 0.0, 300.0, 4.0), None);
    }

    #[test]
    fn long_gops_make_long_segments() {
        let map = SegmentMap::from_keyframes(&[0.0, 10.0, 20.0, 30.0], 0.0, 35.0, 4.0).unwrap();
        assert_eq!(map.len(), 4);
        assert_eq!(map.length(0), 10.0);
    }

    #[test]
    fn fixed_segments_and_lookup() {
        let map = SegmentMap::fixed(10.5, 4.0);
        assert_eq!((map.len(), map.start(2), map.length(2)), (3, 8.0, 2.5));
        assert_eq!(SegmentMap::fixed(8.5, 4.0).len(), 2, "0.5 s sliver merged");
        assert_eq!(map.segment_at(0.0), 0);
        assert_eq!(map.segment_at(3.97), 1, "just before a boundary");
        assert_eq!(map.segment_at(7.9), 1);
        assert_eq!(map.segment_at(99.0), 2);
    }
}
