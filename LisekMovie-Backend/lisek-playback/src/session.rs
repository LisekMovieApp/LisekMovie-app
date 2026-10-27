//! HLS playback sessions.
//!
//! Each session runs at most one FFmpeg process ("run") at a time. A run
//! writes fragmented MP4 to a pipe; this module groups the fragments into
//! the segments of a [`SegmentMap`] and writes each one to disk when it is
//! complete. No `-re`: FFmpeg runs at full speed until it is
//! `throttle_lead_seconds` ahead of the segment the player last asked for,
//! then the segmenter simply stops reading the pipe. FFmpeg blocks on its
//! next write and uses no CPU, GPU or disk until the player catches up to
//! half the lead (back-pressure, so it works the same on Linux and
//! Windows). A player that stays paused for two minutes loses the process
//! altogether, which frees the NVENC session.
//!
//! A request for a segment that is not on disk either waits for the
//! current run (when it is about to produce it) or restarts FFmpeg at
//! that segment: that is a seek. Segments further than
//! `segment_retention_seconds` behind the player, or far ahead of it after
//! a seek back, are deleted as playback moves.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use lisek_core::playback_settings::PlaybackSettings;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, Notify, watch};
use tokio::task::JoinHandle;

use crate::command::{self, OUTPUT_TS_OFFSET, RunArgs};
use crate::decision::{AudioAction, PlaybackPlan};
use crate::fmp4::{self, Track};
use crate::hls::{self, Rendition, Variant};
use crate::keyframes::SegmentMap;
use crate::subtitles::{self, CueStore, ExternalSubtitle, TrackRef};
use crate::transcode::{self, FfmpegTools};

/// A run paused by throttling for this long without any request is
/// stopped; the next request starts a new one.
const IDLE_RUN_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a request waits for its segment.
const SEGMENT_TIMEOUT: Duration = Duration::from_secs(90);
/// A segment this close after the one FFmpeg is writing is waited for
/// rather than restarting FFmpeg at it.
const WAIT_AHEAD_SECONDS: f64 = 20.0;
/// Longest wait for a subtitle segment's cues. Shorter than hls.js's
/// time-to-first-byte timeout (10 s): an empty answer leaves a gap, a
/// timed-out one stalls the player's subtitle loading.
const SUBTITLE_WAIT: Duration = Duration::from_secs(8);
/// How long a subtitle segment no run is heading for waits for one to
/// start (a seek back: the video request restarting FFmpeg there usually
/// arrives together with the subtitle request).
const SUBTITLE_GRACE: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionError {
    NotFound,
    CapacityReached,
    Failed(String),
    Timeout,
    Io,
}

impl SessionError {
    pub fn code(&self) -> &'static str {
        match self {
            SessionError::NotFound => "segment_not_found",
            SessionError::CapacityReached => "transcode_capacity_reached",
            SessionError::Failed(_) => "ffmpeg_failed",
            SessionError::Timeout => "segment_not_ready",
            SessionError::Io => "transcode_directory_unavailable",
        }
    }
}

/// Everything a session needs, fixed when it starts.
pub struct SessionSpec {
    pub id: String,
    pub input: PathBuf,
    pub directory: PathBuf,
    pub plan: PlaybackPlan,
    pub segments: SegmentMap,
    /// The file's start time; playlist time zero.
    pub origin: f64,
    pub settings: PlaybackSettings,
    pub tools: FfmpegTools,
    /// Where the first run starts (resume position).
    pub start_position: f64,
    /// Embedded text subtitle streams exported as WebVTT by every run.
    pub text_subtitles: Vec<i32>,
    pub externals: Vec<ExternalSubtitle>,
    /// Subtitle renditions listed in the master playlist.
    pub renditions: Vec<Rendition>,
    pub frame_rate: Option<f64>,
    /// The device this session transcodes on (`gpu::Target::id`), if the
    /// plan uses one (Faza 14 load balancing counts sessions by it).
    pub hw_target: Option<String>,
    /// Keeps the device counted while the session starts.
    pub reservation: Option<crate::gpu::Reservation>,
}

struct RunHandle {
    seq: u64,
    start: usize,
    /// The segment being written; all before it (from `start`) are done.
    next: usize,
    finished: bool,
    failure: Option<String>,
    task: JoinHandle<()>,
}

#[derive(Default)]
struct State {
    produced: BTreeSet<usize>,
    tracks: Option<Vec<Track>>,
    run: Option<RunHandle>,
    runs: u64,
    cues: HashMap<i32, CueStore>,
    external_vtt: HashMap<usize, Arc<str>>,
}

pub struct Session {
    spec: SessionSpec,
    /// `-sub_charenc` for an external text subtitle being burnt in.
    burn_in_charset: Option<String>,
    state: Mutex<State>,
    changed: Notify,
    playhead: watch::Sender<usize>,
    touched: std::sync::Mutex<Instant>,
}

static SESSIONS: LazyLock<std::sync::Mutex<HashMap<String, Arc<Session>>>> =
    LazyLock::new(Default::default);

fn registry() -> std::sync::MutexGuard<'static, HashMap<String, Arc<Session>>> {
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// Sessions encoding video right now. Background jobs (trickplay) wait
/// while there are any, so they never compete with a viewer for the CPU.
pub fn encoding_sessions() -> usize {
    registry().values().filter(|s| s.spec.plan.encodes_video()).count()
}

/// Sessions per transcoding device right now (`gpu::Target::id`).
pub fn target_sessions() -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for session in registry().values() {
        if let Some(id) = &session.spec.hw_target {
            *counts.entry(id.clone()).or_insert(0) += 1;
        }
    }
    counts
}

/// Starts a session and its first run at the resume position. Its device
/// was chosen before planning (`gpu::reserve`); the reservation ends once
/// the session is registered and counts by itself.
pub async fn create(mut spec: SessionSpec) -> Result<Arc<Session>, SessionError> {
    let reservation = spec.reservation.take();
    if spec.plan.encodes_video() {
        let encoding = registry().values().filter(|s| s.spec.plan.encodes_video()).count();
        if encoding >= spec.settings.max_concurrent_transcodes as usize {
            return Err(SessionError::CapacityReached);
        }
    }
    tokio::fs::create_dir_all(&spec.directory).await.map_err(|_| SessionError::Io)?;
    let first = spec.segments.segment_at(spec.start_position);
    let burn_in_charset = match spec.plan.subtitle.as_ref().map(|s| (s.track, s.delivery)) {
        Some((TrackRef::External(n), crate::decision::SubtitleDelivery::BurnIn)) => {
            match spec.externals.get(n).map(|e| e.path.clone()) {
                Some(path) => tokio::fs::read(&path)
                    .await
                    .ok()
                    .filter(|bytes| subtitles::needs_charset(bytes))
                    .map(|_| spec.settings.subtitle_fallback_charset.clone()),
                None => None,
            }
        }
        _ => None,
    };
    let session = Arc::new(Session {
        burn_in_charset,
        playhead: watch::Sender::new(first),
        spec,
        state: Mutex::new(State::default()),
        changed: Notify::new(),
        touched: std::sync::Mutex::new(Instant::now()),
    });
    {
        let mut state = session.state.lock().await;
        session.start_run(&mut state, first)?;
    }
    registry().insert(session.spec.id.clone(), session.clone());
    drop(reservation);
    Ok(session)
}

pub fn get(id: &str) -> Option<Arc<Session>> {
    registry().get(id).cloned()
}

pub fn exists(id: &str) -> bool {
    registry().contains_key(id)
}

/// Marks a session as in use (progress reports); `false` if unknown.
pub fn touch(id: &str) -> bool {
    get(id).map(|s| s.touch()).is_some()
}

/// Stops FFmpeg and deletes the session's files.
pub async fn stop(id: &str) {
    let Some(session) = registry().remove(id) else { return };
    let run = session.state.lock().await.run.take();
    if let Some(run) = run {
        run.task.abort();
        let _ = run.task.await;
    }
    let _ = tokio::fs::remove_dir_all(&session.spec.directory).await;
}

/// Stops sessions nobody has touched for `max_idle`.
pub async fn stop_idle(max_idle: Duration) {
    let idle: Vec<String> = registry()
        .iter()
        .filter(|(_, s)| s.idle_for() > max_idle)
        .map(|(id, _)| id.clone())
        .collect();
    for id in idle {
        stop(&id).await;
    }
}

/// Segments to delete: further than `retention` seconds behind the
/// player, or beyond what throttling would produce ahead of it.
pub fn segments_to_prune(
    produced: &BTreeSet<usize>,
    requested: usize,
    map: &SegmentMap,
    retention: f64,
    lead: f64,
) -> Vec<usize> {
    let position = map.start(requested);
    let keep_from = position - retention;
    let keep_until = position + lead + map.longest() + WAIT_AHEAD_SECONDS;
    produced
        .iter()
        .copied()
        .filter(|&i| i != requested && (map.end(i) < keep_from || map.start(i) > keep_until))
        .collect()
}

impl Session {
    pub fn spec(&self) -> &SessionSpec {
        &self.spec
    }

    pub fn touch(&self) {
        *self.touched.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.touched.lock().unwrap_or_else(|e| e.into_inner()).elapsed()
    }

    fn segment_path(&self, index: usize) -> PathBuf {
        self.spec.directory.join(hls::segment_name(index))
    }

    fn init_path(&self) -> PathBuf {
        self.spec.directory.join("init.mp4")
    }

    /// `?ticket=...`-style suffix for every URI in a playlist.
    pub fn video_playlist(&self, query: &str) -> String {
        hls::video_playlist(&self.spec.segments, query)
    }

    pub async fn master_playlist(&self, query: &str) -> Result<String, SessionError> {
        let tracks = self.wait_for_init().await?;
        let codecs: Option<Vec<String>> = tracks.iter().map(|t| t.codec.clone()).collect();
        let plan = &self.spec.plan;
        let audio_kbps = match plan.audio.as_ref().map(|a| &a.action) {
            Some(AudioAction::Encode { bitrate_kbps, .. }) => *bitrate_kbps,
            Some(AudioAction::Copy) => 640,
            None => 0,
        };
        let variant = Variant {
            bandwidth_bps: (plan.output_bitrate_kbps.max(1000) as u64 + audio_kbps as u64) * 1100,
            codecs: codecs.map(|c| c.join(",")),
            width: plan.output_width,
            height: plan.output_height,
        };
        Ok(hls::master_playlist(&variant, &self.spec.renditions, query))
    }

    pub fn subtitle_playlist(&self, track: TrackRef, query: &str) -> Result<String, SessionError> {
        let segmented = match track {
            TrackRef::Embedded(index) if self.spec.text_subtitles.contains(&index) => true,
            TrackRef::External(n) if n < self.spec.externals.len() => false,
            _ => return Err(SessionError::NotFound),
        };
        Ok(hls::subtitle_playlist(&self.spec.segments, track, segmented, query))
    }

    async fn wait_for_init(&self) -> Result<Vec<Track>, SessionError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.state.lock().await;
                if let Some(tracks) = &state.tracks {
                    return Ok(tracks.clone());
                }
                if let Some(failure) = state.run.as_ref().and_then(|run| run.failure.as_ref()) {
                    return Err(SessionError::Failed(failure.clone()));
                }
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Err(SessionError::Timeout);
            }
        }
    }

    pub async fn init_segment(&self) -> Result<PathBuf, SessionError> {
        self.touch();
        self.wait_for_init().await.map(|_| self.init_path())
    }

    /// The file of media segment `index`, producing it first if needed.
    pub async fn media_segment(self: &Arc<Self>, index: usize) -> Result<PathBuf, SessionError> {
        if index >= self.spec.segments.len() {
            return Err(SessionError::NotFound);
        }
        self.touch();
        self.playhead.send_replace(index);
        let deadline = tokio::time::Instant::now() + SEGMENT_TIMEOUT;
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = self.state.lock().await;
                if state.produced.contains(&index) {
                    let path = self.segment_path(index);
                    if tokio::fs::try_exists(&path).await.unwrap_or(false) {
                        self.prune(&mut state, index).await;
                        return Ok(path);
                    }
                    state.produced.remove(&index);
                }
                let map = &self.spec.segments;
                let restart = match &state.run {
                    None => true,
                    Some(run) if run.failure.is_some() && index >= run.start && index <= run.next => {
                        return Err(SessionError::Failed(run.failure.clone().unwrap_or_default()));
                    }
                    Some(run) => {
                        run.finished
                            || index < run.start
                            || map.start(index) - map.start(run.next) > WAIT_AHEAD_SECONDS
                    }
                };
                if restart {
                    self.start_run(&mut state, index)?;
                }
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Err(SessionError::Timeout);
            }
        }
    }

    /// WebVTT for HLS subtitle segment `index` of `track`.
    pub async fn subtitle_segment(&self, track: TrackRef, index: usize) -> Result<String, SessionError> {
        self.touch();
        let mpegts = ((self.spec.origin + OUTPUT_TS_OFFSET) * 90_000.0).round().max(0.0) as u64;
        match track {
            TrackRef::External(n) if index == 0 => {
                let document = self.external_vtt(n).await?;
                // Converted cues are already on the playlist timeline.
                let (cues, _) = subtitles::parse_vtt(&document, 0.0, true);
                Ok(subtitles::vtt_document(cues.iter(), Some(mpegts)))
            }
            TrackRef::Embedded(stream) if self.spec.text_subtitles.contains(&stream) => {
                if index >= self.spec.segments.len() {
                    return Err(SessionError::NotFound);
                }
                // Cues up to a segment's end have been read once FFmpeg has
                // moved past that segment. Never restart FFmpeg for them.
                // A segment before the run's start (hls.js loads one segment
                // before the start position) is never reached by this run:
                // answer it empty after a short grace instead of making the
                // player wait.
                let started = tokio::time::Instant::now();
                let deadline = started + SUBTITLE_WAIT;
                let grace = started + SUBTITLE_GRACE;
                loop {
                    let notified = self.changed.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    {
                        let state = self.state.lock().await;
                        let passed = state.produced.contains(&index)
                            || state.run.as_ref().is_some_and(|r| {
                                (r.start <= index && r.next > index) || (r.finished && r.failure.is_none() && r.start <= index)
                            });
                        if passed {
                            let (start, end) = (self.spec.segments.start(index), self.spec.segments.end(index));
                            let cues = state.cues.get(&stream);
                            let document = match cues {
                                Some(store) => subtitles::vtt_document(store.overlapping(start, end), Some(mpegts)),
                                None => subtitles::vtt_document(std::iter::empty(), Some(mpegts)),
                            };
                            return Ok(document);
                        }
                        let heading_there = state.run.as_ref().is_some_and(|r| r.start <= index && !r.finished);
                        if !heading_there && tokio::time::Instant::now() >= grace {
                            return Ok(subtitles::vtt_document(std::iter::empty(), Some(mpegts)));
                        }
                    }
                    let wake = if tokio::time::Instant::now() < grace { grace.min(deadline) } else { deadline };
                    if tokio::time::timeout_at(wake, notified).await.is_err() && wake == deadline {
                        return Ok(subtitles::vtt_document(std::iter::empty(), Some(mpegts)));
                    }
                }
            }
            _ => Err(SessionError::NotFound),
        }
    }

    /// An external subtitle file as WebVTT (cached per session).
    pub async fn external_vtt(&self, n: usize) -> Result<Arc<str>, SessionError> {
        if let Some(doc) = self.state.lock().await.external_vtt.get(&n) {
            return Ok(doc.clone());
        }
        let external = self.spec.externals.get(n).ok_or(SessionError::NotFound)?;
        let document: Arc<str> =
            convert_external(external, &self.spec.settings, self.spec.frame_rate)
                .await
                .map_err(SessionError::Failed)?
                .into();
        self.state.lock().await.external_vtt.insert(n, document.clone());
        Ok(document)
    }

    async fn prune(&self, state: &mut State, requested: usize) {
        let settings = &self.spec.settings;
        for index in segments_to_prune(
            &state.produced,
            requested,
            &self.spec.segments,
            settings.segment_retention_seconds as f64,
            settings.throttle_lead_seconds as f64,
        ) {
            state.produced.remove(&index);
            let _ = tokio::fs::remove_file(self.segment_path(index)).await;
        }
    }

    /// Replaces the current run with one starting at segment `start`.
    fn start_run(self: &Arc<Self>, state: &mut State, start: usize) -> Result<(), SessionError> {
        if let Some(old) = state.run.take() {
            old.task.abort();
        }
        state.runs += 1;
        let seq = state.runs;
        let spec = &self.spec;
        let subtitle_outputs: Vec<(i32, PathBuf)> = spec
            .text_subtitles
            .iter()
            .map(|index| (*index, spec.directory.join(format!("sub-{index}-r{seq}.vtt"))))
            .collect();
        let args = command::hls_run_args(&RunArgs {
            input: &spec.input,
            plan: &spec.plan,
            settings: &spec.settings,
            tools: &spec.tools,
            start: spec.segments.start(start),
            origin: spec.origin,
            segment_seconds: spec.settings.hls_segment_seconds as f64,
            subtitle_outputs: &subtitle_outputs,
            external_charset: self.burn_in_charset.as_deref(),
        });
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(spec.directory.join("ffmpeg.log"))
            .map_err(|_| SessionError::Io)?;
        let mut cmd = transcode::command("ffmpeg", &spec.settings);
        cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(log);
        tracing::debug!(session = %spec.id, segment = start, ?args, "starting ffmpeg");
        let child = cmd
            .spawn()
            .map_err(|_| SessionError::Failed("ffmpeg_unavailable".into()))?;
        let task = tokio::spawn(run(self.clone(), seq, start, child, subtitle_outputs));
        state.run = Some(RunHandle { seq, start, next: start, finished: false, failure: None, task });
        self.changed.notify_waiters();
        Ok(())
    }

    /// Waits while FFmpeg is far enough ahead of the player. `false` when
    /// the run should end (the player went away or stayed paused).
    async fn throttle(&self, next: usize) -> bool {
        let settings = &self.spec.settings;
        if !settings.throttling {
            return true;
        }
        let map = &self.spec.segments;
        let lead = settings.throttle_lead_seconds as f64;
        let mut playhead = self.playhead.subscribe();
        let mut paused = false;
        loop {
            let requested = *playhead.borrow_and_update();
            let ahead = map.start(next) - map.start(requested);
            let limit = if paused { lead / 2.0 } else { lead };
            if ahead < limit {
                if paused {
                    tracing::trace!(session = %self.spec.id, next, requested, "ffmpeg resumed");
                }
                return true;
            }
            if !paused {
                tracing::trace!(session = %self.spec.id, next, requested, "ffmpeg throttled");
                paused = true;
            }
            match tokio::time::timeout(IDLE_RUN_TIMEOUT, playhead.changed()).await {
                Ok(Ok(())) => continue,
                Ok(Err(_)) | Err(_) => return false,
            }
        }
    }

    /// Applies `update` to run `seq` if it is still the current run.
    async fn update_run(&self, seq: u64, update: impl FnOnce(&mut RunHandle, &mut BTreeSet<usize>)) {
        let mut state = self.state.lock().await;
        let State { run, produced, .. } = &mut *state;
        if let Some(run) = run.as_mut().filter(|r| r.seq == seq) {
            update(run, produced);
        }
        drop(state);
        self.changed.notify_waiters();
    }
}

/// Reads newly written cues from a run's WebVTT outputs.
struct CueReader {
    stream: i32,
    path: PathBuf,
    offset: u64,
    pending: Vec<u8>,
}

impl CueReader {
    async fn read(&mut self, origin: f64, at_eof: bool) -> Vec<subtitles::Cue> {
        use tokio::io::{AsyncReadExt, AsyncSeekExt};
        let Ok(mut file) = tokio::fs::File::open(&self.path).await else { return vec![] };
        if file.seek(std::io::SeekFrom::Start(self.offset)).await.is_err() {
            return vec![];
        }
        let mut buf = vec![];
        if let Ok(n) = file.read_to_end(&mut buf).await {
            self.offset += n as u64;
            self.pending.extend_from_slice(&buf);
        }
        let valid = match std::str::from_utf8(&self.pending) {
            Ok(text) => text.len(),
            Err(e) => e.valid_up_to(),
        };
        let text = std::str::from_utf8(&self.pending[..valid]).unwrap_or_default();
        let (cues, used) = subtitles::parse_vtt(text, origin, at_eof);
        self.pending.drain(..used);
        cues
    }
}

/// One FFmpeg run: init segment, then fragments grouped into segments.
async fn run(
    session: Arc<Session>,
    seq: u64,
    start: usize,
    mut child: tokio::process::Child,
    subtitle_outputs: Vec<(i32, PathBuf)>,
) {
    let outcome = segment_stream(&session, seq, start, &mut child, &subtitle_outputs).await;
    let failure = match outcome {
        Ok(true) => match child.wait().await {
            Ok(status) if status.success() => None,
            _ => Some("ffmpeg_failed".to_owned()),
        },
        Ok(false) => {
            // Stopped by throttling: kill FFmpeg (frees encoder sessions).
            let _ = child.kill().await;
            None
        }
        Err(err) => {
            let _ = child.kill().await;
            Some(err)
        }
    };
    if let Some(failure) = &failure {
        let tail = log_tail(&session.spec.directory.join("ffmpeg.log")).await;
        tracing::warn!(session = %session.spec.id, %failure, ffmpeg = %tail, "ffmpeg run failed");
    }
    session
        .update_run(seq, |run, _| {
            run.finished = true;
            run.failure = failure;
        })
        .await;
}

async fn log_tail(path: &Path) -> String {
    let text = tokio::fs::read(path).await.unwrap_or_default();
    let tail = &text[text.len().saturating_sub(1500)..];
    String::from_utf8_lossy(tail).trim().to_owned()
}

/// `Ok(true)`: FFmpeg's output ended; `Ok(false)`: stopped while
/// throttled; `Err`: broken output.
async fn segment_stream(
    session: &Arc<Session>,
    seq: u64,
    start: usize,
    child: &mut tokio::process::Child,
    subtitle_outputs: &[(i32, PathBuf)],
) -> Result<bool, String> {
    let spec = &session.spec;
    let map = &spec.segments;
    let stdout = child.stdout.take().ok_or("no_ffmpeg_output")?;
    let mut reader = BufReader::with_capacity(1 << 20, stdout);
    let mut cue_readers: Vec<CueReader> = subtitle_outputs
        .iter()
        .map(|(stream, path)| CueReader { stream: *stream, path: path.clone(), offset: 0, pending: vec![] })
        .collect();

    // Init segment: everything before the first fragment.
    let mut init = vec![];
    let mut moov = None;
    let first_moof = loop {
        let Some(mp4) = fmp4::read_box(&mut reader).await.map_err(|e| e.to_string())? else {
            return Err("ffmpeg_produced_no_media".into());
        };
        match &mp4.kind {
            b"moof" => break mp4,
            b"moov" => moov = Some(mp4.bytes.clone()),
            _ => {}
        }
        init.extend_from_slice(&mp4.bytes);
    };
    let tracks = fmp4::parse_init(&moov.ok_or("no_moov")?);
    let video = tracks
        .iter()
        .find(|t| &t.handler == b"vide")
        .cloned()
        .ok_or("no_video_track")?;
    {
        let mut state = session.state.lock().await;
        if state.tracks.is_none() {
            write_atomically(&session.init_path(), &init).await.map_err(|e| e.to_string())?;
            state.tracks = Some(tracks);
        }
    }
    session.changed.notify_waiters();

    let mut current: Option<(usize, tokio::fs::File, PathBuf)> = None;
    let mut last_done: Option<usize> = None;
    let mut pending = Some(first_moof);
    loop {
        let moof = match pending.take() {
            Some(moof) => moof,
            None => match fmp4::read_box(&mut reader).await.map_err(|e| e.to_string())? {
                None => break,
                Some(b) if &b.kind == b"moof" => b,
                Some(_) => continue, // mfra and friends at the end
            },
        };
        let mdat = fmp4::read_box(&mut reader)
            .await
            .map_err(|e| e.to_string())?
            .filter(|b| &b.kind == b"mdat")
            .ok_or("fragment_without_mdat")?;
        // Playlist time of the fragment's first frame and its length.
        let timing = fmp4::fragment_timing(&moof.bytes, &video)
            .map(|(pts, duration)| (pts - OUTPUT_TS_OFFSET - spec.origin, duration));
        let segment = match (timing, &current) {
            (Some((time, _)), _) => map.segment_at(time),
            (None, Some(c)) => c.0,
            (None, None) => last_done.map_or(start, |d| d + 1),
        };
        if segment < start {
            continue; // FFmpeg seeked to an earlier keyframe
        }
        if current.as_ref().is_some_and(|c| c.0 != segment) {
            // A segment that ended short of its listed end (rounding).
            let done = current.take().expect("checked above");
            last_done = Some(done.0);
            finish_segment(session, seq, done, segment, &mut cue_readers).await?;
            if !session.throttle(segment).await {
                return Ok(false);
            }
        }
        if current.is_none() {
            if let Some(done) = last_done {
                fill_skipped(session, seq, done, segment).await;
            }
            let path = session.segment_path(segment).with_extension("m4s.part");
            let file = tokio::fs::File::create(&path).await.map_err(|e| e.to_string())?;
            current = Some((segment, file, path));
        }
        let (_, file, _) = current.as_mut().expect("segment opened above");
        file.write_all(&moof.bytes).await.map_err(|e| e.to_string())?;
        file.write_all(&mdat.bytes).await.map_err(|e| e.to_string())?;
        // Done once a fragment reaches the segment's end: no waiting for
        // the next fragment (a whole GOP later) to find out.
        if timing.is_some_and(|(time, duration)| time + duration >= map.end(segment) - 0.05) {
            let done = current.take().expect("opened above");
            last_done = Some(segment);
            finish_segment(session, seq, done, segment + 1, &mut cue_readers).await?;
            if !session.throttle(segment + 1).await {
                return Ok(false);
            }
        }
    }
    if let Some(done) = current.take() {
        finish_segment(session, seq, done, map.len(), &mut cue_readers).await?;
    }
    for reader in &mut cue_readers {
        let cues = reader.read(spec.origin, true).await;
        add_cues(session, reader.stream, cues).await;
    }
    Ok(true)
}

async fn add_cues(session: &Session, stream: i32, cues: Vec<subtitles::Cue>) {
    if cues.is_empty() {
        return;
    }
    let mut state = session.state.lock().await;
    let store = state.cues.entry(stream).or_default();
    for cue in cues {
        store.insert(cue);
    }
}

/// Moves a complete segment into place and records it. `next` is the
/// segment FFmpeg moved on to.
async fn finish_segment(
    session: &Arc<Session>,
    seq: u64,
    (index, mut file, part): (usize, tokio::fs::File, PathBuf),
    next: usize,
    cue_readers: &mut [CueReader],
) -> Result<(), String> {
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    tokio::fs::rename(&part, session.segment_path(index)).await.map_err(|e| e.to_string())?;
    for reader in cue_readers.iter_mut() {
        let cues = reader.read(session.spec.origin, false).await;
        add_cues(session, reader.stream, cues).await;
    }
    let requested = *session.playhead.borrow();
    session
        .update_run(seq, |run, produced| {
            produced.insert(index);
            run.next = next;
        })
        .await;
    let mut state = session.state.lock().await;
    session.prune(&mut state, requested).await;
    Ok(())
}

/// Segments a long GOP jumped over (no keyframe inside them) get a copy
/// of the previous segment, so the playlist has no holes; the player just
/// buffers that range again.
async fn fill_skipped(session: &Arc<Session>, seq: u64, done: usize, next: usize) {
    let source = session.segment_path(done);
    for skipped in done + 1..next.min(session.spec.segments.len()) {
        tracing::debug!(session = %session.spec.id, skipped, "segment without a keyframe");
        if tokio::fs::copy(&source, session.segment_path(skipped)).await.is_ok() {
            session.update_run(seq, |_, produced| { produced.insert(skipped); }).await;
        }
    }
}

async fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let part = path.with_extension("part");
    tokio::fs::write(&part, bytes).await?;
    tokio::fs::rename(&part, path).await
}

/// Converts an external subtitle file to WebVTT with FFmpeg: legacy
/// charsets (CP1250 for Polish `.srt`) and MicroDVD frame timing handled.
pub async fn convert_external(
    external: &ExternalSubtitle,
    settings: &PlaybackSettings,
    frame_rate: Option<f64>,
) -> Result<String, String> {
    if external.kind != subtitles::SubtitleKind::Text {
        return Err("subtitle_not_text".into());
    }
    let bytes = tokio::fs::read(&external.path).await.map_err(|_| "subtitle_unreadable")?;
    if bytes.len() > 20 << 20 {
        return Err("subtitle_too_large".into());
    }
    let charset = subtitles::needs_charset(&bytes).then_some(settings.subtitle_fallback_charset.as_str());
    let fps = subtitles::is_microdvd(&bytes).then(|| frame_rate.unwrap_or(23.976));
    let args = subtitles::external_to_vtt_args(&external.path, charset, fps);
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        transcode::command("ffmpeg", settings).args(args).output(),
    )
    .await
    .map_err(|_| "subtitle_conversion_timed_out")?
    .map_err(|_| "ffmpeg_unavailable")?;
    if !output.status.success() {
        return Err("subtitle_conversion_failed".into());
    }
    String::from_utf8(output.stdout).map_err(|_| "subtitle_conversion_failed".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prunes_behind_retention_and_far_ahead() {
        let map = SegmentMap::fixed(600.0, 4.0);
        let produced: BTreeSet<usize> = (0..150).collect();
        // Player at 300 s, keep 60 s behind, 30 s lead.
        let pruned = segments_to_prune(&produced, 75, &map, 60.0, 30.0);
        assert!(pruned.contains(&0) && pruned.contains(&58));
        assert!(!pruned.contains(&60), "60 ends at 244 s, inside the window");
        assert!(!pruned.contains(&75) && !pruned.contains(&88), "88 starts at 352 s");
        assert!(pruned.contains(&89) && pruned.contains(&149), "far ahead after a seek back");
        assert!(segments_to_prune(&BTreeSet::from([75]), 75, &map, 10.0, 10.0).is_empty());
    }
}
