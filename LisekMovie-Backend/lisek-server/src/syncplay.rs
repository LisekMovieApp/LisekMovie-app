//! Faza 20: watching together ("Syncplay"). A room plays one film or
//! episode for everyone in it: play, pause and seek from any member are
//! applied by every player in the room.
//!
//! Rooms live in memory (a restart ends them). `POST /api/syncplay/rooms`
//! creates one for an item and returns its six-character code; the link
//! `/together/<code>` opens the item's player in that room. Members talk
//! over a WebSocket authenticated like the notification socket — the
//! session in the first frame, never in a URL — and must be allowed to see
//! the item (parental controls).
//!
//! The server keeps the room's state (playing or paused, the position and
//! when it was set) and works out the current position from it. Every
//! member gets each change (except its own), a full member list when
//! someone joins or leaves, and the state every `STATE_EVERY`, so players
//! that drifted apart (buffering, a slow machine) are pulled back together.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Path, State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::{Semaphore, broadcast};
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::state::SharedState;

const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LENGTH: usize = 6;
pub const MAX_ROOMS: usize = 200;
pub const MAX_MEMBERS: usize = 16;
/// An empty room is kept this long (the creator is still opening the player).
const EMPTY_ROOM_TTL: Duration = Duration::from_secs(10 * 60);
const STATE_EVERY: Duration = Duration::from_secs(10);
/// At most this many commands per member per `COMMAND_WINDOW`.
const MAX_COMMANDS: usize = 20;
const COMMAND_WINDOW: Duration = Duration::from_secs(10);
const MAX_POSITION: f64 = 7.0 * 24.0 * 3600.0;

static CONNECTIONS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(512)));
static NEXT_CONNECTION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Play,
    Pause,
    Seek,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
pub struct Member {
    pub user_id: Uuid,
    pub username: String,
}

/// What the room broadcasts to its members.
#[derive(Clone, Debug)]
enum RoomEvent {
    Command { action: Action, position: f64, by: String, origin: u64 },
    Members(Vec<Member>),
}

struct Room {
    item_id: Uuid,
    title: String,
    playing: bool,
    position: f64,
    since: Instant,
    members: HashMap<u64, Member>,
    empty_since: Option<Instant>,
    events: broadcast::Sender<RoomEvent>,
}

impl Room {
    /// The position now: while playing, it moves on with the clock.
    fn current_position(&self, now: Instant) -> f64 {
        if self.playing {
            self.position + now.saturating_duration_since(self.since).as_secs_f64()
        } else {
            self.position
        }
    }
    fn member_list(&self) -> Vec<Member> {
        let mut list: Vec<Member> = self.members.values().cloned().collect();
        list.sort_by_key(|m| m.username.to_lowercase());
        list
    }
    fn apply(&mut self, action: Action, position: f64, now: Instant) {
        self.position = position;
        self.since = now;
        match action {
            Action::Play => self.playing = true,
            Action::Pause => self.playing = false,
            Action::Seek => {}
        }
    }
}

/// Every open room, by code.
#[derive(Default)]
pub struct Rooms {
    rooms: Mutex<HashMap<String, Room>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RoomError {
    TooManyRooms,
    NotFound,
    Full,
}

fn new_code() -> String {
    let random = Uuid::new_v4();
    random
        .as_bytes()
        .iter()
        .take(CODE_LENGTH)
        .map(|b| CODE_ALPHABET[usize::from(*b) % CODE_ALPHABET.len()] as char)
        .collect()
}

/// A code as typed by a person: case and spaces do not matter.
pub fn normalize_code(code: &str) -> Option<String> {
    let code: String = code.chars().filter(|c| !c.is_whitespace() && *c != '-').collect::<String>().to_uppercase();
    (code.len() == CODE_LENGTH && code.bytes().all(|b| CODE_ALPHABET.contains(&b))).then_some(code)
}

impl Rooms {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Room>> {
        self.rooms.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn prune(rooms: &mut HashMap<String, Room>, now: Instant) {
        rooms.retain(|_, room| room.empty_since.is_none_or(|since| now.duration_since(since) < EMPTY_ROOM_TTL));
    }

    pub fn create(&self, item_id: Uuid, title: String, position: f64) -> Result<String, RoomError> {
        let now = Instant::now();
        let mut rooms = self.lock();
        Self::prune(&mut rooms, now);
        if rooms.len() >= MAX_ROOMS {
            return Err(RoomError::TooManyRooms);
        }
        let code = loop {
            let code = new_code();
            if !rooms.contains_key(&code) {
                break code;
            }
        };
        let (events, _) = broadcast::channel(64);
        rooms.insert(
            code.clone(),
            Room {
                item_id,
                title,
                playing: false,
                position: position.clamp(0.0, MAX_POSITION),
                since: now,
                members: HashMap::new(),
                empty_since: Some(now),
                events,
            },
        );
        Ok(code)
    }

    /// The room's item, title, members and state.
    pub fn info(&self, code: &str) -> Option<RoomInfo> {
        let now = Instant::now();
        let mut rooms = self.lock();
        Self::prune(&mut rooms, now);
        rooms.get(code).map(|room| RoomInfo {
            code: code.to_string(),
            item_id: room.item_id,
            title: room.title.clone(),
            members: room.member_list(),
            playing: room.playing,
            position: room.current_position(now),
        })
    }

    fn join(&self, code: &str, connection: u64, member: Member) -> Result<(broadcast::Receiver<RoomEvent>, RoomInfo), RoomError> {
        let now = Instant::now();
        let mut rooms = self.lock();
        let room = rooms.get_mut(code).ok_or(RoomError::NotFound)?;
        if room.members.len() >= MAX_MEMBERS {
            return Err(RoomError::Full);
        }
        room.members.insert(connection, member);
        room.empty_since = None;
        let receiver = room.events.subscribe();
        let _ = room.events.send(RoomEvent::Members(room.member_list()));
        Ok((
            receiver,
            RoomInfo {
                code: code.to_string(),
                item_id: room.item_id,
                title: room.title.clone(),
                members: room.member_list(),
                playing: room.playing,
                position: room.current_position(now),
            },
        ))
    }

    fn leave(&self, code: &str, connection: u64) {
        let mut rooms = self.lock();
        if let Some(room) = rooms.get_mut(code) {
            room.members.remove(&connection);
            if room.members.is_empty() {
                room.empty_since = Some(Instant::now());
                // Nobody watching: the room waits paused where it was.
                let now = Instant::now();
                room.position = room.current_position(now);
                room.playing = false;
                room.since = now;
            } else {
                let _ = room.events.send(RoomEvent::Members(room.member_list()));
            }
        }
    }

    fn command(&self, code: &str, connection: u64, action: Action, position: f64) -> bool {
        let now = Instant::now();
        let mut rooms = self.lock();
        let Some(room) = rooms.get_mut(code) else { return false };
        let Some(by) = room.members.get(&connection).map(|m| m.username.clone()) else { return false };
        room.apply(action, position, now);
        let _ = room.events.send(RoomEvent::Command { action, position, by, origin: connection });
        true
    }

    fn state(&self, code: &str) -> Option<(bool, f64)> {
        let rooms = self.lock();
        rooms.get(code).map(|room| (room.playing, room.current_position(Instant::now())))
    }
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct RoomInfo {
    pub code: String,
    pub item_id: Uuid,
    pub title: String,
    pub members: Vec<Member>,
    pub playing: bool,
    pub position: f64,
}

pub struct SyncError(StatusCode, &'static str);
impl IntoResponse for SyncError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error_code": self.1 }))).into_response()
    }
}
fn room_not_found() -> SyncError {
    SyncError(StatusCode::NOT_FOUND, "room_not_found")
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct CreateRoom {
    pub item_id: Uuid,
    /// Where the room starts (seconds); 0 when omitted.
    #[serde(default)]
    pub position: f64,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CreatedRoom {
    pub code: String,
    pub item_id: Uuid,
    /// Path to share: opens the player in this room.
    pub link: String,
}

/// Opens a room for a film or an episode.
#[utoipa::path(post,path="/api/syncplay/rooms",tag="media",request_body=CreateRoom,responses((status=200,body=CreatedRoom),(status=401),(status=404),(status=429,description="too_many_rooms")),security(("lisek_auth"=[])))]
pub async fn create_room(
    auth: AuthUser,
    State(state): State<SharedState>,
    Json(req): Json<CreateRoom>,
) -> Result<Json<CreatedRoom>, SyncError> {
    let pool = state.pool().ok_or(SyncError(StatusCode::SERVICE_UNAVAILABLE, "server_not_ready"))?;
    let not_found = || SyncError(StatusCode::NOT_FOUND, "media_not_found");
    if !auth.can_see(&pool, req.item_id).await.map_err(|_| SyncError(StatusCode::INTERNAL_SERVER_ERROR, "internal_error"))? {
        return Err(not_found());
    }
    let item = lisek_db::get_media_item_detail(&pool, req.item_id)
        .await
        .map_err(|_| SyncError(StatusCode::INTERNAL_SERVER_ERROR, "internal_error"))?
        .ok_or_else(not_found)?;
    if !matches!(item.item_type.as_str(), "movie" | "episode") {
        return Err(not_found());
    }
    let position = if req.position.is_finite() { req.position } else { 0.0 };
    let code = state
        .syncplay
        .create(item.id, item.title.clone(), position)
        .map_err(|_| SyncError(StatusCode::TOO_MANY_REQUESTS, "too_many_rooms"))?;
    tracing::info!(%code, item_id = %item.id, user = %auth.user.username, "watch-together room opened");
    Ok(Json(CreatedRoom { link: format!("/together/{code}"), code, item_id: item.id }))
}

/// A room's item, members and state (for the join link).
#[utoipa::path(get,path="/api/syncplay/rooms/{code}",tag="media",params(("code"=String,Path)),responses((status=200,body=RoomInfo),(status=401),(status=404)),security(("lisek_auth"=[])))]
pub async fn get_room(
    auth: AuthUser,
    State(state): State<SharedState>,
    Path(code): Path<String>,
) -> Result<Json<RoomInfo>, SyncError> {
    let code = normalize_code(&code).ok_or_else(room_not_found)?;
    let info = state.syncplay.info(&code).ok_or_else(room_not_found)?;
    let pool = state.pool().ok_or(SyncError(StatusCode::SERVICE_UNAVAILABLE, "server_not_ready"))?;
    // A room for something this account may not watch does not exist for it.
    if !auth.can_see(&pool, info.item_id).await.unwrap_or(false) {
        return Err(room_not_found());
    }
    Ok(Json(info))
}

#[derive(Deserialize)]
struct Hello {
    token: String,
    code: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Play { position: f64 },
    Pause { position: f64 },
    Seek { position: f64 },
    /// Asks for the room's state now.
    State,
}

#[utoipa::path(get,path="/api/syncplay/socket",tag="media",responses((status=101,description="WebSocket. First frame: JSON {token, code}. Server sends joined (room, members, state), then members, command {action: play|pause|seek, position, by} and state {playing, position} every 10 s. Client sends {type: play|pause|seek, position} or {type: state}. Errors: {type: error, code: room_not_found|room_full|unauthorized}."),(status=503)))]
pub async fn socket(State(state): State<SharedState>, ws: WebSocketUpgrade) -> Result<Response, StatusCode> {
    let permit = CONNECTIONS.clone().try_acquire_owned().map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(ws.max_message_size(4096).max_frame_size(4096).on_upgrade(move |socket| async move {
        let _permit = permit;
        run(socket, state).await;
    }))
}

async fn send_json(socket: &mut WebSocket, value: serde_json::Value) -> bool {
    socket.send(Message::Text(value.to_string().into())).await.is_ok()
}

async fn refuse(mut socket: WebSocket, code: &str) {
    let _ = send_json(&mut socket, serde_json::json!({"type": "error", "code": code})).await;
    let _ = socket.send(Message::Close(None)).await;
}

fn valid_position(position: f64) -> bool {
    position.is_finite() && (0.0..=MAX_POSITION).contains(&position)
}

async fn run(mut socket: WebSocket, state: SharedState) {
    let Some(pool) = state.pool() else { return };
    let Ok(Some(Ok(Message::Text(text)))) = tokio::time::timeout(Duration::from_secs(5), socket.recv()).await else {
        return;
    };
    let Ok(hello) = serde_json::from_str::<Hello>(&text) else {
        return refuse(socket, "unauthorized").await;
    };
    if hello.token.is_empty() || hello.token.len() > 1024 {
        return refuse(socket, "unauthorized").await;
    }
    let hash = lisek_auth::hash_session_token(&hello.token);
    let (user, access) = match lisek_db::access::find_session(&pool, &hash).await {
        Ok(Some((user, access))) if !user.must_change_password => (user, access),
        _ => return refuse(socket, "unauthorized").await,
    };
    let Some(code) = normalize_code(&hello.code) else {
        return refuse(socket, "room_not_found").await;
    };
    let Some(info) = state.syncplay.info(&code) else {
        return refuse(socket, "room_not_found").await;
    };
    let visible = access.browses_everything()
        || lisek_db::access::item_visible(&pool, info.item_id, &access).await.ok().flatten().unwrap_or(false);
    if !visible {
        return refuse(socket, "room_not_found").await;
    }
    let connection = NEXT_CONNECTION.fetch_add(1, Ordering::Relaxed);
    let member = Member { user_id: user.id, username: user.username.clone() };
    let (mut events, info) = match state.syncplay.join(&code, connection, member) {
        Ok(joined) => joined,
        Err(RoomError::Full) => return refuse(socket, "room_full").await,
        Err(_) => return refuse(socket, "room_not_found").await,
    };
    tracing::debug!(%code, user = %user.username, "joined watch-together room");
    let joined = serde_json::json!({
        "type": "joined",
        "code": info.code,
        "item_id": info.item_id,
        "title": info.title,
        "members": info.members,
        "playing": info.playing,
        "position": info.position,
    });
    if send_json(&mut socket, joined).await {
        serve(&mut socket, &state, &code, connection, &mut events).await;
    }
    state.syncplay.leave(&code, connection);
    let _ = socket.send(Message::Close(None)).await;
}

async fn serve(
    socket: &mut WebSocket,
    state: &SharedState,
    code: &str,
    connection: u64,
    events: &mut broadcast::Receiver<RoomEvent>,
) {
    let mut ticker = tokio::time::interval(STATE_EVERY);
    ticker.tick().await;
    let mut recent: VecDeque<Instant> = VecDeque::new();
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    _ => return,
                };
                let Ok(message) = serde_json::from_str::<ClientMessage>(&text) else { continue };
                let (action, position) = match message {
                    ClientMessage::Play { position } => (Action::Play, position),
                    ClientMessage::Pause { position } => (Action::Pause, position),
                    ClientMessage::Seek { position } => (Action::Seek, position),
                    ClientMessage::State => {
                        let Some((playing, position)) = state.syncplay.state(code) else { return };
                        if !send_json(socket, serde_json::json!({"type": "state", "playing": playing, "position": position})).await { return; }
                        continue;
                    }
                };
                if !valid_position(position) { continue; }
                let now = Instant::now();
                while recent.front().is_some_and(|at| now.duration_since(*at) > COMMAND_WINDOW) { recent.pop_front(); }
                if recent.len() >= MAX_COMMANDS { continue; }
                recent.push_back(now);
                if !state.syncplay.command(code, connection, action, position) { return; }
            }
            event = events.recv() => match event {
                Ok(RoomEvent::Command { origin, .. }) if origin == connection => {}
                Ok(RoomEvent::Command { action, position, by, .. }) => {
                    if !send_json(socket, serde_json::json!({"type": "command", "action": action, "position": position, "by": by})).await { return; }
                }
                Ok(RoomEvent::Members(members)) => {
                    if !send_json(socket, serde_json::json!({"type": "members", "members": members})).await { return; }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            },
            _ = ticker.tick() => {
                let Some((playing, position)) = state.syncplay.state(code) else { return };
                if !send_json(socket, serde_json::json!({"type": "state", "playing": playing, "position": position})).await { return; }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str) -> Member {
        Member { user_id: Uuid::new_v4(), username: name.into() }
    }

    #[test]
    fn codes_are_short_and_unambiguous() {
        let code = new_code();
        assert_eq!(code.len(), CODE_LENGTH);
        assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        assert_eq!(normalize_code(" ab3-de "), None, "too short");
        assert_eq!(normalize_code(" ab3-dez ").as_deref(), Some("AB3DEZ"));
        assert_eq!(normalize_code("abc-def").as_deref(), Some("ABCDEF"));
        assert_eq!(normalize_code("ABCDE0"), None, "0 looks like O");
    }

    #[test]
    fn commands_reach_the_other_members_and_move_the_state() {
        let rooms = Rooms::default();
        let code = rooms.create(Uuid::nil(), "Film".into(), 12.0).unwrap();
        let (mut anna, info) = rooms.join(&code, 1, member("Anna")).unwrap();
        assert!(!info.playing);
        assert_eq!(info.position, 12.0);
        let (mut bartek, info) = rooms.join(&code, 2, member("Bartek")).unwrap();
        assert_eq!(info.members.len(), 2);
        // Member lists after each join.
        assert!(matches!(anna.try_recv().unwrap(), RoomEvent::Members(m) if m.len() == 1));
        assert!(matches!(anna.try_recv().unwrap(), RoomEvent::Members(m) if m.len() == 2));
        assert!(matches!(bartek.try_recv().unwrap(), RoomEvent::Members(m) if m.len() == 2));

        assert!(rooms.command(&code, 1, Action::Seek, 100.0));
        assert!(rooms.command(&code, 1, Action::Play, 100.0));
        match bartek.try_recv().unwrap() {
            RoomEvent::Command { action, position, by, origin } => {
                assert_eq!((action, position, by.as_str(), origin), (Action::Seek, 100.0, "Anna", 1));
            }
            other => panic!("{other:?}"),
        }
        let (playing, position) = rooms.state(&code).unwrap();
        assert!(playing && (100.0..101.0).contains(&position));
        // Not a member: ignored.
        assert!(!rooms.command(&code, 99, Action::Pause, 0.0));

        rooms.leave(&code, 1);
        rooms.leave(&code, 2);
        let (playing, _) = rooms.state(&code).unwrap();
        assert!(!playing, "an empty room waits paused");
        assert!(rooms.info(&code).unwrap().members.is_empty());
    }

    #[test]
    fn rooms_have_limits() {
        let rooms = Rooms::default();
        let code = rooms.create(Uuid::nil(), "Film".into(), 0.0).unwrap();
        for n in 0..MAX_MEMBERS as u64 {
            rooms.join(&code, n, member("x")).unwrap();
        }
        assert_eq!(rooms.join(&code, 999, member("y")).err(), Some(RoomError::Full));
        assert_eq!(rooms.join("ZZZZZZ", 1, member("y")).err(), Some(RoomError::NotFound));
        assert!(valid_position(0.0) && !valid_position(f64::NAN) && !valid_position(-1.0));
    }
}
