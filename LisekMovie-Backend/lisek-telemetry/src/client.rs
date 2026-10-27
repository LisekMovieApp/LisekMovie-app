//! What kind of device a playback session comes from, read from the
//! `User-Agent` header (Faza 13). Only coarse classes are kept — platform,
//! device type, browser family — enough for "how much is watched on TVs vs
//! phones", not a fingerprint.
//!
//! Order matters: TV platforms are checked first (many TV browsers say
//! "Linux" or "Android"), then mobile systems (iPadOS and Android also
//! claim desktop tokens), then desktop systems.

use serde::Serialize;

/// Operating system family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    Macos,
    Linux,
    Android,
    Ios,
    SmartTv,
    Other,
    Unknown,
}

/// Form factor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeviceType {
    Desktop,
    Mobile,
    Tablet,
    Tv,
    Unknown,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Macos => "macos",
            Self::Linux => "linux",
            Self::Android => "android",
            Self::Ios => "ios",
            Self::SmartTv => "smart_tv",
            Self::Other => "other",
            Self::Unknown => "unknown",
        }
    }

    /// Every value, for validating filters.
    pub const ALL: [Platform; 8] = [
        Self::Windows,
        Self::Macos,
        Self::Linux,
        Self::Android,
        Self::Ios,
        Self::SmartTv,
        Self::Other,
        Self::Unknown,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.as_str() == value)
    }
}

impl DeviceType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Desktop => "desktop",
            Self::Mobile => "mobile",
            Self::Tablet => "tablet",
            Self::Tv => "tv",
            Self::Unknown => "unknown",
        }
    }

    pub const ALL: [DeviceType; 5] = [
        Self::Desktop,
        Self::Mobile,
        Self::Tablet,
        Self::Tv,
        Self::Unknown,
    ];

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.as_str() == value)
    }
}

/// The classes of one `User-Agent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInfo {
    /// The header, trimmed and cut to [`MAX_USER_AGENT`] characters.
    pub user_agent: Option<String>,
    pub platform: Platform,
    pub device_type: DeviceType,
    /// `chrome`, `firefox`, `safari`, `edge`, `opera`, `samsung`.
    pub browser: Option<&'static str>,
}

/// Longest `User-Agent` stored (the database checks the same limit).
pub const MAX_USER_AGENT: usize = 512;

const TV_MARKERS: [&str; 16] = [
    "smart-tv",
    "smarttv",
    "smart tv",
    "tizen",
    "web0s",
    "webos",
    "netcast",
    "hbbtv",
    "bravia",
    "googletv",
    "android tv",
    "appletv",
    "crkey",
    "roku",
    "aftb",
    "afts",
];

/// Classifies a `User-Agent` header value.
pub fn classify(user_agent: Option<&str>) -> ClientInfo {
    let Some(raw) = user_agent.map(str::trim).filter(|s| !s.is_empty()) else {
        return ClientInfo {
            user_agent: None,
            platform: Platform::Unknown,
            device_type: DeviceType::Unknown,
            browser: None,
        };
    };
    let stored: String = raw.chars().take(MAX_USER_AGENT).collect();
    let ua = raw.to_ascii_lowercase();
    let has = |needle: &str| ua.contains(needle);

    let fire_tv = (has("aft") && has("amazon")) || has("aftm") || has("aftt");
    let (platform, device_type) = if TV_MARKERS.iter().any(|m| has(m))
        || fire_tv
        || has("; tv")
        || has("playstation")
        || has("xbox")
    {
        (Platform::SmartTv, DeviceType::Tv)
    } else if has("iphone") || has("ipod") {
        (Platform::Ios, DeviceType::Mobile)
    } else if has("ipad") {
        (Platform::Ios, DeviceType::Tablet)
    } else if has("android") {
        let kind = if has("mobile") {
            DeviceType::Mobile
        } else {
            DeviceType::Tablet
        };
        (Platform::Android, kind)
    } else if has("windows phone") {
        (Platform::Other, DeviceType::Mobile)
    } else if has("windows") {
        (Platform::Windows, DeviceType::Desktop)
    } else if has("macintosh") || has("mac os x") {
        (Platform::Macos, DeviceType::Desktop)
    } else if has("cros") {
        (Platform::Other, DeviceType::Desktop)
    } else if has("linux") || has("x11") {
        (Platform::Linux, DeviceType::Desktop)
    } else {
        (Platform::Other, DeviceType::Unknown)
    };

    // Brand tokens come before the engines they also mention.
    let browser = if has("edg/") || has("edga/") || has("edgios/") {
        Some("edge")
    } else if has("opr/") || has("opera") {
        Some("opera")
    } else if has("samsungbrowser") {
        Some("samsung")
    } else if has("firefox/") || has("fxios/") {
        Some("firefox")
    } else if has("chrome/") || has("crios/") || has("chromium/") {
        Some("chrome")
    } else if has("safari/") && has("version/") {
        Some("safari")
    } else {
        None
    };

    ClientInfo {
        user_agent: Some(stored),
        platform,
        device_type,
        browser,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(ua: &str) -> (Platform, DeviceType, Option<&'static str>) {
        let info = classify(Some(ua));
        (info.platform, info.device_type, info.browser)
    }

    #[test]
    fn desktop_browsers() {
        assert_eq!(
            kind(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36"
            ),
            (Platform::Windows, DeviceType::Desktop, Some("chrome"))
        );
        assert_eq!(
            kind(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/130.0.0.0"
            ),
            (Platform::Windows, DeviceType::Desktop, Some("edge"))
        );
        assert_eq!(
            kind("Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0"),
            (Platform::Linux, DeviceType::Desktop, Some("firefox"))
        );
        assert_eq!(
            kind(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_6) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15"
            ),
            (Platform::Macos, DeviceType::Desktop, Some("safari"))
        );
    }

    #[test]
    fn phones_and_tablets() {
        assert_eq!(
            kind(
                "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1"
            ),
            (Platform::Ios, DeviceType::Mobile, Some("safari"))
        );
        assert_eq!(
            kind(
                "Mozilla/5.0 (iPad; CPU OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/130.0 Mobile/15E148 Safari/604.1"
            ),
            (Platform::Ios, DeviceType::Tablet, Some("chrome"))
        );
        assert_eq!(
            kind(
                "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Mobile Safari/537.36"
            ),
            (Platform::Android, DeviceType::Mobile, Some("chrome"))
        );
        assert_eq!(
            kind(
                "Mozilla/5.0 (Linux; Android 13; SM-X710) AppleWebKit/537.36 (KHTML, like Gecko) SamsungBrowser/25.0 Chrome/121.0.0.0 Safari/537.36"
            ),
            (Platform::Android, DeviceType::Tablet, Some("samsung"))
        );
    }

    #[test]
    fn televisions() {
        for ua in [
            "Mozilla/5.0 (SMART-TV; LINUX; Tizen 7.0) AppleWebKit/537.36 (KHTML, like Gecko) 94.0.4606.31/7.0 TV Safari/537.36",
            "Mozilla/5.0 (Web0S; Linux/SmartTV) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/87.0.4280.88 Safari/537.36 WebAppManager",
            "Mozilla/5.0 (Linux; Android 12; BRAVIA 4K VH2 Build/STT1.211025.001.Z4) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/117.0 Safari/537.36",
            "Mozilla/5.0 (Linux; Android 9; AFTMM Build/PS7633) AppleWebKit/537.36 (KHTML, like Gecko) Silk/120 like Chrome/120 Safari/537.36",
            "Mozilla/5.0 (X11; Linux armv7l) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/114.0 Safari/537.36 CrKey/1.56.500000",
            "Mozilla/5.0 (Linux; Android 11; Chromecast; TV) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120 Safari/537.36",
        ] {
            let info = classify(Some(ua));
            assert_eq!(
                (info.platform, info.device_type),
                (Platform::SmartTv, DeviceType::Tv),
                "{ua}"
            );
        }
    }

    #[test]
    fn missing_odd_and_long_values() {
        let none = classify(None);
        assert_eq!(
            (none.platform, none.device_type, none.user_agent),
            (Platform::Unknown, DeviceType::Unknown, None)
        );
        assert_eq!(classify(Some("   ")).platform, Platform::Unknown);
        let curl = classify(Some("curl/8.9.1"));
        assert_eq!((curl.platform, curl.browser), (Platform::Other, None));
        let long = "x".repeat(2000);
        assert_eq!(
            classify(Some(&long)).user_agent.unwrap().len(),
            MAX_USER_AGENT
        );
        assert_eq!(Platform::parse("smart_tv"), Some(Platform::SmartTv));
        assert_eq!(Platform::parse("tv"), None);
        assert_eq!(DeviceType::parse("tablet"), Some(DeviceType::Tablet));
    }
}
