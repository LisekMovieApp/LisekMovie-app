//! FFmpeg and FFprobe stderr in the server log (Faza 17): every line as a
//! `tracing` event with target `ffmpeg` / `ffprobe` and a `[FFMPEG]` /
//! `[FFPROBE]` prefix, so the admin log viewer can show or hide them as
//! their own source. The tools run with `-loglevel warning` / `-v error`,
//! so these are warnings and errors, not progress chatter.

/// Which tool wrote the lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Ffmpeg,
    Ffprobe,
}

impl Tool {
    pub fn of(program: &str) -> Self {
        if program.contains("ffprobe") { Tool::Ffprobe } else { Tool::Ffmpeg }
    }
}

/// More lines than this from one run are summarized, not logged one by one.
pub const MAX_LINES: usize = 200;

/// How serious a line reads: errors as ERROR, the rest as WARN.
fn is_error(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    ["error", "failed", "invalid", "cannot", "could not", "no such file", "unable", "not found", "fatal"]
        .iter()
        .any(|word| lower.contains(word))
}

/// One stderr line. `context` says what the run was for (a session id, a
/// file path).
pub fn line(tool: Tool, context: &str, line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    match (tool, is_error(line)) {
        (Tool::Ffmpeg, true) => tracing::error!(target: "ffmpeg", context, "[FFMPEG] {line}"),
        (Tool::Ffmpeg, false) => tracing::warn!(target: "ffmpeg", context, "[FFMPEG] {line}"),
        (Tool::Ffprobe, true) => tracing::error!(target: "ffprobe", context, "[FFPROBE] {line}"),
        (Tool::Ffprobe, false) => tracing::warn!(target: "ffprobe", context, "[FFPROBE] {line}"),
    }
}

/// Every line of a finished run's stderr (at most [`MAX_LINES`]).
pub fn stderr(tool: Tool, context: &str, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    for l in lines.iter().take(MAX_LINES) {
        line(tool, context, l);
    }
    if lines.len() > MAX_LINES {
        let skipped = lines.len() - MAX_LINES;
        match tool {
            Tool::Ffmpeg => tracing::warn!(target: "ffmpeg", context, skipped, "[FFMPEG] … {skipped} more lines"),
            Tool::Ffprobe => tracing::warn!(target: "ffprobe", context, skipped, "[FFPROBE] … {skipped} more lines"),
        }
    }
}

/// Diagnostics (hardware tests) expect failures: their stderr is DEBUG.
pub fn debug(tool: Tool, context: &str, bytes: &[u8]) {
    let text = String::from_utf8_lossy(bytes);
    for l in text.lines().map(str::trim).filter(|l| !l.is_empty()).take(20) {
        match tool {
            Tool::Ffmpeg => tracing::debug!(target: "ffmpeg", context, "[FFMPEG] {l}"),
            Tool::Ffprobe => tracing::debug!(target: "ffprobe", context, "[FFPROBE] {l}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_told_from_warnings() {
        assert!(is_error("Error opening input file x.mkv."));
        assert!(is_error("[h264_qsv @ 0x1] Failed to create a VAAPI device"));
        assert!(!is_error("[mp4 @ 0x2] Non-monotonic DTS; previous: 1, current: 0"));
        assert_eq!(Tool::of("/usr/bin/ffprobe"), Tool::Ffprobe);
        assert_eq!(Tool::of("ffmpeg"), Tool::Ffmpeg);
        // Logging needs no subscriber.
        stderr(Tool::Ffmpeg, "test", b"a\n\nb\n");
    }
}
