//! Persisted playback policy shared by the database, API and FFmpeg planner.
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(default, deny_unknown_fields)]
pub struct PlaybackSettings {
    pub min_resume_percentage: f64,
    pub max_resume_percentage: f64,
    pub min_resume_duration_seconds: u32,
    pub streaming_bitrate_limit_mbps: Option<f64>,
    pub local_networks: Vec<String>,
    pub trusted_proxies: Vec<String>,
    pub ffmpeg_path: String,
    pub transcoding_temp_path: String,
    pub hardware_acceleration: String,
    pub hardware_encoding: bool,
    pub hardware_decoding_codecs: Vec<String>,
    pub decode_hevc_10bit: bool,
    pub decode_vp9_10bit: bool,
    pub enhanced_nvdec: bool,
    pub encoding_threads: u16,
    pub encoder_preset: String,
    pub h264_crf: u8,
    pub max_output_height: u32,
    pub video_bitrate_kbps: u32,
    pub audio_bitrate_kbps: u32,
    pub audio_vbr: bool,
    pub downmix_audio_boost: f64,
    pub max_muxing_queue_size: u32,
    pub deinterlace_method: String,
    pub deinterlace_double_rate: bool,
    pub tonemapping: bool,
    pub tonemapping_algorithm: String,
    pub tonemapping_desat: f64,
    pub tonemapping_peak: f64,
    pub tonemapping_param: Option<f64>,
}
impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            min_resume_percentage: 5.0,
            max_resume_percentage: 90.0,
            min_resume_duration_seconds: 300,
            streaming_bitrate_limit_mbps: None,
            local_networks: [
                "127.0.0.0/8",
                "::1/128",
                "10.0.0.0/8",
                "172.16.0.0/12",
                "192.168.0.0/16",
                "fc00::/7",
            ]
            .map(String::from)
            .to_vec(),
            trusted_proxies: vec![],
            ffmpeg_path: String::new(),
            transcoding_temp_path: String::new(),
            hardware_acceleration: "none".into(),
            hardware_encoding: true,
            hardware_decoding_codecs: ["h264", "hevc", "mpeg2video", "mpeg4", "vc1", "vp8", "vp9"]
                .map(String::from)
                .to_vec(),
            decode_hevc_10bit: true,
            decode_vp9_10bit: true,
            enhanced_nvdec: true,
            encoding_threads: 0,
            encoder_preset: "veryfast".into(),
            h264_crf: 23,
            max_output_height: 720,
            video_bitrate_kbps: 2500,
            audio_bitrate_kbps: 128,
            audio_vbr: false,
            downmix_audio_boost: 2.0,
            max_muxing_queue_size: 2048,
            deinterlace_method: "yadif".into(),
            deinterlace_double_rate: false,
            tonemapping: false,
            tonemapping_algorithm: "hable".into(),
            tonemapping_desat: 0.0,
            tonemapping_peak: 100.0,
            tonemapping_param: None,
        }
    }
}
impl PlaybackSettings {
    pub fn validate(&self) -> bool {
        self.min_resume_percentage.is_finite()
            && self.max_resume_percentage.is_finite()
            && self.min_resume_percentage >= 0.0
            && self.min_resume_percentage < self.max_resume_percentage
            && self.max_resume_percentage <= 100.0
            && self.min_resume_duration_seconds <= 604800
            && self
                .streaming_bitrate_limit_mbps
                .is_none_or(|n| n.is_finite() && (0.0..=10000.0).contains(&n))
            && self.local_networks.len() <= 64
            && self.trusted_proxies.len() <= 64
            && self
                .local_networks
                .iter()
                .chain(&self.trusted_proxies)
                .all(|n| n.parse::<ipnet::IpNet>().is_ok())
            && self.ffmpeg_path.len() <= 4096
            && !self.ffmpeg_path.contains('\0')
            && self.transcoding_temp_path.len() <= 4096
            && !self.transcoding_temp_path.contains('\0')
            && matches!(self.hardware_acceleration.as_str(), "none" | "nvenc")
            && self.hardware_decoding_codecs.len() <= 8
            && self.hardware_decoding_codecs.iter().all(|s| {
                matches!(
                    s.as_str(),
                    "h264" | "hevc" | "mpeg2video" | "mpeg4" | "vc1" | "vp8" | "vp9" | "av1"
                )
            })
            && self.encoding_threads <= 256
            && self.h264_crf <= 51
            && matches!(
                self.encoder_preset.as_str(),
                "ultrafast"
                    | "superfast"
                    | "veryfast"
                    | "faster"
                    | "fast"
                    | "medium"
                    | "slow"
                    | "slower"
                    | "veryslow"
            )
            && (144..=4320).contains(&self.max_output_height)
            && self.max_output_height % 2 == 0
            && (100..=200000).contains(&self.video_bitrate_kbps)
            && (32..=512).contains(&self.audio_bitrate_kbps)
            && self.downmix_audio_boost.is_finite()
            && (0.5..=3.0).contains(&self.downmix_audio_boost)
            && (128..=65536).contains(&self.max_muxing_queue_size)
            && matches!(self.deinterlace_method.as_str(), "yadif" | "bwdif" | "none")
            && matches!(
                self.tonemapping_algorithm.as_str(),
                "hable" | "reinhard" | "mobius" | "clip" | "linear" | "gamma"
            )
            && self.tonemapping_desat.is_finite()
            && (0.0..=100.0).contains(&self.tonemapping_desat)
            && self.tonemapping_peak.is_finite()
            && (0.0..=10000.0).contains(&self.tonemapping_peak)
            && self
                .tonemapping_param
                .is_none_or(|n| n.is_finite() && (0.0..=100.0).contains(&n))
    }

    /// Video resume semantics: ignore the opening, complete short clips/past the end threshold.
    pub fn resume_state(&self, position: f64, duration: f64) -> (f64, bool) {
        if !position.is_finite() || !duration.is_finite() || duration <= 0.0 || position <= 0.0 {
            return (0.0, false);
        }
        let pct = position / duration * 100.0;
        if pct < self.min_resume_percentage {
            (0.0, false)
        } else if pct > self.max_resume_percentage
            || position >= duration - 1.0
            || duration < self.min_resume_duration_seconds as f64
        {
            (0.0, true)
        } else {
            (position.min(duration), false)
        }
    }

    pub fn remote_limit_kbps(&self, address: IpAddr) -> Option<u32> {
        if in_networks(address, &self.local_networks) {
            None
        } else {
            self.streaming_bitrate_limit_mbps
                .filter(|v| *v > 0.0)
                .map(|v| (v * 1000.0).floor() as u32)
        }
    }
}

pub fn in_networks(ip: IpAddr, networks: &[String]) -> bool {
    let ip = match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        ip => ip,
    };
    networks
        .iter()
        .filter_map(|n| n.parse::<ipnet::IpNet>().ok())
        .any(|n| n.contains(&ip))
}

/// Only honor a forwarding chain from a configured proxy. Walk from the nearest hop.
pub fn client_address(peer: IpAddr, forwarded: Option<&str>, trusted: &[String]) -> IpAddr {
    if !in_networks(peer, trusted) {
        return peer;
    }
    // Missing/malformed forwarding data must not turn a remote request into the
    // proxy's local address. The unspecified address receives the remote policy.
    let unknown = IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED);
    let Some(chain) = forwarded.filter(|s| s.len() <= 2048) else {
        return unknown;
    };
    let mut address = peer;
    for hop in chain.split(',').rev() {
        if !in_networks(address, trusted) {
            break;
        }
        let Ok(hop) = hop.trim().parse::<IpAddr>() else { return unknown; };
        address = hop;
    }
    address
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_boundaries() {
        let c = PlaybackSettings::default();
        assert_eq!(c.resume_state(49.0, 1000.0), (0.0, false));
        assert_eq!(c.resume_state(50.0, 1000.0), (50.0, false));
        assert_eq!(c.resume_state(900.0, 1000.0), (900.0, false));
        assert_eq!(c.resume_state(901.0, 1000.0), (0.0, true));
        assert_eq!(c.resume_state(10.0, 100.0), (0.0, true));
        assert_eq!(c.resume_state(0.0, 100.0), (0.0, false));
    }
    #[test]
    fn remote_limit_and_proxy_spoofing() {
        let c = PlaybackSettings {
            streaming_bitrate_limit_mbps: Some(0.25),
            ..Default::default()
        };
        assert_eq!(c.remote_limit_kbps("8.8.8.8".parse().unwrap()), Some(250));
        assert_eq!(
            c.remote_limit_kbps("::ffff:192.168.1.10".parse().unwrap()),
            None
        );
        let peer = "8.8.8.8".parse().unwrap();
        assert_eq!(client_address(peer, Some("127.0.0.1"), &[]), peer);
        assert_eq!(
            client_address(
                "127.0.0.1".parse().unwrap(),
                Some("192.168.1.1, 8.8.8.8"),
                &["127.0.0.1/32".into()]
            ),
            peer
        );
    }
    #[test]
    fn rejects_invalid_policy() {
        let mut c = PlaybackSettings::default();
        assert!(c.validate());
        c.min_resume_percentage = 95.0;
        assert!(!c.validate());
        c = PlaybackSettings::default();
        c.tonemapping_algorithm = "hable,evil".into();
        assert!(!c.validate());
    }
    #[test]
    fn malformed_forwarding_cannot_bypass_remote_limits() {
        let config = PlaybackSettings { streaming_bitrate_limit_mbps: Some(1.0), ..Default::default() };
        let proxy = "127.0.0.1".parse().unwrap();
        let trusted = ["127.0.0.1/32".into()];
        assert_eq!(client_address(proxy, Some("garbage, 8.8.8.8"), &trusted), "8.8.8.8".parse::<IpAddr>().unwrap());
        for header in [None, Some("garbage"), Some("")] {
            assert_eq!(config.remote_limit_kbps(client_address(proxy, header, &trusted)), Some(1000));
        }
    }
}
