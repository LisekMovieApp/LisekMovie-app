//! What a playback client can decode. Browsers report it themselves (the
//! web player probes `MediaSource.isTypeSupported` and friends); a client
//! that reports nothing gets the conservative preset of its family,
//! recognised from the User-Agent.

use lisek_core::media_probe::HdrFormat;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientFamily {
    Chrome,
    Edge,
    Firefox,
    Safari,
    /// Smart TVs and TV boxes (Tizen, webOS, Android TV, Fire TV).
    Tv,
    #[default]
    Unknown,
}

/// How the client plays HLS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HlsSupport {
    /// The media element plays `.m3u8` itself (Safari, TVs).
    Native,
    /// Through Media Source Extensions (hls.js).
    #[default]
    Mse,
    /// Only plain files; everything must be direct played.
    None,
}

/// Decoding abilities of one client. Codec names are FFmpeg codec names;
/// a `_10bit` suffix adds 10-bit decoding of that codec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ClientCapabilities {
    pub family: ClientFamily,
    /// Containers played from a plain URL: `mp4`, `webm`, `mkv`, `ts`.
    pub containers: Vec<String>,
    /// `h264`, `h264_10bit`, `hevc`, `hevc_10bit`, `av1`, `av1_10bit`,
    /// `vp9`, `vp9_10bit`, `mpeg2video`, `vc1`, `mpeg4`.
    pub video_codecs: Vec<String>,
    /// `aac`, `mp3`, `opus`, `flac`, `ac3`, `eac3`, `alac`, `dts`,
    /// `truehd`, `vorbis`.
    pub audio_codecs: Vec<String>,
    /// Channels the client outputs without downmixing (2 for most
    /// browsers, 6 or 8 with a surround setup).
    pub max_audio_channels: u32,
    /// `hdr10`, `hlg`, `dolby_vision`. Empty on SDR displays.
    pub hdr_formats: Vec<String>,
    pub hls: HlsSupport,
    /// Deinterlaces by itself (TVs do; browsers show combing).
    pub interlaced: bool,
    /// Picture size worth sending when re-encoding (the screen), 0 for no
    /// limit.
    pub max_width: u32,
    pub max_height: u32,
}

impl Default for ClientCapabilities {
    fn default() -> Self {
        Self::preset(ClientFamily::Unknown)
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

impl ClientCapabilities {
    /// What every member of `family` decodes. Deliberately narrow: HEVC,
    /// AC-3 and HDR depend on the device, so only a client that reports
    /// them gets them.
    pub fn preset(family: ClientFamily) -> Self {
        let base = Self {
            family,
            containers: strings(&["mp4"]),
            video_codecs: strings(&["h264"]),
            audio_codecs: strings(&["aac", "mp3"]),
            max_audio_channels: 2,
            hdr_formats: vec![],
            hls: HlsSupport::Mse,
            interlaced: false,
            max_width: 0,
            max_height: 0,
        };
        match family {
            ClientFamily::Chrome | ClientFamily::Edge | ClientFamily::Firefox => Self {
                containers: strings(&["mp4", "webm"]),
                video_codecs: strings(&["h264", "vp9", "av1"]),
                audio_codecs: strings(&["aac", "mp3", "opus", "flac", "vorbis"]),
                ..base
            },
            ClientFamily::Safari => Self {
                containers: strings(&["mp4"]),
                video_codecs: strings(&["h264", "hevc", "hevc_10bit"]),
                audio_codecs: strings(&["aac", "mp3", "alac", "flac", "ac3", "eac3"]),
                hls: HlsSupport::Native,
                ..base
            },
            ClientFamily::Tv => Self {
                containers: strings(&["mp4", "mkv", "ts"]),
                video_codecs: strings(&["h264", "hevc", "hevc_10bit"]),
                audio_codecs: strings(&["aac", "mp3", "ac3", "eac3"]),
                max_audio_channels: 6,
                hdr_formats: strings(&["hdr10", "hlg"]),
                hls: HlsSupport::Native,
                interlaced: true,
                ..base
            },
            ClientFamily::Unknown => base,
        }
    }

    /// The capabilities to plan with: what the client reported (bounded,
    /// so a request cannot grow without limit), or its family's preset.
    pub fn effective(reported: Option<Self>, user_agent: Option<&str>) -> Self {
        let family = user_agent.map(ClientFamily::from_user_agent).unwrap_or_default();
        match reported {
            Some(mut caps) => {
                if caps.family == ClientFamily::Unknown {
                    caps.family = family;
                }
                for list in [
                    &mut caps.containers,
                    &mut caps.video_codecs,
                    &mut caps.audio_codecs,
                    &mut caps.hdr_formats,
                ] {
                    list.truncate(32);
                    list.retain(|v| v.len() <= 32);
                    for value in list.iter_mut() {
                        value.make_ascii_lowercase();
                    }
                }
                caps.max_audio_channels = caps.max_audio_channels.clamp(1, 8);
                caps
            }
            None => Self::preset(family),
        }
    }

    pub fn video(&self, codec: &str) -> bool {
        self.video_codecs.iter().any(|c| c == codec)
    }
    pub fn audio(&self, codec: &str) -> bool {
        self.audio_codecs.iter().any(|c| c == codec)
    }
    pub fn container(&self, container: &str) -> bool {
        self.containers.iter().any(|c| c == container)
    }

    /// Whether the display shows `format` as HDR. Dolby Vision profiles 7
    /// and 8 also play on HDR10 displays through their base layer.
    pub fn shows_hdr(&self, format: HdrFormat) -> bool {
        let has = |name: &str| self.hdr_formats.iter().any(|f| f == name);
        match format {
            HdrFormat::Hdr10 => has("hdr10"),
            HdrFormat::Hlg => has("hlg"),
            HdrFormat::DolbyVision { profile } => {
                has("dolby_vision") || (profile != 5 && has("hdr10"))
            }
        }
    }
}

impl ClientFamily {
    pub fn from_user_agent(user_agent: &str) -> Self {
        let ua = user_agent.to_ascii_lowercase();
        let any = |needles: &[&str]| needles.iter().any(|n| ua.contains(n));
        if any(&["tizen", "web0s", "webos", "smart-tv", "smarttv", "android tv", "; aft", "bravia", "crkey"]) {
            ClientFamily::Tv
        } else if any(&["iphone", "ipad", "crios", "fxios"]) {
            // Every iOS browser is WebKit.
            ClientFamily::Safari
        } else if ua.contains("edg/") {
            ClientFamily::Edge
        } else if ua.contains("firefox/") {
            ClientFamily::Firefox
        } else if any(&["chrome/", "chromium/"]) {
            ClientFamily::Chrome
        } else if ua.contains("safari/") {
            ClientFamily::Safari
        } else {
            ClientFamily::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_families_from_user_agents() {
        let cases = [
            ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36", ClientFamily::Chrome),
            ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36 Edg/141.0", ClientFamily::Edge),
            ("Mozilla/5.0 (X11; Linux x86_64; rv:143.0) Gecko/20100101 Firefox/143.0", ClientFamily::Firefox),
            ("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15", ClientFamily::Safari),
            ("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/141.0 Mobile/15E148 Safari/604.1", ClientFamily::Safari),
            ("Mozilla/5.0 (SMART-TV; Linux; Tizen 8.0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/108.0 TV Safari/537.36", ClientFamily::Tv),
            ("Mozilla/5.0 (Web0S; Linux/SmartTV) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/94.0 Safari/537.36 WebAppManager", ClientFamily::Tv),
            ("curl/8.5", ClientFamily::Unknown),
        ];
        for (ua, family) in cases {
            assert_eq!(ClientFamily::from_user_agent(ua), family, "{ua}");
        }
    }

    #[test]
    fn reported_capabilities_win_and_are_bounded() {
        let reported = ClientCapabilities {
            family: ClientFamily::Unknown,
            video_codecs: (0..100).map(|i| format!("C{i}")).collect(),
            max_audio_channels: 99,
            ..ClientCapabilities::preset(ClientFamily::Unknown)
        };
        let caps = ClientCapabilities::effective(Some(reported), Some("Firefox/143.0"));
        assert_eq!(caps.family, ClientFamily::Firefox);
        assert_eq!(caps.video_codecs.len(), 32);
        assert!(caps.video("c0"), "codec names are normalised to lower case");
        assert_eq!(caps.max_audio_channels, 8);
        let preset = ClientCapabilities::effective(None, Some("Firefox/143.0"));
        assert!(preset.video("vp9") && !preset.video("hevc"));
    }

    #[test]
    fn dolby_vision_with_a_base_layer_counts_as_hdr10() {
        let caps = ClientCapabilities { hdr_formats: vec!["hdr10".into()], ..Default::default() };
        assert!(caps.shows_hdr(HdrFormat::DolbyVision { profile: 8 }));
        assert!(!caps.shows_hdr(HdrFormat::DolbyVision { profile: 5 }));
        assert!(!caps.shows_hdr(HdrFormat::Hlg));
    }
}
