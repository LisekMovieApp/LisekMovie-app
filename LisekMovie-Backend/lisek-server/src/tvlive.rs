//! Faza 26: running channels — one shared HLS stream per channel.
//!
//! A channel is streamed once, however many people watch it: the first
//! viewer starts it, the last one leaving (no request for `IDLE_TIMEOUT`)
//! stops it, and `keep_running` channels stay up. Two kinds:
//!
//! - **Own channels** follow the planned timeline (`lisek_db::tv`). Every
//!   timeline entry is encoded by its own short FFmpeg process into one
//!   uniform MPEG-TS (H.264 at the channel's quality, 25 fps, AAC 48 kHz
//!   stereo; a silent track when the file has no audio), with
//!   `-output_ts_offset` keeping the timestamps running, and the pieces are
//!   fed through a pipe into one FFmpeg that cuts the HLS segments. Files
//!   of any size, codec or frame rate therefore play back to back without a
//!   break in the playlist. A gap in the timeline plays the channel's
//!   fallback ad/bumper, or a plain slate, so the stream never ends.
//! - **IPTV channels** (Faza 20) are one FFmpeg reading the provider's
//!   stream, shared in the same way.
//!
//! Viewers in the browser get a ticket in the path (as in Faza 20); IPTV
//! players use an API key (`/iptv/...`).

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Notify, watch};
use uuid::Uuid;

use lisek_db::tv as db;

use crate::state::SharedState;

pub const MAX_TV_STREAMS: usize = 8;
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const START_TIMEOUT: Duration = Duration::from_secs(30);
const TS_PACKET: usize = 188;
/// The slate (nothing to play, no fallback) is cut into pieces this long.
const SLATE_SECONDS: f64 = 10.0;
/// An item shorter than this is skipped, not encoded.
const MIN_PLAY_SECONDS: f64 = 0.4;

#[derive(Debug, PartialEq, Eq)]
pub enum StreamError {
    Capacity,
    Unavailable,
}

/// What a stream plays.
#[derive(Clone)]
pub enum Source {
    Own(Uuid),
    Iptv { url: String, transcode: bool },
}

pub struct ChannelStream {
    pub id: Uuid,
    pub name: String,
    pub dir: PathBuf,
    source: Source,
    stop: watch::Sender<bool>,
    reload: Notify,
    last_access: Mutex<Instant>,
    keep_running: AtomicBool,
    finished: AtomicBool,
    log: Mutex<VecDeque<String>>,
}

impl ChannelStream {
    fn touch(&self) {
        *self.last_access.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last_access.lock().unwrap_or_else(|e| e.into_inner()).elapsed()
    }

    pub fn is_running(&self) -> bool {
        !self.finished.load(Ordering::Relaxed) && !*self.stop.borrow()
    }

    pub fn set_keep_running(&self, keep: bool) {
        self.keep_running.store(keep, Ordering::Relaxed);
    }

    pub fn log_tail(&self) -> String {
        self.log.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect::<Vec<_>>().join(" | ")
    }

    fn push_log(&self, line: String) {
        let mut log = self.log.lock().unwrap_or_else(|e| e.into_inner());
        if log.len() >= 20 {
            log.pop_front();
        }
        log.push_back(line);
    }

    fn stop(&self) {
        let _ = self.stop.send(true);
    }
}

struct Viewer {
    user_id: Uuid,
    channel_id: Uuid,
    ticket: String,
    last_access: Instant,
}

/// Running channels and the browser viewers watching them.
#[derive(Default)]
pub struct TvSessions {
    streams: Mutex<HashMap<Uuid, Arc<ChannelStream>>>,
    viewers: Mutex<HashMap<Uuid, Viewer>>,
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl TvSessions {
    fn streams(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Arc<ChannelStream>>> {
        self.streams.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn viewers(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Viewer>> {
        self.viewers.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get(&self, id: Uuid) -> Option<Arc<ChannelStream>> {
        self.streams().get(&id).cloned()
    }

    /// Ids of the channels streaming now, with how many browser viewers each has.
    pub fn running(&self) -> Vec<(Uuid, usize)> {
        let viewers = self.viewers();
        self.streams()
            .keys()
            .map(|id| (*id, viewers.values().filter(|v| v.channel_id == *id).count()))
            .collect()
    }

    /// Stops a channel's stream (it was disabled or deleted).
    pub fn stop_channel(&self, id: Uuid) {
        if let Some(stream) = self.streams().remove(&id) {
            stream.stop();
            self.viewers().retain(|_, v| v.channel_id != id);
            remove_dir_later(stream.dir.clone());
        }
    }

    /// Makes a stream follow a changed timeline or settings: the piece
    /// being encoded is dropped and the next one is chosen afresh; the
    /// segments in between stay continuous.
    pub fn restart_channel(&self, id: Uuid) {
        if let Some(stream) = self.get(id) {
            stream.reload.notify_one();
        }
    }

    /// A viewer's request: keeps the stream alive; the viewer's directory.
    pub fn touch_viewer(&self, session_id: Uuid, ticket: &str) -> Option<PathBuf> {
        let channel_id = {
            let mut viewers = self.viewers();
            let viewer = viewers.get_mut(&session_id)?;
            if !constant_time_eq(viewer.ticket.as_bytes(), ticket.as_bytes()) {
                return None;
            }
            viewer.last_access = Instant::now();
            viewer.channel_id
        };
        self.touch_channel(channel_id)
    }

    /// A request for a channel's files (an IPTV client): keeps it alive.
    pub fn touch_channel(&self, id: Uuid) -> Option<PathBuf> {
        let stream = self.get(id)?;
        stream.touch();
        Some(stream.dir.clone())
    }

    pub fn add_viewer(&self, user_id: Uuid, channel_id: Uuid) -> (Uuid, String) {
        let session_id = Uuid::new_v4();
        let ticket = Uuid::new_v4().simple().to_string();
        // One channel per account: switching replaces the earlier viewer.
        let mut viewers = self.viewers();
        viewers.retain(|_, v| v.user_id != user_id);
        viewers.insert(session_id, Viewer { user_id, channel_id, ticket: ticket.clone(), last_access: Instant::now() });
        (session_id, ticket)
    }

    pub fn stop_viewer(&self, session_id: Uuid, user_id: Uuid) -> bool {
        let mut viewers = self.viewers();
        if viewers.get(&session_id).is_none_or(|v| v.user_id != user_id) {
            return false;
        }
        viewers.remove(&session_id);
        true
    }

    /// Drops viewers and streams nobody uses. A viewer who left no
    /// longer keeps the stream alive: it simply stops being asked for.
    pub fn reap(&self, idle: Duration) {
        self.viewers().retain(|_, v| v.last_access.elapsed() <= idle);
        let stopped: Vec<Arc<ChannelStream>> = {
            let mut streams = self.streams();
            let ids: Vec<Uuid> = streams
                .values()
                .filter(|s| s.finished.load(Ordering::Relaxed) || (!s.keep_running.load(Ordering::Relaxed) && s.idle_for() > idle))
                .map(|s| s.id)
                .collect();
            ids.into_iter().filter_map(|id| streams.remove(&id)).collect()
        };
        for stream in stopped {
            tracing::debug!(channel = %stream.name, "TV stream stopped (idle or ended)");
            stream.stop();
            remove_dir_later(stream.dir.clone());
        }
    }
}

fn remove_dir_later(dir: PathBuf) {
    tokio::spawn(async move {
        // The processes get a moment to see the stop signal.
        tokio::time::sleep(Duration::from_secs(2)).await;
        let _ = tokio::fs::remove_dir_all(&dir).await;
    });
}

/// Reaper and `keep_running` supervisor; spawned once.
pub async fn run_reaper(state: SharedState) {
    loop {
        tokio::time::sleep(Duration::from_secs(15)).await;
        state.tv.reap(IDLE_TIMEOUT);
        let Some(pool) = state.pool() else { continue };
        let Ok(channels) = db::list_channels(&pool).await else { continue };
        for channel in channels {
            if let Some(stream) = state.tv.get(channel.id) {
                stream.set_keep_running(channel.keep_running && channel.enabled);
            } else if channel.enabled && channel.keep_running {
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = ensure_own(&state, &channel).await {
                        tracing::warn!(channel = %channel.name, error = ?e, "keeping a TV channel running failed");
                    }
                });
            }
        }
    }
}

// ---- Starting ------------------------------------------------------------------

fn stream_dir(id: Uuid) -> PathBuf {
    std::env::temp_dir().join("lisekmovie-tv").join(format!("{id}-{}", Uuid::new_v4().simple()))
}

/// The stream of an own channel, started when it is not running.
pub async fn ensure_own(state: &SharedState, channel: &db::TvChannel) -> Result<Arc<ChannelStream>, StreamError> {
    ensure(state, channel.id, &channel.name, Source::Own(channel.id), channel.keep_running).await
}

/// The shared stream of an IPTV channel (Faza 20).
pub async fn ensure_iptv(state: &SharedState, id: Uuid, name: &str, url: &str, transcode: bool) -> Result<Arc<ChannelStream>, StreamError> {
    ensure(state, id, name, Source::Iptv { url: url.to_string(), transcode }, false).await
}

async fn ensure(state: &SharedState, id: Uuid, name: &str, source: Source, keep_running: bool) -> Result<Arc<ChannelStream>, StreamError> {
    let (stream, created) = {
        let mut streams = state.tv.streams();
        // A running IPTV stream that copies the video is replaced when a viewer
        // needs it re-encoded (the browser could not decode it).
        let needs_transcode = matches!(source, Source::Iptv { transcode: true, .. });
        let reusable = |s: &Arc<ChannelStream>| s.is_running() && !(needs_transcode && matches!(s.source, Source::Iptv { transcode: false, .. }));
        if let Some(stream) = streams.get(&id).filter(|s| reusable(s)) {
            (stream.clone(), false)
        } else {
            if streams.len() >= MAX_TV_STREAMS {
                return Err(StreamError::Capacity);
            }
            let dir = stream_dir(id);
            let (stop, _) = watch::channel(false);
            let stream = Arc::new(ChannelStream {
                id,
                name: name.to_string(),
                dir,
                source,
                stop,
                reload: Notify::new(),
                last_access: Mutex::new(Instant::now()),
                keep_running: AtomicBool::new(keep_running),
                finished: AtomicBool::new(false),
                log: Mutex::new(VecDeque::new()),
            });
            if let Some(old) = streams.insert(id, stream.clone()) {
                old.stop();
                remove_dir_later(old.dir.clone());
            }
            (stream, true)
        }
    };
    if created {
        tokio::fs::create_dir_all(&stream.dir).await.map_err(|_| StreamError::Unavailable)?;
        tokio::spawn(run(state.clone(), stream.clone()));
    }
    // Ready once the playlist names a segment.
    let started = Instant::now();
    loop {
        if let Ok(text) = tokio::fs::read_to_string(stream.dir.join("index.m3u8")).await
            && text.contains(".ts")
        {
            stream.touch();
            return Ok(stream);
        }
        if stream.finished.load(Ordering::Relaxed) || started.elapsed() > START_TIMEOUT {
            tracing::warn!(channel = %stream.name, ffmpeg = %stream.log_tail(), "TV stream did not start");
            state.tv.stop_channel(id);
            return Err(StreamError::Unavailable);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

// ---- FFmpeg arguments --------------------------------------------------------------

/// Output size and bitrate for a channel's `quality`.
pub fn quality_profile(quality: &str) -> (u32, u32, u32) {
    match quality {
        "480p" => (854, 480, 1500),
        "1080p" => (1920, 1080, 5500),
        _ => (1280, 720, 3000),
    }
}

/// What one encoder process plays.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The file; none: the slate.
    pub path: Option<String>,
    /// Where in the file to start (seconds).
    pub seek: f64,
    pub duration: f64,
    pub has_audio: bool,
    /// ffprobe index of the video stream.
    pub video_index: Option<i32>,
    /// A one-line description for the logs.
    pub label: String,
}

const SILENCE: &str = "anullsrc=r=48000:cl=stereo";

/// FFmpeg arguments encoding one plan into the channel's uniform MPEG-TS on stdout.
pub fn encoder_args(plan: &Plan, quality: &str, offset: f64) -> Vec<String> {
    let (w, h, kbps) = quality_profile(quality);
    let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error"].map(String::from).to_vec();
    match &plan.path {
        Some(path) => {
            args.extend(["-re".into(), "-ss".into(), format!("{:.3}", plan.seek.max(0.0)), "-i".into(), path.clone()]);
            if !plan.has_audio {
                args.extend(["-re", "-f", "lavfi", "-i", SILENCE].map(String::from));
            }
            let video = plan.video_index.map_or_else(|| "0:v:0".to_string(), |i| format!("0:{i}"));
            args.extend(["-map".into(), video, "-map".into(), if plan.has_audio { "0:a:0".into() } else { "1:a:0".to_string() }]);
        }
        None => {
            args.extend(["-re".into(), "-f".into(), "lavfi".into(), "-i".into(), format!("color=c=0x14110a:s={w}x{h}:r=25")]);
            args.extend(["-re", "-f", "lavfi", "-i", SILENCE].map(String::from));
            args.extend(["-map", "0:v:0", "-map", "1:a:0"].map(String::from));
        }
    }
    args.extend(["-sn".into(), "-dn".into(), "-t".into(), format!("{:.3}", plan.duration)]);
    args.extend([
        "-vf".into(),
        format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=25,format=yuv420p"),
        "-af".into(),
        "aresample=48000:async=1,aformat=channel_layouts=stereo".into(),
    ]);
    args.extend(
        [
            "-c:v", "libx264", "-preset", "veryfast", "-profile:v", "high", "-crf", "23", "-g", "50", "-keyint_min", "50", "-sc_threshold", "0",
        ]
        .map(String::from),
    );
    args.extend(["-maxrate".into(), format!("{kbps}k"), "-bufsize".into(), format!("{}k", kbps * 2)]);
    args.extend(["-c:a", "aac", "-b:a", "128k", "-ar", "48000", "-ac", "2"].map(String::from));
    args.extend(["-output_ts_offset".into(), format!("{offset:.3}"), "-muxdelay".into(), "0".into(), "-f".into(), "mpegts".into(), "-".into()]);
    args
}

/// FFmpeg arguments cutting the pieces on stdin into the live HLS window.
pub fn segmenter_args(dir: &Path) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "warning", "-f", "mpegts", "-i", "pipe:0", "-c", "copy"].map(String::from).to_vec();
    args.extend(
        [
            "-f", "hls", "-hls_time", "4", "-hls_list_size", "10", "-hls_flags", "delete_segments+independent_segments+omit_endlist",
        ]
        .map(String::from),
    );
    args.push("-hls_segment_filename".into());
    args.push(dir.join("seg%06d.ts").to_string_lossy().into_owned());
    args.push(dir.join("index.m3u8").to_string_lossy().into_owned());
    args
}

// ---- The runners ---------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Outcome {
    /// The piece played to its end.
    Finished,
    /// The timeline or settings changed.
    Reloaded,
    Stopped,
    /// The encoder failed (the tail of its messages).
    Failed(String),
    SegmenterDied,
}

fn command(ffmpeg: &Path, args: &[String]) -> Command {
    let mut command = Command::new(ffmpeg);
    command.args(args).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    command
}

fn log_stderr(child: &mut Child, stream: Arc<ChannelStream>, who: &'static str) {
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "lisek_playback::ffmpeg", channel = %stream.name, "[FFMPEG {who}] {line}");
                stream.push_log(format!("{who}: {line}"));
            }
        });
    }
}

async fn run(state: SharedState, stream: Arc<ChannelStream>) {
    let result = match stream.source.clone() {
        Source::Own(id) => run_own(&state, &stream, id).await,
        Source::Iptv { url, transcode } => run_iptv(&state, &stream, &url, transcode).await,
    };
    match result {
        Ok(()) => tracing::info!(channel = %stream.name, "TV stream ended"),
        Err(e) => tracing::warn!(channel = %stream.name, error = %e, "TV stream failed"),
    }
    stream.finished.store(true, Ordering::Relaxed);
}

async fn run_iptv(state: &SharedState, stream: &Arc<ChannelStream>, url: &str, transcode: bool) -> Result<(), String> {
    let pool = state.pool().ok_or("server not ready")?;
    let settings = lisek_db::playback::settings(&pool).await.map_err(|e| e.to_string())?;
    let ffmpeg = lisek_playback::transcode::binary_path("ffmpeg", &settings);
    let args = crate::livetv::ffmpeg_args(url, &stream.dir, transcode);
    let mut child = command(&ffmpeg, &args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(|e| e.to_string())?;
    log_stderr(&mut child, stream.clone(), "iptv");
    let mut stop = stream.stop.subscribe();
    tokio::select! {
        status = child.wait() => {
            status.map_err(|e| e.to_string())?;
            Ok(())
        }
        _ = stop.changed() => Ok(()),
    }
}

async fn run_own(state: &SharedState, stream: &Arc<ChannelStream>, id: Uuid) -> Result<(), String> {
    let pool = state.pool().ok_or("server not ready")?;
    let settings = lisek_db::playback::settings(&pool).await.map_err(|e| e.to_string())?;
    let ffmpeg = lisek_playback::transcode::binary_path("ffmpeg", &settings);
    let mut segmenter = command(&ffmpeg, &segmenter_args(&stream.dir))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start FFmpeg: {e}"))?;
    log_stderr(&mut segmenter, stream.clone(), "hls");
    let mut sink = segmenter.stdin.take().ok_or("no pipe to FFmpeg")?;
    let mut stop = stream.stop.subscribe();
    let started = Utc::now();
    let mut last_end = 0.0_f64;
    let mut failures = 0u32;
    loop {
        if *stop.borrow() {
            return Ok(());
        }
        let Some(channel) = db::get_channel(&pool, id).await.map_err(|e| e.to_string())? else { return Ok(()) };
        if !channel.enabled {
            return Ok(());
        }
        let now = Utc::now();
        let plan = next_plan(&pool, &settings, id, now).await.map_err(|e| e.to_string())?;
        let offset = ((now - started).num_milliseconds() as f64 / 1000.0).max(last_end);
        tracing::debug!(channel = %channel.name, what = %plan.label, seconds = plan.duration, "TV piece starts");
        let mut encoder = command(&ffmpeg, &encoder_args(&plan, &channel.quality, offset))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start FFmpeg: {e}"))?;
        log_stderr(&mut encoder, stream.clone(), "enc");
        let outcome = pump(&mut encoder, &mut sink, &mut segmenter, &mut stop, &stream.reload).await;
        let _ = encoder.start_kill();
        let _ = encoder.wait().await;
        let played = (Utc::now() - now).num_milliseconds() as f64 / 1000.0;
        match outcome {
            Outcome::Finished => {
                failures = 0;
                last_end = offset + plan.duration;
            }
            Outcome::Reloaded => last_end = offset + played.min(plan.duration),
            Outcome::Stopped => return Ok(()),
            Outcome::SegmenterDied => return Err(format!("the segmenter stopped: {}", stream.log_tail())),
            Outcome::Failed(message) => {
                failures += 1;
                tracing::warn!(channel = %channel.name, what = %plan.label, %message, "TV piece failed");
                last_end = offset + played.min(plan.duration);
                // A piece that cannot be played must not spin the loop.
                if played < 1.0 {
                    tokio::time::sleep(Duration::from_millis(500 * u64::from(failures.min(6)))).await;
                }
            }
        }
    }
}

/// Copies the encoder's MPEG-TS into the segmenter, whole packets only, so a
/// piece cut short never leaves half a packet in the stream.
async fn pump(encoder: &mut Child, sink: &mut ChildStdin, segmenter: &mut Child, stop: &mut watch::Receiver<bool>, reload: &Notify) -> Outcome {
    let Some(mut out) = encoder.stdout.take() else { return Outcome::Failed("no output".into()) };
    let mut buffer = vec![0u8; TS_PACKET * 256];
    let mut pending: Vec<u8> = Vec::with_capacity(TS_PACKET * 512);
    loop {
        tokio::select! {
            read = out.read(&mut buffer) => {
                match read {
                    Ok(0) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buffer[..n]);
                        let whole = pending.len() / TS_PACKET * TS_PACKET;
                        if whole > 0 {
                            if sink.write_all(&pending[..whole]).await.is_err() {
                                return Outcome::SegmenterDied;
                            }
                            pending.drain(..whole);
                        }
                    }
                    Err(e) => return Outcome::Failed(e.to_string()),
                }
            }
            () = reload.notified() => return Outcome::Reloaded,
            _ = stop.changed() => return Outcome::Stopped,
            _ = segmenter.wait() => return Outcome::SegmenterDied,
        }
    }
    match encoder.wait().await {
        Ok(status) if status.success() => Outcome::Finished,
        Ok(status) => Outcome::Failed(format!("FFmpeg exited with {status}")),
        Err(e) => Outcome::Failed(e.to_string()),
    }
}

/// What to play at `now`: the timeline entry (from where it is by now), or
/// the fallback when there is nothing.
async fn next_plan(pool: &lisek_db::PgPool, settings: &lisek_core::playback_settings::PlaybackSettings, channel: Uuid, now: DateTime<Utc>) -> Result<Plan, lisek_db::DbError> {
    let entry = db::item_at(pool, channel, now).await?;
    if let Some(entry) = &entry
        && let Some(path) = &entry.path
    {
        let into = (now - entry.starts_at).num_milliseconds() as f64 / 1000.0;
        let seek = f64::from(entry.in_ms) / 1000.0 + into;
        let left = (entry.ends_at - now).num_milliseconds() as f64 / 1000.0;
        let content_left = f64::from(entry.out_ms) / 1000.0 - seek;
        // A little shorter than planned is fine (the next piece takes over),
        // longer never.
        let duration = left.min(if content_left > 0.0 { content_left } else { left });
        if duration >= MIN_PLAY_SECONDS {
            return Ok(match lisek_playback::transcode::probe(Path::new(path), settings).await {
                Ok(info) => Plan {
                    path: Some(path.clone()),
                    seek,
                    duration,
                    has_audio: info.audio_channels > 0,
                    video_index: info.video_index,
                    label: entry.title.clone(),
                },
                // Unreadable: a slate for the length of the item keeps the stream going.
                Err(e) => slate(duration.min(SLATE_SECONDS), format!("unreadable {path}: {e}")),
            });
        }
        // The last fraction of a second: wait for the next entry.
        return Ok(slate(duration.max(MIN_PLAY_SECONDS), "tail of an item".into()));
    }
    // A gap (or an item whose file is gone) until the next entry starts.
    let until = match &entry {
        Some(entry) => entry.ends_at,
        None => db::item_after(pool, channel, now).await?.map_or(now + chrono::Duration::seconds(60), |next| next.starts_at),
    };
    let gap = ((until - now).num_milliseconds() as f64 / 1000.0).max(MIN_PLAY_SECONDS);
    if let Some((path, duration_ms)) = db::fallback_file(pool, channel).await?
        && let Ok(info) = lisek_playback::transcode::probe(Path::new(&path), settings).await
    {
        return Ok(Plan {
            path: Some(path),
            seek: 0.0,
            duration: gap.min(f64::from(duration_ms) / 1000.0),
            has_audio: info.audio_channels > 0,
            video_index: info.video_index,
            label: "fallback".into(),
        });
    }
    Ok(slate(gap.min(SLATE_SECONDS), "slate".into()))
}

fn slate(duration: f64, label: String) -> Plan {
    Plan { path: None, seek: 0.0, duration, has_audio: false, video_index: None, label }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(path: Option<&str>, has_audio: bool) -> Plan {
        Plan { path: path.map(String::from), seek: 12.5, duration: 30.0, has_audio, video_index: Some(2), label: "x".into() }
    }

    #[test]
    fn a_file_is_seeked_read_in_real_time_and_cut_to_length() {
        let args = encoder_args(&plan(Some("/m/film.mkv"), true), "720p", 100.0).join(" ");
        assert!(args.contains("-re -ss 12.500 -i /m/film.mkv"), "{args}");
        assert!(args.contains("-map 0:2 -map 0:a:0"));
        assert!(args.contains("-t 30.000"));
        assert!(args.contains("-output_ts_offset 100.000"));
        assert!(args.contains("scale=1280:720"));
        assert!(args.ends_with("-f mpegts -"));
        assert!(!args.contains("anullsrc"));
    }

    #[test]
    fn a_file_without_audio_gets_silence() {
        let args = encoder_args(&plan(Some("/m/clip.avi"), false), "480p", 0.0).join(" ");
        assert!(args.contains("anullsrc") && args.contains("-map 1:a:0"));
        assert!(args.contains("scale=854:480"));
    }

    #[test]
    fn the_slate_is_generated_not_read() {
        let args = encoder_args(&slate(10.0, "slate".into()), "1080p", 5.0).join(" ");
        assert!(args.contains("color=c=0x14110a:s=1920x1080:r=25") && args.contains("anullsrc"));
        assert!(!args.contains("-ss"));
    }

    #[test]
    fn every_piece_is_encoded_the_same_way() {
        let a = encoder_args(&plan(Some("/m/a.mp4"), true), "720p", 0.0);
        let b = encoder_args(&plan(None, false), "720p", 0.0);
        for needle in ["libx264", "aac", "-ar 48000", "-ac 2", "-g 50", "fps=25"] {
            assert!(a.join(" ").contains(needle) && b.join(" ").contains(needle), "{needle}");
        }
    }

    #[test]
    fn the_segmenter_writes_a_live_window() {
        let args = segmenter_args(Path::new("/tmp/tv")).join(" ");
        assert!(args.contains("-f mpegts -i pipe:0 -c copy"));
        assert!(args.contains("delete_segments") && args.contains("omit_endlist"));
        assert!(args.ends_with("index.m3u8"));
    }

    #[test]
    fn unknown_quality_means_720p() {
        assert_eq!(quality_profile("4k"), (1280, 720, 3000));
    }

    #[test]
    fn viewers_have_one_channel_each_and_need_their_ticket() {
        let sessions = TvSessions::default();
        let (user, channel) = (Uuid::new_v4(), Uuid::new_v4());
        let (first, ticket) = sessions.add_viewer(user, channel);
        assert!(sessions.touch_viewer(first, &ticket).is_none(), "no stream is running");
        let (second, _) = sessions.add_viewer(user, channel);
        assert!(!sessions.stop_viewer(first, user), "the first viewer was replaced");
        assert!(!sessions.stop_viewer(second, Uuid::new_v4()), "only the owner stops it");
        assert!(sessions.stop_viewer(second, user));
    }
}
