//! Viewing time as people read it in reports and bot messages
//! (Faza 13): "1h 38m", "8m 48s", "45s".

/// Formats whole seconds: hours and minutes from one hour up, minutes and
/// seconds below it, seconds alone under a minute. Negative or non-finite
/// values count as zero.
pub fn human(seconds: f64) -> String {
    let total = if seconds.is_finite() && seconds > 0.0 {
        seconds.round() as u64
    } else {
        0
    };
    let (h, m, s) = (total / 3600, total % 3600 / 60, total % 60);
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m {s}s")
    } else {
        format!("{s}s")
    }
}

#[cfg(test)]
mod tests {
    use super::human;

    #[test]
    fn matches_the_report_style() {
        assert_eq!(human(5880.0), "1h 38m");
        assert_eq!(human(528.0), "8m 48s");
        assert_eq!(human(3270.4), "54m 30s");
        assert_eq!(human(11640.0), "3h 14m");
        assert_eq!(human(45.0), "45s");
        assert_eq!(human(0.0), "0s");
        assert_eq!(human(-3.0), "0s");
        assert_eq!(human(f64::NAN), "0s");
        assert_eq!(human(3600.0), "1h 0m");
    }
}
