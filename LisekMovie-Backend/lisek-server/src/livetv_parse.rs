//! Faza 20: parsing IPTV channel lists (M3U / M3U8 "extended" playlists)
//! and programme guides (XMLTV), as served by IPTV providers, ErsatzTV,
//! Tvheadend and friends.
use std::collections::HashMap;

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use lisek_db::livetv::{NewChannel, NewProgram};
use quick_xml::events::Event;

/// Most channels kept from one playlist.
pub const MAX_CHANNELS: usize = 5_000;
/// Most programmes kept from one guide.
pub const MAX_PROGRAMS: usize = 300_000;

/// Stream URL schemes a channel may use. Anything else (`file:`, `concat:`,
/// `pipe:` …) would let a playlist make FFmpeg read the server's own files.
pub const STREAM_SCHEMES: &[&str] = &["http", "https", "rtmp", "rtmps", "rtsp", "udp", "rtp"];

pub fn allowed_stream_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else { return false };
    !rest.is_empty() && STREAM_SCHEMES.contains(&scheme.to_ascii_lowercase().as_str())
}

/// `key="value"` pairs of an `#EXTINF` line (before the title comma).
fn attributes(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut rest = text;
    while let Some(eq) = rest.find("=\"") {
        let key_start = rest[..eq].rfind(|c: char| c.is_whitespace()).map_or(0, |i| i + 1);
        let key = rest[key_start..eq].trim().to_ascii_lowercase();
        let value_start = eq + 2;
        let Some(len) = rest[value_start..].find('"') else { break };
        let value = rest[value_start..value_start + len].trim().to_string();
        if !key.is_empty() && !value.is_empty() {
            out.insert(key, value);
        }
        rest = &rest[value_start + len + 1..];
    }
    out
}

/// An `#EXTINF` line split at the first comma outside quotes: the
/// duration and attributes, then the title.
fn split_extinf(line: &str) -> (&str, &str) {
    let mut quoted = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => return (&line[..i], line[i + 1..].trim()),
            _ => {}
        }
    }
    (line, "")
}

fn non_empty(value: Option<&String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Channels of an extended M3U playlist, in order. Entries with a stream
/// scheme outside `STREAM_SCHEMES` are skipped.
pub fn parse_m3u(text: &str) -> Vec<NewChannel> {
    let mut channels: Vec<NewChannel> = Vec::new();
    let mut keys: HashMap<String, usize> = HashMap::new();
    let mut pending: Option<(HashMap<String, String>, String)> = None;
    let mut group: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}');
        if line.is_empty() {
            continue;
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            let (head, title) = split_extinf(info);
            pending = Some((attributes(head), title.to_string()));
            group = None;
            continue;
        }
        if let Some(value) = line.strip_prefix("#EXTGRP:") {
            group = Some(value.trim().to_string()).filter(|g| !g.is_empty());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        let Some((attrs, title)) = pending.take() else { continue };
        if !allowed_stream_url(line) || channels.len() >= MAX_CHANNELS {
            continue;
        }
        let name = if title.is_empty() {
            non_empty(attrs.get("tvg-name")).unwrap_or_else(|| format!("{}", channels.len() + 1))
        } else {
            title
        };
        let tvg_id = non_empty(attrs.get("tvg-id"));
        // Same tvg-id twice (one channel in two qualities): both stay.
        let base = tvg_id.clone().unwrap_or_else(|| line.to_string());
        let seen = keys.entry(base.clone()).or_insert(0);
        *seen += 1;
        let key = if *seen == 1 { base } else { format!("{base}#{seen}") };
        channels.push(NewChannel {
            key,
            tvg_id,
            name,
            number: non_empty(attrs.get("tvg-chno")).or_else(|| non_empty(attrs.get("channel-number"))),
            group_title: non_empty(attrs.get("group-title")).or_else(|| group.clone()),
            logo_url: non_empty(attrs.get("tvg-logo")).filter(|u| u.starts_with("http://") || u.starts_with("https://")),
            stream_url: line.to_string(),
        });
    }
    channels
}

/// An XMLTV time: `20261101203000 +0100`, without seconds, or without a
/// zone (then UTC).
pub fn parse_xmltv_time(value: &str) -> Option<DateTime<Utc>> {
    let value = value.trim();
    let (stamp, zone) = match value.split_once(' ') {
        Some((stamp, zone)) => (stamp, Some(zone.trim())),
        None => (value, None),
    };
    let digits: String = stamp.chars().take_while(|c| c.is_ascii_digit()).collect();
    let digits = match digits.len() {
        12 => format!("{digits}00"),
        14 => digits,
        _ => return None,
    };
    let naive = NaiveDateTime::parse_from_str(&digits, "%Y%m%d%H%M%S").ok()?;
    match zone {
        Some(zone) if !zone.is_empty() => {
            let offset = DateTime::parse_from_str(&format!("{digits} {zone}"), "%Y%m%d%H%M%S %z").ok()?;
            Some(offset.with_timezone(&Utc))
        }
        _ => Some(Utc.from_utc_datetime(&naive)),
    }
}

/// XMLTV channel ids → indexes into `channels`, by tvg-id first and then
/// by display name (case-insensitive) for playlists without tvg-ids.
fn channel_index(channels: &[NewChannel], xml_names: &HashMap<String, Vec<String>>) -> HashMap<String, Vec<usize>> {
    let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, channel) in channels.iter().enumerate() {
        if let Some(id) = &channel.tvg_id {
            by_id.entry(id.to_lowercase()).or_default().push(i);
        }
        by_name.entry(channel.name.to_lowercase()).or_default().push(i);
    }
    let mut out: HashMap<String, Vec<usize>> = HashMap::new();
    for (xml_id, names) in xml_names {
        if let Some(found) = by_id.get(&xml_id.to_lowercase()) {
            out.insert(xml_id.clone(), found.clone());
            continue;
        }
        if let Some(found) = names.iter().find_map(|n| by_name.get(&n.to_lowercase())) {
            out.insert(xml_id.clone(), found.clone());
        }
    }
    out
}

#[derive(Default)]
struct Programme {
    channel: String,
    start: Option<DateTime<Utc>>,
    stop: Option<DateTime<Utc>>,
    title: Option<String>,
    subtitle: Option<String>,
    description: Option<String>,
    category: Option<String>,
    icon: Option<String>,
}

fn attr(element: &quick_xml::events::BytesStart<'_>, name: &[u8]) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

/// Programmes of an XMLTV guide for `channels`, those ending after `from`
/// and starting before `to`.
pub fn parse_xmltv(
    xml: &[u8],
    channels: &[NewChannel],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<NewProgram>, String> {
    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buf = Vec::new();
    // Text of the element being read; quick-xml hands it over in pieces
    // around entity references (`&amp;`, `&#347;`).
    let mut text_buf = String::new();
    let mut names: HashMap<String, Vec<String>> = HashMap::new();
    let mut raw: Vec<Programme> = Vec::new();
    let mut current_channel: Option<String> = None;
    let mut current: Option<Programme> = None;
    let mut field: Option<&'static str> = None;
    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => return Err(format!("XMLTV is not valid XML: {e}")),
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                match e.name().as_ref() {
                    b"channel" => current_channel = attr(&e, b"id"),
                    b"display-name" if current_channel.is_some() => {
                        field = Some("display-name");
                        text_buf.clear();
                    }
                    b"programme" if raw.len() < MAX_PROGRAMS => {
                        current = Some(Programme {
                            channel: attr(&e, b"channel").unwrap_or_default(),
                            start: attr(&e, b"start").and_then(|s| parse_xmltv_time(&s)),
                            stop: attr(&e, b"stop").and_then(|s| parse_xmltv_time(&s)),
                            ..Programme::default()
                        });
                    }
                    b"title" | b"sub-title" | b"desc" | b"category" if current.is_some() => {
                        field = match e.name().as_ref() {
                            b"title" => Some("title"),
                            b"sub-title" => Some("sub-title"),
                            b"desc" => Some("desc"),
                            _ => Some("category"),
                        };
                        text_buf.clear();
                    }
                    b"icon" => {
                        if let Some(program) = current.as_mut()
                            && program.icon.is_none()
                        {
                            program.icon = attr(&e, b"src").filter(|u| u.starts_with("http"));
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(text)) => {
                if field.is_some()
                    && let Ok(value) = text.decode()
                {
                    text_buf.push_str(&value);
                }
            }
            Ok(Event::CData(data)) => {
                if field.is_some()
                    && let Ok(value) = data.decode()
                {
                    text_buf.push_str(&value);
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                if field.is_some() {
                    if let Ok(Some(c)) = reference.resolve_char_ref() {
                        text_buf.push(c);
                    } else if let Ok(name) = reference.decode()
                        && let Some(value) = quick_xml::escape::resolve_predefined_entity(&name)
                    {
                        text_buf.push_str(value);
                    }
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"channel" => current_channel = None,
                b"programme" => {
                    if let Some(program) = current.take() {
                        raw.push(program);
                    }
                    field = None;
                }
                _ => {
                    if let Some(name) = field.take() {
                        let value = text_buf.trim().to_string();
                        text_buf.clear();
                        if value.is_empty() {
                            // Nothing to keep.
                        } else if name == "display-name" {
                            if let Some(id) = &current_channel {
                                names.entry(id.clone()).or_default().push(value);
                            }
                        } else if let Some(program) = current.as_mut() {
                            let slot = match name {
                                "title" => &mut program.title,
                                "sub-title" => &mut program.subtitle,
                                "desc" => &mut program.description,
                                _ => &mut program.category,
                            };
                            // The first one (usually the main language) wins.
                            if slot.is_none() {
                                *slot = Some(value);
                            }
                        }
                    }
                }
            },
            Ok(_) => {}
        }
        buf.clear();
    }
    // Guides without <channel> entries still match by id.
    for program in &raw {
        names.entry(program.channel.clone()).or_default();
    }
    let index = channel_index(channels, &names);
    let mut out = Vec::new();
    for program in raw {
        let (Some(start), Some(title)) = (program.start, program.title) else { continue };
        let stop = program.stop.unwrap_or(start + chrono::Duration::minutes(30));
        if stop <= from || start >= to || stop <= start {
            continue;
        }
        let Some(targets) = index.get(&program.channel) else { continue };
        for &channel in targets {
            out.push(NewProgram {
                channel,
                starts_at: start,
                ends_at: stop,
                title: title.clone(),
                subtitle: program.subtitle.clone(),
                description: program.description.clone(),
                category: program.category.clone(),
                icon_url: program.icon.clone(),
            });
        }
    }
    Ok(out)
}

/// Gzip-compressed guides (`guide.xml.gz`) are unpacked, others pass.
pub fn maybe_gunzip(bytes: Vec<u8>, limit: usize) -> Result<Vec<u8>, String> {
    use std::io::Read;
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return Ok(bytes);
    }
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes.as_slice())
        .take(limit as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("guide archive is damaged: {e}"))?;
    if out.len() > limit {
        return Err("guide is too large".to_string());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYLIST: &str = "#EXTM3U x-tvg-url=\"http://epg\"\n\
#EXTINF:-1 tvg-id=\"tvp1.pl\" tvg-name=\"TVP 1\" tvg-logo=\"https://logo/tvp1.png\" group-title=\"Ogólne\" tvg-chno=\"1\",TVP 1 HD\n\
http://iptv/tvp1.m3u8\n\
#EXTINF:-1 tvg-id=\"tvp1.pl\",TVP 1 SD\n\
http://iptv/tvp1-sd.ts\n\
#EXTINF:0,Kanał, z przecinkiem\n\
#EXTGRP:Lokalne\n\
#EXTVLCOPT:http-user-agent=VLC\n\
udp://239.0.0.1:1234\n\
#EXTINF:-1,Zły\n\
file:///etc/passwd\n\
#EXTINF:-1 tvg-logo=\"file:///x.png\",Bez id\n\
https://iptv/other\n";

    #[test]
    fn m3u_channels_with_attributes_groups_and_safe_urls() {
        let channels = parse_m3u(PLAYLIST);
        assert_eq!(channels.len(), 4, "the file: entry is dropped");
        let first = &channels[0];
        assert_eq!(first.name, "TVP 1 HD");
        assert_eq!(first.tvg_id.as_deref(), Some("tvp1.pl"));
        assert_eq!(first.number.as_deref(), Some("1"));
        assert_eq!(first.group_title.as_deref(), Some("Ogólne"));
        assert_eq!(first.logo_url.as_deref(), Some("https://logo/tvp1.png"));
        assert_eq!(first.key, "tvp1.pl");
        assert_eq!(channels[1].key, "tvp1.pl#2", "a second quality keeps its own key");
        assert_eq!(channels[2].name, "Kanał, z przecinkiem");
        assert_eq!(channels[2].group_title.as_deref(), Some("Lokalne"));
        assert_eq!(channels[2].stream_url, "udp://239.0.0.1:1234");
        assert_eq!(channels[3].key, "https://iptv/other");
        assert_eq!(channels[3].logo_url, None, "only http(s) logos");
    }

    #[test]
    fn xmltv_times() {
        let t = parse_xmltv_time("20261101203000 +0100").unwrap();
        assert_eq!(t.to_rfc3339(), "2026-11-01T19:30:00+00:00");
        assert_eq!(parse_xmltv_time("202611012030").unwrap().to_rfc3339(), "2026-11-01T20:30:00+00:00");
        assert!(parse_xmltv_time("tomorrow").is_none());
    }

    #[test]
    fn xmltv_programmes_match_channels_by_id_and_name() {
        let channels = parse_m3u(PLAYLIST);
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<tv>
  <channel id="tvp1.pl"><display-name>TVP 1</display-name></channel>
  <channel id="local-7"><display-name lang="pl">Kana&#322;, z przecinkiem</display-name></channel>
  <programme start="20261101180000 +0000" stop="20261101190000 +0000" channel="tvp1.pl">
    <title lang="pl">Teleexpress</title><title lang="en">News</title>
    <sub-title>Wydanie wieczorne</sub-title>
    <desc>Wiadomo&#347;ci &amp; pogoda</desc>
    <category>News</category>
    <icon src="https://img/te.jpg"/>
  </programme>
  <programme start="20261101190000 +0000" stop="20261101200000 +0000" channel="local-7"><title>Lokalne</title></programme>
  <programme start="20261001190000 +0000" stop="20261001200000 +0000" channel="local-7"><title>Stare</title></programme>
  <programme start="20261101190000 +0000" stop="20261101200000 +0000" channel="nieznany"><title>Nikt</title></programme>
</tv>"#;
        let from = parse_xmltv_time("20261101000000").unwrap();
        let to = parse_xmltv_time("20261108000000").unwrap();
        let programs = parse_xmltv(xml, &channels, from, to).unwrap();
        // Teleexpress on both TVP 1 entries, Lokalne on the named channel.
        assert_eq!(programs.len(), 3);
        let news = &programs[0];
        assert_eq!((news.channel, news.title.as_str()), (0, "Teleexpress"));
        assert_eq!(news.subtitle.as_deref(), Some("Wydanie wieczorne"));
        assert_eq!(news.description.as_deref(), Some("Wiadomości & pogoda"));
        assert_eq!(news.icon_url.as_deref(), Some("https://img/te.jpg"));
        assert_eq!(programs[1].channel, 1);
        assert_eq!((programs[2].channel, programs[2].title.as_str()), (2, "Lokalne"));
        assert!(parse_xmltv(b"<tv><programme", &channels, from, to).is_err() || parse_xmltv(b"<tv><programme", &channels, from, to).unwrap().is_empty());
    }

    #[test]
    fn gzip_guides() {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"<tv/>").unwrap();
        let packed = encoder.finish().unwrap();
        assert_eq!(maybe_gunzip(packed.clone(), 100).unwrap(), b"<tv/>");
        assert!(maybe_gunzip(packed, 2).is_err());
        assert_eq!(maybe_gunzip(b"<tv/>".to_vec(), 100).unwrap(), b"<tv/>");
    }

    #[test]
    fn stream_schemes() {
        assert!(allowed_stream_url("HTTP://a/b"));
        assert!(allowed_stream_url("rtmp://a/live"));
        assert!(!allowed_stream_url("file:///etc/passwd"));
        assert!(!allowed_stream_url("concat:http://a|http://b"));
        assert!(!allowed_stream_url("http://"));
    }
}
