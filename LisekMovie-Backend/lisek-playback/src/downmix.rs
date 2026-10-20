//! Surround to stereo downmix. Each algorithm is an FFmpeg `-af` chain
//! built on swresample's rematrixing, which knows every channel layout
//! (5.1, 5.1(side), 6.1, 7.1 ...) by name, so no per-layout matrix is
//! hard-coded here.
//!
//! All chains normalise the matrix (`rematrix_maxval=1`, no clipping
//! inside the mix), apply the configured boost to win back the loudness
//! normalisation costs, and end in a limiter so the boost cannot clip.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownmixAlgorithm {
    /// ITU-R BS.775: centre and surrounds at -3 dB, LFE dropped.
    Standard,
    /// Centre +3 dB, surrounds -6 dB: speech stays clear over music and
    /// effects. Also suits Polish voice-over (lektor) tracks, whose
    /// narrator sits in the centre channel.
    Dialogue,
    /// Dialogue plus dynamic range compression, for quiet listening.
    Night,
    /// Dolby Pro Logic II matrix encoding, for receivers that rebuild
    /// surround from stereo.
    ProLogicIi,
}

/// What the client asked for. `Auto` lets the server choose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DownmixPreference {
    #[default]
    Auto,
    Standard,
    Dialogue,
    Night,
    ProLogicIi,
}

impl DownmixAlgorithm {
    /// Parses the `downmix_algorithm` setting.
    pub fn from_setting(value: &str) -> Self {
        match value {
            "dialogue" => Self::Dialogue,
            "night" => Self::Night,
            "pro_logic_ii" => Self::ProLogicIi,
            _ => Self::Standard,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Dialogue => "dialogue",
            Self::Night => "night",
            Self::ProLogicIi => "pro_logic_ii",
        }
    }

    /// The `-af` chain producing stereo. `boost` is the configured
    /// `downmix_audio_boost` (linear gain).
    pub fn filter(self, boost: f64) -> String {
        let boost = if boost.is_finite() { boost.clamp(0.5, 3.0) } else { 1.0 };
        let limiter = "alimiter=limit=0.97:attack=5:release=50:level=0";
        let rematrix = |levels: &str| {
            format!("aresample=ochl=stereo:{levels}:lfe_mix_level=0:rematrix_maxval=1")
        };
        match self {
            Self::Standard => format!(
                "{},volume={boost},{limiter}",
                rematrix("clev=0.707:slev=0.707")
            ),
            Self::Dialogue => format!(
                "{},volume={boost},{limiter}",
                rematrix("clev=1.414:slev=0.5")
            ),
            // Compress above -20 dBFS at 4:1 before the boost: loud
            // effects come down, speech keeps its level.
            Self::Night => format!(
                "{},acompressor=threshold=0.1:ratio=4:attack=20:release=250:makeup=2,volume={boost},{limiter}",
                rematrix("clev=1.414:slev=0.5")
            ),
            Self::ProLogicIi => format!(
                "{},volume={boost},{limiter}",
                rematrix("matrix_encoding=dplii:clev=0.707:slev=0.707")
            ),
        }
    }
}

/// The downmix to apply when `source_channels` must fit into
/// `target_channels`, or `None` when no downmix to stereo is needed.
/// Reductions that keep surround (7.1 to 5.1) use FFmpeg's default
/// matrix and need no algorithm.
///
/// `Auto` takes the server default, except for tracks titled as a Polish
/// voice-over ("Lektor"), which get [`DownmixAlgorithm::Dialogue`] so the
/// narrator is not buried under the original soundtrack.
pub fn choose(
    preference: DownmixPreference,
    server_default: DownmixAlgorithm,
    source_channels: u32,
    target_channels: u32,
    track_title: Option<&str>,
) -> Option<DownmixAlgorithm> {
    if source_channels <= target_channels || target_channels > 2 {
        return None;
    }
    Some(match preference {
        DownmixPreference::Standard => DownmixAlgorithm::Standard,
        DownmixPreference::Dialogue => DownmixAlgorithm::Dialogue,
        DownmixPreference::Night => DownmixAlgorithm::Night,
        DownmixPreference::ProLogicIi => DownmixAlgorithm::ProLogicIi,
        DownmixPreference::Auto => {
            let lektor = track_title.is_some_and(|t| t.to_lowercase().contains("lektor"));
            if lektor && server_default == DownmixAlgorithm::Standard {
                DownmixAlgorithm::Dialogue
            } else {
                server_default
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_surround_to_stereo_needs_an_algorithm() {
        let d = DownmixAlgorithm::Standard;
        assert_eq!(choose(DownmixPreference::Auto, d, 2, 2, None), None);
        assert_eq!(choose(DownmixPreference::Auto, d, 8, 6, None), None);
        assert_eq!(choose(DownmixPreference::Auto, d, 6, 2, None), Some(d));
        assert_eq!(
            choose(DownmixPreference::Night, d, 6, 2, None),
            Some(DownmixAlgorithm::Night)
        );
    }

    #[test]
    fn auto_favours_dialogue_for_voice_over_tracks() {
        let pick = |default, title| choose(DownmixPreference::Auto, default, 6, 2, title);
        assert_eq!(pick(DownmixAlgorithm::Standard, Some("Lektor PL")), Some(DownmixAlgorithm::Dialogue));
        assert_eq!(pick(DownmixAlgorithm::Standard, Some("English")), Some(DownmixAlgorithm::Standard));
        // An explicit server default is respected.
        assert_eq!(pick(DownmixAlgorithm::Night, Some("Lektor")), Some(DownmixAlgorithm::Night));
    }

    #[test]
    fn filters_normalise_boost_and_limit() {
        let f = DownmixAlgorithm::Dialogue.filter(2.0);
        assert!(f.starts_with("aresample=ochl=stereo:clev=1.414:slev=0.5:lfe_mix_level=0:rematrix_maxval=1"));
        assert!(f.contains("volume=2") && f.ends_with("level=0"));
        assert!(DownmixAlgorithm::Night.filter(9.0).contains("acompressor=") );
        assert!(DownmixAlgorithm::Night.filter(9.0).contains("volume=3,"), "boost is clamped");
        assert!(DownmixAlgorithm::ProLogicIi.filter(1.0).contains("matrix_encoding=dplii"));
        assert_eq!(DownmixAlgorithm::from_setting("night"), DownmixAlgorithm::Night);
        assert_eq!(DownmixAlgorithm::from_setting("bogus"), DownmixAlgorithm::Standard);
    }
}
