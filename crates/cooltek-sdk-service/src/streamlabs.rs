//! Streamlabs Desktop control over its Remote Control API (JSON-RPC 2.0).
//!
//! One action is one short connection, like [`crate::obs`]: TCP connect to a
//! loopback endpoint, WebSocket upgrade, `TcpServerService.auth` with the API
//! token, the action's requests (a scene in another collection, a toggle or a
//! fader step read before they write, on the same connection), then a close
//! frame. No event subscription, nothing cached or retried.
//!
//! **The transport is unverified.** It follows a static reading of the
//! official MasterHUB app (`docs/research/masterhub-streamlabs-actions.md`),
//! which talks SockJS to `http://HOST:PORT/api`. This client instead uses the
//! raw WebSocket endpoint that SockJS servers expose next to their framed
//! transports, `/api/websocket`, and sends plain JSON-RPC text frames. Path
//! and framing live only in [`API_PATH`] and [`rpc_message`]; nothing has
//! been checked against a real Streamlabs Desktop yet.
//!
//! Security properties, as for OBS:
//!
//! - **Loopback only.** [`StreamlabsEndpoint`] accepts what
//!   [`ObsEndpoint`] accepts (`127.0.0.1:PORT`, `[::1]:PORT`,
//!   `localhost:PORT`) and the connection code is the OBS client's, which
//!   checks the connected peer, follows no redirects and uses no proxy.
//! - **The token never leaves memory.** [`StreamlabsToken`] has no
//!   `Display` or `Serialize`, its `Debug` prints `<redacted>`, and no error
//!   contains it: the auth reply's text is never shown.
//! - **Bounded input.** Every frame is at most [`MAX_FRAME_BYTES`], a whole
//!   action shares one deadline (at most [`DEFAULT_TIMEOUT`]), lists are at
//!   most [`crate::obs::MAX_LISTED_NAMES`] long and resource ids at most
//!   [`MAX_RESOURCE_BYTES`]. Server text in an error is sanitized and only
//!   shown for a request that carried no arguments.
//! - **Names are data.** Scenes, collections, items and audio sources are
//!   matched by exact name against what Streamlabs lists on the same
//!   connection; only the ids and resource ids Streamlabs returned are sent
//!   back, as `serde_json` values. Methods are `&'static str` fixed in code.

use crate::obs::{self, Connection, ObsEndpoint, Service};
use crate::profile::{Action, ObsMuteOp, StreamlabsAudioOp, StreamlabsVisibilityOp};
use serde_json::{Value, json};
use std::{
    fmt,
    io::BufRead,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

/// Upper bound on one Streamlabs action: connect, handshake, auth, every request.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);
/// Largest WebSocket frame payload accepted from Streamlabs. Larger than
/// OBS's: `ScenesService.getScenes` returns every scene with all its items.
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Largest accepted API token, in UTF-8 bytes.
pub const MAX_TOKEN_BYTES: usize = 256;
/// Largest resource id or object id accepted from Streamlabs (and sent back).
pub const MAX_RESOURCE_BYTES: usize = 512;
/// `action_unavailable` reason for a Streamlabs action in a run without a
/// Streamlabs endpoint and token.
pub const STREAMLABS_UNAVAILABLE: &str = "Streamlabs is not configured for this run (turn on Streamlabs in OpenStreamHub Settings and enter its API token, or pass --streamlabs-endpoint with --streamlabs-token-stdin); no action was taken";

// ---- Transport: the one place to change after a real-app check. ----

/// The WebSocket request path: SockJS's raw WebSocket endpoint under the
/// official app's `/api` prefix. UNVERIFIED against Streamlabs Desktop.
const API_PATH: &str = "/api/websocket";

/// One JSON-RPC 2.0 request as Streamlabs' Remote Control API expects it:
/// the method on a resource (a service name such as `StreamingService`, or a
/// `resourceId` Streamlabs returned) with positional arguments. Sent as one
/// plain WebSocket text frame (no SockJS framing). UNVERIFIED framing.
fn rpc_message(id: u64, method: &str, resource: &str, args: &[Value]) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "method":method, "params":{"resource":resource, "args":args}})
}

const SERVICE: Service = Service {
    name: "Streamlabs",
    running_hint: "is Streamlabs Desktop running with Remote Control on?",
    path: API_PATH,
    subprotocol: None,
    max_frame_bytes: MAX_FRAME_BYTES,
    timeout: DEFAULT_TIMEOUT,
    close_error,
};

/// How often `activeCollection` is read while a collection loads.
const COLLECTION_POLL: Duration = Duration::from_millis(100);

const AUTH_FAILED: &str =
    "Streamlabs rejected the API token (authentication failed); check the Remote Control token";

/// Why Streamlabs closed the connection, from the close code only.
fn close_error(payload: &[u8]) -> String {
    match payload {
        [high, low, ..] => format!(
            "Streamlabs closed the connection (code {})",
            u16::from_be_bytes([*high, *low])
        ),
        _ => "Streamlabs closed the connection".into(),
    }
}

/// A loopback Streamlabs Remote Control endpoint, with the same rules as
/// [`ObsEndpoint`]. The default is `127.0.0.1:59650`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StreamlabsEndpoint(ObsEndpoint);

impl StreamlabsEndpoint {
    /// Streamlabs Desktop's default Remote Control port.
    pub const DEFAULT_PORT: u16 = 59650;

    /// Parses exactly `127.0.0.1:PORT`, `[::1]:PORT` or `localhost:PORT`;
    /// anything else is refused (see [`ObsEndpoint::parse`]).
    pub fn parse(value: &str) -> Result<Self, String> {
        ObsEndpoint::parse(value).map(Self).map_err(|_| {
            "Streamlabs endpoint must be 127.0.0.1:PORT, [::1]:PORT or localhost:PORT with a port from 1 to 65535".into()
        })
    }

    /// Builds an endpoint from a settings host (`127.0.0.1`, `::1` or
    /// `localhost`) and port.
    pub fn new(host: &str, port: u16) -> Result<Self, String> {
        if port == 0 {
            return Err("Streamlabs port must be between 1 and 65535".into());
        }
        ObsEndpoint::new(host, port)
            .map(Self)
            .map_err(|_| "Streamlabs host must be 127.0.0.1, ::1 or localhost".into())
    }

    /// The settings form of the host: `127.0.0.1`, `::1` or `localhost`.
    pub fn host(&self) -> &'static str {
        self.0.host()
    }

    /// The TCP port.
    pub fn port(&self) -> u16 {
        self.0.port()
    }
}

impl Default for StreamlabsEndpoint {
    fn default() -> Self {
        Self(ObsEndpoint::new("127.0.0.1", Self::DEFAULT_PORT).expect("a valid default"))
    }
}

impl fmt::Display for StreamlabsEndpoint {
    /// The canonical form [`StreamlabsEndpoint::parse`] accepts.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The Streamlabs Remote Control API token, held only in memory.
///
/// It has no `Display` or `Serialize`; `Debug` prints `<redacted>`. Like
/// [`crate::obs::ObsPassword`] it is not zeroed on drop.
#[derive(Clone, Eq, PartialEq)]
pub struct StreamlabsToken(String);

impl StreamlabsToken {
    /// Accepts a nonempty token of at most [`MAX_TOKEN_BYTES`] bytes without
    /// control characters. Errors never contain the token.
    pub fn new(token: String) -> Result<Self, String> {
        if token.is_empty() || token.len() > MAX_TOKEN_BYTES {
            return Err(format!(
                "Streamlabs token must be nonempty and at most {MAX_TOKEN_BYTES} bytes"
            ));
        }
        if token.chars().any(char::is_control) {
            return Err("Streamlabs token must not contain control characters".into());
        }
        Ok(Self(token))
    }

    /// Reads the token from the next line of `reader` (the
    /// `--streamlabs-token-stdin` channel, after the OBS password line when
    /// both are given): at most [`MAX_TOKEN_BYTES`] bytes of UTF-8 followed
    /// by an optional `\n` (or `\r\n`) or end of input.
    pub fn read_line(reader: impl BufRead) -> Result<Self, String> {
        let mut bytes = Vec::new();
        reader
            .take(MAX_TOKEN_BYTES as u64 + 3)
            .read_until(b'\n', &mut bytes)
            .map_err(|error| format!("could not read the Streamlabs token: {}", error.kind()))?;
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
        } else if bytes.len() > MAX_TOKEN_BYTES {
            return Err(format!(
                "Streamlabs token must be nonempty and at most {MAX_TOKEN_BYTES} bytes"
            ));
        }
        let token = String::from_utf8(bytes)
            .map_err(|_| "Streamlabs token must be valid UTF-8".to_string())?;
        Self::new(token)
    }

    /// The exact bytes [`StreamlabsToken::read_line`] accepts: the token and
    /// a newline. The only way to read the token outside this module, for a
    /// supervisor that writes it to a child's stdin pipe.
    pub fn stdin_line(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.0.len() + 1);
        bytes.extend_from_slice(self.0.as_bytes());
        bytes.push(b'\n');
        bytes
    }
}

impl fmt::Debug for StreamlabsToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StreamlabsToken(<redacted>)")
    }
}

/// Talks to Streamlabs Desktop over loopback, one connection per action.
#[derive(Clone, Debug)]
pub struct StreamlabsClient {
    endpoint: StreamlabsEndpoint,
    token: StreamlabsToken,
    timeout: Duration,
    cancellation: Option<Arc<AtomicBool>>,
}

impl StreamlabsClient {
    /// A client for `endpoint`. Constructing it performs no I/O.
    pub fn new(endpoint: StreamlabsEndpoint, token: StreamlabsToken) -> Self {
        Self {
            endpoint,
            token,
            timeout: DEFAULT_TIMEOUT,
            cancellation: None,
        }
    }

    /// Shortens the per-action deadline; it never exceeds [`DEFAULT_TIMEOUT`].
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout.min(DEFAULT_TIMEOUT);
        self
    }

    /// Stops before connecting, and while waiting on Streamlabs, once `flag` is set.
    pub fn with_cancellation(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancellation = Some(flag);
        self
    }

    /// The configured endpoint.
    pub fn endpoint(&self) -> StreamlabsEndpoint {
        self.endpoint
    }

    /// Runs a Streamlabs action on one connection and deadline. An action
    /// that does not talk to Streamlabs fails without connecting. No retry;
    /// after an error an earlier request may already have taken effect.
    pub fn run(&self, action: &Action) -> Result<(), String> {
        if !action.uses_streamlabs()
            || matches!(
                action,
                Action::MultiAction { .. } | Action::Conditional { .. }
            )
        {
            return Err("this action does not talk to Streamlabs; no action was taken".into());
        }
        self.session(|session| run_action(session, action))
    }

    fn session<T>(
        &self,
        script: impl FnOnce(&mut Session<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let deadline = Instant::now() + self.timeout;
        let mut connection = Connection::open(
            SERVICE,
            self.endpoint.0,
            deadline,
            self.cancellation.clone(),
        )
        .map_err(|error| self.error(error))?;
        let mut session = Session {
            connection: &mut connection,
            next_id: 1,
        };
        let result = session
            .authenticate(&self.endpoint, &self.token)
            .and_then(|()| script(&mut session));
        connection.close();
        result.map_err(|error| self.error(error))
    }

    fn error(&self, detail: String) -> String {
        format!("Streamlabs at {}: {detail}", self.endpoint)
    }

    /// Opens one connection, authenticates, reads `StreamingService.getModel`
    /// (a read that changes nothing) and closes, to say why Streamlabs can or
    /// cannot be used. Neither the token nor text Streamlabs sends is ever
    /// part of the result.
    pub fn probe(&self) -> StreamlabsStatus {
        let deadline = Instant::now() + self.timeout;
        let Ok(mut connection) = Connection::open(
            SERVICE,
            self.endpoint.0,
            deadline,
            self.cancellation.clone(),
        ) else {
            return StreamlabsStatus::Unreachable;
        };
        let status = self.probe_connection(&mut connection);
        connection.close();
        status
    }

    fn probe_connection(&self, connection: &mut Connection) -> StreamlabsStatus {
        if connection.handshake(&self.endpoint.to_string()).is_err() {
            return StreamlabsStatus::NotStreamlabs;
        }
        let mut session = Session {
            connection,
            next_id: 1,
        };
        match session.auth(&self.token) {
            Ok(()) => {}
            Err(error) if error == AUTH_FAILED => return StreamlabsStatus::TokenRefused,
            Err(error) if error.contains("not JSON-RPC") => {
                return StreamlabsStatus::NotStreamlabs;
            }
            Err(_) => return StreamlabsStatus::Failed,
        }
        match session.call("StreamingService", "getModel", &[]) {
            Ok(Value::Object(_)) => StreamlabsStatus::Connected,
            _ => StreamlabsStatus::Failed,
        }
    }
}

/// What [`StreamlabsClient::probe`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamlabsStatus {
    /// Authentication and a harmless read succeeded.
    Connected,
    /// Nothing accepted the connection (Streamlabs Desktop closed, Remote
    /// Control off, or another port).
    Unreachable,
    /// Something accepted the connection but did not upgrade to a WebSocket
    /// or did not answer JSON-RPC.
    NotStreamlabs,
    /// Streamlabs rejected the API token.
    TokenRefused,
    /// Streamlabs answered, but authentication or the read failed in some
    /// other way (including a timeout).
    Failed,
}

/// An authenticated connection inside [`StreamlabsClient::session`].
struct Session<'a> {
    connection: &'a mut Connection,
    next_id: u64,
}

impl Session<'_> {
    fn authenticate(
        &mut self,
        endpoint: &StreamlabsEndpoint,
        token: &StreamlabsToken,
    ) -> Result<(), String> {
        self.connection.handshake(&endpoint.to_string())?;
        self.auth(token)
    }

    fn auth(&mut self, token: &StreamlabsToken) -> Result<(), String> {
        // The reply's error text is never shown: it could repeat the token.
        match self.call("TcpServerService", "auth", &[json!(token.0)]) {
            Ok(Value::Bool(true)) => Ok(()),
            Ok(_) => Err(AUTH_FAILED.into()),
            Err(error) if error.starts_with("Streamlabs refused") => Err(AUTH_FAILED.into()),
            Err(error) => Err(error),
        }
    }

    /// Calls `method` on `resource` and returns its `result`. Messages
    /// without an id (events, promise results) are skipped. A method that
    /// returns a promise answers with a `SUBSCRIPTION` object, returned as is.
    fn call(
        &mut self,
        resource: &str,
        method: &'static str,
        args: &[Value],
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        self.connection
            .send_message(&rpc_message(id, method, resource, args))?;
        for _ in 0..=obs::MAX_SKIPPED_MESSAGES {
            let text = self.connection.read_text()?;
            let message: Value = serde_json::from_str(&text)
                .map_err(|_| "Streamlabs sent a message that is not JSON-RPC".to_string())?;
            match message.get("id") {
                None | Some(Value::Null) => continue,
                Some(answer) if answer.as_u64() == Some(id) => {
                    return check_response(&message, method, args.is_empty());
                }
                Some(_) => return Err("Streamlabs answered a different request".into()),
            }
        }
        Err("Streamlabs sent too many messages before the response".into())
    }
}

fn check_response(message: &Value, method: &str, show_text: bool) -> Result<Value, String> {
    if let Some(error) = message.get("error") {
        let code = error
            .get("code")
            .and_then(Value::as_i64)
            .map_or_else(|| "no code".to_string(), |code| format!("code {code}"));
        // A request with arguments may carry an id Streamlabs repeats, and
        // the auth request carries the token: its text is never shown.
        let text = match error.get("message").and_then(Value::as_str) {
            Some(text) if show_text => format!(": {}", obs::sanitize(text)),
            _ => String::new(),
        };
        return Err(format!("Streamlabs refused {method} ({code}){text}"));
    }
    message
        .get("result")
        .cloned()
        .ok_or_else(|| "Streamlabs response has neither result nor error".into())
}

/// A string id field of a Streamlabs object, bounded and without control
/// characters, to be sent back as a resource or argument.
fn id_field(object: &Value, field: &str) -> Result<String, String> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|id| {
            !id.is_empty() && id.len() <= MAX_RESOURCE_BYTES && !id.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .ok_or_else(|| format!("Streamlabs response has no usable {field}"))
}

/// The entries of a list reply, bounded.
fn listed<'v>(value: &'v Value, what: &str) -> Result<&'v [Value], String> {
    let items = value
        .as_array()
        .ok_or_else(|| format!("Streamlabs response is not a list of {what}"))?;
    if items.len() > obs::MAX_LISTED_NAMES {
        return Err(format!(
            "Streamlabs lists more than {} {what}",
            obs::MAX_LISTED_NAMES
        ));
    }
    Ok(items)
}

/// The one entry named exactly `name`. None or several is an error that
/// names `what`, never the name.
fn named<'v>(items: &'v [Value], name: &str, what: &str) -> Result<&'v Value, String> {
    let mut found = items
        .iter()
        .filter(|item| item.get("name").and_then(Value::as_str) == Some(name));
    match (found.next(), found.next()) {
        (Some(item), None) => Ok(item),
        (None, _) => Err(format!("Streamlabs has no {what} with that name")),
        (Some(_), Some(_)) => Err(format!(
            "Streamlabs has more than one {what} with that name"
        )),
    }
}

fn flag(object: &Value, field: &str) -> Result<bool, String> {
    object
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("Streamlabs response has no {field}"))
}

/// The fader position after a step of `step` percent, held between 0 and 1.
/// (The official app does not clamp.)
fn adjusted_deflection(current: f64, step: i8) -> f64 {
    (current + f64::from(step) / 100.0).clamp(0.0, 1.0)
}

fn run_action(session: &mut Session<'_>, action: &Action) -> Result<(), String> {
    match action {
        Action::StreamlabsRecord => session
            .call("StreamingService", "toggleRecording", &[])
            .map(drop),
        Action::StreamlabsStream { .. } => session
            .call("StreamingService", "toggleStreaming", &[])
            .map(drop),
        Action::StreamlabsStudioMode => {
            let model = session.call("TransitionsService", "getModel", &[])?;
            let method = if flag(&model, "studioMode")? {
                "disableStudioMode"
            } else {
                "enableStudioMode"
            };
            session.call("TransitionsService", method, &[]).map(drop)
        }
        Action::StreamlabsTransition => session
            .call("TransitionsService", "executeStudioModeTransition", &[])
            .map(drop),
        Action::StreamlabsScene { scene, collection } => {
            if let Some(collection) = collection {
                ensure_collection(session, collection)?;
            }
            let scenes = session.call("ScenesService", "getScenes", &[])?;
            let id = id_field(named(listed(&scenes, "scenes")?, scene, "scene")?, "id")?;
            session
                .call("ScenesService", "makeSceneActive", &[json!(id)])
                .map(drop)
        }
        Action::StreamlabsSourceVisibility { scene, source, op } => {
            let scenes = session.call("ScenesService", "getScenes", &[])?;
            let scene = named(listed(&scenes, "scenes")?, scene, "scene")?;
            let nodes = listed(scene.get("nodes").unwrap_or(&Value::Null), "scene items")?;
            let node = named(nodes, source, "item in that scene")?;
            let items = target_items(nodes, node)?;
            let visible = match op {
                StreamlabsVisibilityOp::Show => true,
                StreamlabsVisibilityOp::Hide => false,
                // A folder shows everything when anything is hidden.
                StreamlabsVisibilityOp::Toggle => {
                    let mut any_hidden = false;
                    for item in &items {
                        any_hidden |= !flag(item, "visible")?;
                    }
                    any_hidden
                }
            };
            for item in items {
                let resource = id_field(item, "resourceId")?;
                session.call(&resource, "setVisibility", &[json!(visible)])?;
            }
            Ok(())
        }
        Action::StreamlabsAudio { source, op } => {
            let sources = session.call("AudioService", "getSources", &[])?;
            let source = named(listed(&sources, "audio sources")?, source, "audio source")?;
            let resource = id_field(source, "resourceId")?;
            match op {
                StreamlabsAudioOp::Set(percent) => session.call(
                    &resource,
                    "setDeflection",
                    &[json!(f64::from(*percent) / 100.0)],
                ),
                StreamlabsAudioOp::Adjust(step) => {
                    let model = session.call(&resource, "getModel", &[])?;
                    let current = model
                        .pointer("/fader/deflection")
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite())
                        .ok_or("Streamlabs response has no fader deflection")?;
                    session.call(
                        &resource,
                        "setDeflection",
                        &[json!(adjusted_deflection(current, *step))],
                    )
                }
                StreamlabsAudioOp::Mute(mode) => {
                    let muted = match mode {
                        ObsMuteOp::Mute => true,
                        ObsMuteOp::Unmute => false,
                        ObsMuteOp::Toggle => {
                            !flag(&session.call(&resource, "getModel", &[])?, "muted")?
                        }
                    };
                    session.call(&resource, "setMuted", &[json!(muted)])
                }
            }
            .map(drop)
        }
        _ => Err("this action does not talk to Streamlabs; no action was taken".into()),
    }
}

/// The scene items a visibility action changes: the item itself, or every
/// item inside a folder (nested folders included, by `parentId`).
fn target_items<'v>(nodes: &'v [Value], node: &'v Value) -> Result<Vec<&'v Value>, String> {
    if node.get("sceneNodeType").and_then(Value::as_str) != Some("folder") {
        return Ok(vec![node]);
    }
    let mut folders = vec![id_field(node, "id")?];
    let mut items = Vec::new();
    // Each pass adds the children of the folders found so far; a node list
    // of n entries settles within n passes.
    for _ in 0..nodes.len() {
        let before = folders.len() + items.len();
        for candidate in nodes {
            let parent = candidate.get("parentId").and_then(Value::as_str);
            if !parent.is_some_and(|parent| folders.iter().any(|folder| folder == parent)) {
                continue;
            }
            let id = id_field(candidate, "id")?;
            if candidate.get("sceneNodeType").and_then(Value::as_str) == Some("folder") {
                if !folders.contains(&id) {
                    folders.push(id);
                }
            } else if !items
                .iter()
                .any(|item: &&Value| item.get("id") == candidate.get("id"))
            {
                items.push(candidate);
            }
        }
        if folders.len() + items.len() == before {
            break;
        }
    }
    if items.is_empty() {
        return Err("the Streamlabs folder holds no items; no action was taken".into());
    }
    Ok(items)
}

/// Makes `name` the active scene collection: nothing when it already is,
/// otherwise `load`, then reads `activeCollection` until it is the loaded one
/// (within the action deadline; no fixed wait).
fn ensure_collection(session: &mut Session<'_>, name: &str) -> Result<(), String> {
    let active = session.call("SceneCollectionsService", "activeCollection", &[])?;
    if active.get("name").and_then(Value::as_str) == Some(name) {
        return Ok(());
    }
    let collections = session.call("SceneCollectionsService", "collections", &[])?;
    let id = id_field(
        named(
            listed(&collections, "scene collections")?,
            name,
            "scene collection",
        )?,
        "id",
    )?;
    session.call("SceneCollectionsService", "load", &[json!(id)])?;
    loop {
        session
            .connection
            .pause(COLLECTION_POLL)
            .map_err(|error| format!("the scene collection did not finish loading: {error}"))?;
        let active = session
            .call("SceneCollectionsService", "activeCollection", &[])
            .map_err(|error| format!("the scene collection did not finish loading: {error}"))?;
        if active.get("id").and_then(Value::as_str) == Some(id.as_str()) {
            return Ok(());
        }
    }
}

/// What Streamlabs currently has, for the app's editor name fields: the scenes
/// of the active scene collection, every scene collection, the active one and
/// the audio sources. One reply of [`StreamlabsClient::catalog`]. Each list
/// keeps Streamlabs' own order and holds each name once, and only names a
/// profile can save (the OBS name rules) are listed. A list beyond
/// [`obs::MAX_LISTED_NAMES`] entries is refused, not truncated, as for OBS.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct StreamlabsCatalog {
    /// Scene names of the active scene collection.
    pub scenes: Vec<String>,
    /// Scene collection names.
    pub scene_collections: Vec<String>,
    /// The active scene collection, when Streamlabs names one a profile can
    /// save. The scenes above belong to it.
    pub active_collection: Option<String>,
    /// Audio source names (the mixer).
    pub audio_sources: Vec<String>,
}

/// Whether a profile could reference `name` (nonempty, at most 128 bytes, no
/// control or invisible characters). Other names are left out of listings.
fn savable(name: &str) -> bool {
    crate::profile::validate_obs_name("", "", name).is_ok()
}

/// The `name` of every entry of a list reply, bounded, in order, each once.
/// An entry without a string `name` is a malformed reply, never skipped.
fn listed_names(value: &Value, what: &str) -> Result<Vec<String>, String> {
    let mut seen = std::collections::HashSet::new();
    let mut names = Vec::new();
    for item in listed(value, what)? {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Streamlabs response has an entry without a name in {what}"))?;
        if savable(name) && seen.insert(name) {
            names.push(name.to_owned());
        }
    }
    Ok(names)
}

impl StreamlabsClient {
    /// Asks Streamlabs for everything the editor name fields offer: one
    /// connection, the read requests the actions themselves send
    /// (`ScenesService.getScenes`, `SceneCollectionsService.collections` and
    /// `activeCollection`, `AudioService.getSources`), one deadline. Nothing
    /// is changed.
    pub fn catalog(&self) -> Result<StreamlabsCatalog, String> {
        self.session(|session| {
            let scenes = listed_names(&session.call("ScenesService", "getScenes", &[])?, "scenes")?;
            let scene_collections = listed_names(
                &session.call("SceneCollectionsService", "collections", &[])?,
                "scene collections",
            )?;
            let active_collection = session
                .call("SceneCollectionsService", "activeCollection", &[])?
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| savable(name))
                .map(str::to_owned);
            let audio_sources = listed_names(
                &session.call("AudioService", "getSources", &[])?,
                "audio sources",
            )?;
            Ok(StreamlabsCatalog {
                scenes,
                scene_collections,
                active_collection,
                audio_sources,
            })
        })
    }

    /// The names of the items of one scene of the active collection, folders
    /// and the items inside them included, in Streamlabs' own order: what a
    /// source visibility in that scene can target. The scene is matched
    /// exactly, as when the action runs; an error never names it.
    pub fn scene_sources(&self, scene: &str) -> Result<Vec<String>, String> {
        self.session(|session| {
            let scenes = session.call("ScenesService", "getScenes", &[])?;
            let scene = named(listed(&scenes, "scenes")?, scene, "scene")?;
            listed_names(scene.get("nodes").unwrap_or(&Value::Null), "scene items")
        })
    }
}

/// A scripted fake Streamlabs server for this module's and other modules' tests.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use std::{
        io::{Read, Write},
        net::{TcpListener, TcpStream},
        thread::{self, JoinHandle},
    };

    /// One reply of the fake server to the next request.
    pub(crate) enum Reply {
        /// `{"result": value}` with the request's id.
        Result(Value),
        /// `{"error": {"code", "message"}}` with the request's id.
        Error(i64, &'static str),
        /// These bytes, then read nothing more.
        Raw(Vec<u8>),
        /// Never answer.
        Silent,
    }

    /// What the fake server saw: the upgrade request head and every request.
    #[derive(Debug, Default)]
    pub(crate) struct Seen {
        pub(crate) head: String,
        pub(crate) requests: Vec<Value>,
    }

    impl Seen {
        /// `(resource, method, args)` of every request after auth.
        pub(crate) fn calls(&self) -> Vec<(String, String, Value)> {
            self.requests
                .iter()
                .skip(1)
                .map(|request| {
                    (
                        request["params"]["resource"].as_str().unwrap().to_owned(),
                        request["method"].as_str().unwrap().to_owned(),
                        request["params"]["args"].clone(),
                    )
                })
                .collect()
        }
    }

    pub(crate) fn frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0x81];
        match payload.len() {
            length @ 0..=125 => frame.push(length as u8),
            length @ 126..=65535 => {
                frame.push(126);
                frame.extend_from_slice(&(length as u16).to_be_bytes());
            }
            length => {
                frame.push(127);
                frame.extend_from_slice(&(length as u64).to_be_bytes());
            }
        }
        frame.extend_from_slice(payload);
        frame
    }

    fn read_head(stream: &mut TcpStream) -> String {
        let mut head = Vec::new();
        let mut byte = [0];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
        }
        String::from_utf8(head).unwrap()
    }

    /// The next client text frame as JSON, or `None` for anything else.
    fn read_request(stream: &mut TcpStream) -> Option<Value> {
        let mut first = [0];
        stream.read_exact(&mut first).ok()?;
        if first[0] != 0x81 {
            return None;
        }
        let mut byte = [0];
        stream.read_exact(&mut byte).ok()?;
        let length = match byte[0] & 0x7F {
            126 => {
                let mut bytes = [0; 2];
                stream.read_exact(&mut bytes).ok()?;
                usize::from(u16::from_be_bytes(bytes))
            }
            short => usize::from(short),
        };
        let mut mask = [0; 4];
        stream.read_exact(&mut mask).ok()?;
        let mut payload = vec![0; length];
        stream.read_exact(&mut payload).ok()?;
        payload
            .iter_mut()
            .enumerate()
            .for_each(|(index, byte)| *byte ^= mask[index % 4]);
        serde_json::from_slice(&payload).ok()
    }

    /// Serves one connection: accepts the upgrade, then answers auth with
    /// `true` and each later request with the next reply.
    pub(crate) fn serve(replies: Vec<Reply>) -> (StreamlabsEndpoint, JoinHandle<Seen>) {
        let mut all = vec![Reply::Result(json!(true))];
        all.extend(replies);
        serve_raw(all)
    }

    /// Like [`serve`], but the auth request gets the first reply too.
    pub(crate) fn serve_raw(replies: Vec<Reply>) -> (StreamlabsEndpoint, JoinHandle<Seen>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let join = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut seen = Seen {
                head: read_head(&mut stream),
                ..Seen::default()
            };
            let key = seen
                .head
                .lines()
                .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
                .unwrap()
                .to_owned();
            let response = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                obs::tests_support::accept(&key)
            );
            stream.write_all(response.as_bytes()).unwrap();
            for reply in replies {
                // The client may stop early (its close frame, or EOF).
                let Some(request) = read_request(&mut stream) else {
                    break;
                };
                let id = request["id"].clone();
                seen.requests.push(request);
                let bytes = match reply {
                    Reply::Result(result) => frame(
                        json!({"jsonrpc":"2.0","id":id,"result":result})
                            .to_string()
                            .as_bytes(),
                    ),
                    Reply::Error(code, message) => frame(
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
                            .to_string()
                            .as_bytes(),
                    ),
                    Reply::Raw(bytes) => {
                        let _ = stream.write_all(&bytes);
                        break;
                    }
                    Reply::Silent => {
                        thread::sleep(Duration::from_millis(900));
                        break;
                    }
                };
                stream.write_all(&bytes).unwrap();
            }
            seen
        });
        (StreamlabsEndpoint::new("127.0.0.1", port).unwrap(), join)
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::{Reply, Seen, frame, serve, serve_raw};
    use super::*;
    use std::sync::atomic::Ordering;

    const TOKEN: &str = "s3cret-token-0123456789abcdef";
    const FAST: Duration = Duration::from_millis(600);

    fn client(endpoint: StreamlabsEndpoint) -> StreamlabsClient {
        StreamlabsClient::new(endpoint, StreamlabsToken::new(TOKEN.into()).unwrap())
            .with_timeout(FAST)
    }

    fn run(action: Action, replies: Vec<Reply>) -> (Result<(), String>, Seen) {
        let (endpoint, server) = serve(replies);
        let result = client(endpoint).run(&action);
        (result, server.join().unwrap())
    }

    fn call(resource: &str, method: &str, args: Value) -> (String, String, Value) {
        (resource.into(), method.into(), args)
    }

    fn ok(value: Value) -> Reply {
        Reply::Result(value)
    }

    fn scenes() -> Value {
        json!([
            {"id":"scene-a","name":"Live","resourceId":"Scene[\"scene-a\"]","nodes":[
                {"id":"i1","name":"Camera","sceneNodeType":"item","visible":true,"resourceId":"SceneItem[\"a\",\"i1\"]"},
                {"id":"f1","name":"Overlays","sceneNodeType":"folder","resourceId":"SceneItemFolder[\"a\",\"f1\"]"},
                {"id":"i2","name":"Logo","sceneNodeType":"item","parentId":"f1","visible":true,"resourceId":"SceneItem[\"a\",\"i2\"]"},
                {"id":"f2","name":"Inner","sceneNodeType":"folder","parentId":"f1","resourceId":"SceneItemFolder[\"a\",\"f2\"]"},
                {"id":"i3","name":"Ticker","sceneNodeType":"item","parentId":"f2","visible":false,"resourceId":"SceneItem[\"a\",\"i3\"]"},
                {"id":"f3","name":"Empty","sceneNodeType":"folder","resourceId":"SceneItemFolder[\"a\",\"f3\"]"}
            ]},
            {"id":"scene-b","name":"Be right back","resourceId":"Scene[\"scene-b\"]","nodes":[]}
        ])
    }

    fn audio() -> Value {
        json!([
            {"sourceId":"mic","name":"Mic","resourceId":"AudioSource[\"mic\"]"},
            {"sourceId":"desk","name":"Desktop Audio","resourceId":"AudioSource[\"desk\"]"}
        ])
    }

    fn assert_no_secret(text: &str) {
        assert!(!text.contains(TOKEN), "token leaked: {text}");
        assert!(!text.contains("s3cret"), "token leaked: {text}");
    }

    #[test]
    fn auth_is_sent_first_on_the_raw_websocket_path() {
        let (result, seen) = run(Action::StreamlabsRecord, vec![ok(Value::Null)]);
        result.unwrap();
        assert!(
            seen.head.starts_with("GET /api/websocket HTTP/1.1\r\n"),
            "{}",
            seen.head
        );
        assert!(
            !seen
                .head
                .to_ascii_lowercase()
                .contains("sec-websocket-protocol")
        );
        assert_eq!(
            seen.requests[0],
            json!({"jsonrpc":"2.0","id":1,"method":"auth","params":{"resource":"TcpServerService","args":[TOKEN]}})
        );
        assert_eq!(seen.requests[1]["id"], 2);
        assert_eq!(
            seen.calls(),
            [call("StreamingService", "toggleRecording", json!([]))]
        );
    }

    #[test]
    fn a_rejected_token_never_appears_in_errors_or_debug() {
        for reply in [
            Reply::Error(-32600, "Invalid token s3cret-token-0123456789abcdef"),
            Reply::Result(json!(false)),
        ] {
            let (endpoint, server) = serve_raw(vec![reply]);
            let client = client(endpoint);
            let error = client.run(&Action::StreamlabsRecord).unwrap_err();
            let seen = server.join().unwrap();
            assert!(error.contains("rejected the API token"), "{error}");
            assert_no_secret(&error);
            assert_no_secret(&format!("{client:?}"));
            // Nothing after a failed auth.
            assert_eq!(seen.requests.len(), 1);
        }
        let token = StreamlabsToken::new(TOKEN.into()).unwrap();
        assert_eq!(format!("{token:?}"), "StreamlabsToken(<redacted>)");
    }

    #[test]
    fn tokens_are_bounded() {
        assert!(StreamlabsToken::new(String::new()).is_err());
        assert!(StreamlabsToken::new("x".repeat(MAX_TOKEN_BYTES)).is_ok());
        let error = StreamlabsToken::new("x".repeat(MAX_TOKEN_BYTES + 1)).unwrap_err();
        assert!(!error.contains("xxx"), "{error}");
        assert!(StreamlabsToken::new("a\nb".into()).is_err());
    }

    #[test]
    fn toggles_and_the_transition_are_one_request_each() {
        for (action, method, service) in [
            (
                Action::StreamlabsRecord,
                "toggleRecording",
                "StreamingService",
            ),
            (
                Action::StreamlabsStream { confirm: None },
                "toggleStreaming",
                "StreamingService",
            ),
            (
                Action::StreamlabsTransition,
                "executeStudioModeTransition",
                "TransitionsService",
            ),
        ] {
            let (result, seen) = run(action, vec![ok(Value::Null)]);
            result.unwrap();
            assert_eq!(seen.calls(), [call(service, method, json!([]))]);
        }
    }

    #[test]
    fn studio_mode_reads_the_model_then_writes_the_opposite() {
        for (on, method) in [(false, "enableStudioMode"), (true, "disableStudioMode")] {
            let (result, seen) = run(
                Action::StreamlabsStudioMode,
                vec![ok(json!({"studioMode":on})), ok(Value::Null)],
            );
            result.unwrap();
            assert_eq!(
                seen.calls(),
                [
                    call("TransitionsService", "getModel", json!([])),
                    call("TransitionsService", method, json!([])),
                ]
            );
        }
        // No answer about studio mode: an error, never a guess.
        let (result, seen) = run(Action::StreamlabsStudioMode, vec![ok(json!({}))]);
        assert!(result.unwrap_err().contains("no studioMode"));
        assert_eq!(seen.calls().len(), 1);
    }

    #[test]
    fn a_scene_in_the_active_collection_is_resolved_by_name() {
        let (result, seen) = run(
            Action::StreamlabsScene {
                scene: "Be right back".into(),
                collection: None,
            },
            vec![ok(scenes()), ok(json!(true))],
        );
        result.unwrap();
        assert_eq!(
            seen.calls(),
            [
                call("ScenesService", "getScenes", json!([])),
                call("ScenesService", "makeSceneActive", json!(["scene-b"])),
            ]
        );
        let (result, seen) = run(
            Action::StreamlabsScene {
                scene: "Secret scene".into(),
                collection: None,
            },
            vec![ok(scenes())],
        );
        let error = result.unwrap_err();
        assert!(error.contains("no scene with that name"), "{error}");
        assert!(!error.contains("Secret"), "{error}");
        assert_eq!(seen.calls().len(), 1);
    }

    #[test]
    fn a_scene_in_another_collection_loads_it_then_polls_until_it_is_active() {
        let a = json!({"id":"col-a","name":"Main"});
        let b = json!({"id":"col-b","name":"Podcast"});
        let (result, seen) = run(
            Action::StreamlabsScene {
                scene: "Live".into(),
                collection: Some("Podcast".into()),
            },
            vec![
                ok(a.clone()),
                ok(json!([a.clone(), b.clone()])),
                // A promise answers with a subscription; the poll decides.
                ok(json!({"_type":"SUBSCRIPTION","resourceId":"p1","emitter":"PROMISE"})),
                ok(a.clone()),
                ok(Value::Null),
                ok(b),
                ok(scenes()),
                ok(json!(true)),
            ],
        );
        result.unwrap();
        let active = call("SceneCollectionsService", "activeCollection", json!([]));
        assert_eq!(
            seen.calls(),
            [
                active.clone(),
                call("SceneCollectionsService", "collections", json!([])),
                call("SceneCollectionsService", "load", json!(["col-b"])),
                active.clone(),
                active.clone(),
                active,
                call("ScenesService", "getScenes", json!([])),
                call("ScenesService", "makeSceneActive", json!(["scene-a"])),
            ]
        );
    }

    #[test]
    fn an_already_active_collection_is_not_reloaded() {
        let (result, seen) = run(
            Action::StreamlabsScene {
                scene: "Live".into(),
                collection: Some("Main".into()),
            },
            vec![
                ok(json!({"id":"col-a","name":"Main"})),
                ok(scenes()),
                ok(json!(true)),
            ],
        );
        result.unwrap();
        assert_eq!(seen.calls().len(), 3);
        assert_eq!(seen.calls()[1].1, "getScenes");
    }

    #[test]
    fn a_collection_that_never_finishes_loading_times_out_before_the_switch() {
        let a = json!({"id":"col-a","name":"Main"});
        let mut replies = vec![
            ok(a.clone()),
            ok(json!([a.clone(), {"id":"col-b","name":"Podcast"}])),
            ok(Value::Null),
        ];
        replies.extend((0..40).map(|_| ok(a.clone())));
        let (endpoint, server) = serve(replies);
        let started = Instant::now();
        let error = client(endpoint)
            .run(&Action::StreamlabsScene {
                scene: "Live".into(),
                collection: Some("Podcast".into()),
            })
            .unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert!(error.contains("did not finish loading"), "{error}");
        assert!(error.contains("timed out"), "{error}");
        drop(server);
    }

    #[test]
    fn an_unknown_collection_stops_before_loading() {
        let (result, seen) = run(
            Action::StreamlabsScene {
                scene: "Live".into(),
                collection: Some("Nope".into()),
            },
            vec![ok(json!({"id":"col-a","name":"Main"})), ok(json!([]))],
        );
        assert!(
            result
                .unwrap_err()
                .contains("no scene collection with that name")
        );
        assert_eq!(seen.calls().len(), 2);
    }

    fn visibility(source: &str, op: StreamlabsVisibilityOp) -> Action {
        Action::StreamlabsSourceVisibility {
            scene: "Live".into(),
            source: source.into(),
            op,
        }
    }

    #[test]
    fn source_visibility_sets_the_item_by_its_resource_id() {
        use StreamlabsVisibilityOp::{Hide, Show, Toggle};
        let item = "SceneItem[\"a\",\"i1\"]";
        for (op, visible) in [(Show, true), (Hide, false), (Toggle, false)] {
            let (result, seen) = run(
                visibility("Camera", op),
                vec![ok(scenes()), ok(Value::Null)],
            );
            result.unwrap();
            assert_eq!(
                seen.calls(),
                [
                    call("ScenesService", "getScenes", json!([])),
                    call(item, "setVisibility", json!([visible])),
                ]
            );
        }
    }

    #[test]
    fn a_folder_toggle_shows_every_nested_item_when_any_is_hidden() {
        let (result, seen) = run(
            visibility("Overlays", StreamlabsVisibilityOp::Toggle),
            vec![ok(scenes()), ok(Value::Null), ok(Value::Null)],
        );
        result.unwrap();
        assert_eq!(
            seen.calls()[1..],
            [
                call("SceneItem[\"a\",\"i2\"]", "setVisibility", json!([true])),
                call("SceneItem[\"a\",\"i3\"]", "setVisibility", json!([true])),
            ]
        );
        let (result, seen) = run(
            visibility("Empty", StreamlabsVisibilityOp::Hide),
            vec![ok(scenes())],
        );
        assert!(result.unwrap_err().contains("holds no items"));
        assert_eq!(seen.calls().len(), 1);
    }

    #[test]
    fn a_missing_or_ambiguous_item_stops_before_any_write() {
        let (result, _) = run(
            visibility("Private cam", StreamlabsVisibilityOp::Show),
            vec![ok(scenes())],
        );
        let error = result.unwrap_err();
        assert!(
            error.contains("no item in that scene with that name"),
            "{error}"
        );
        assert!(!error.contains("Private"), "{error}");
        let twice = json!([{"id":"s","name":"Live","nodes":[
            {"id":"1","name":"Cam","resourceId":"r1","visible":true},
            {"id":"2","name":"Cam","resourceId":"r2","visible":true}
        ]}]);
        let (result, seen) = run(
            visibility("Cam", StreamlabsVisibilityOp::Show),
            vec![ok(twice)],
        );
        assert!(result.unwrap_err().contains("more than one"));
        assert_eq!(seen.calls().len(), 1);
    }

    fn audio_action(op: StreamlabsAudioOp) -> Action {
        Action::StreamlabsAudio {
            source: "Mic".into(),
            op,
        }
    }

    #[test]
    fn audio_set_maps_percent_to_deflection() {
        let (result, seen) = run(
            audio_action(StreamlabsAudioOp::Set(40)),
            vec![ok(audio()), ok(Value::Null)],
        );
        result.unwrap();
        assert_eq!(
            seen.calls(),
            [
                call("AudioService", "getSources", json!([])),
                call("AudioSource[\"mic\"]", "setDeflection", json!([0.4])),
            ]
        );
    }

    #[test]
    fn an_audio_step_is_clamped_between_0_and_1() {
        assert_eq!(adjusted_deflection(0.5, 10), 0.6);
        assert_eq!(adjusted_deflection(0.97, 5), 1.0);
        assert_eq!(adjusted_deflection(0.05, -10), 0.0);
        for (current, step, expected) in [(0.97, 5, 1.0), (0.05, -10, 0.0)] {
            let (result, seen) = run(
                audio_action(StreamlabsAudioOp::Adjust(step)),
                vec![
                    ok(audio()),
                    ok(json!({"fader":{"deflection":current,"db":-3.0},"muted":false})),
                    ok(Value::Null),
                ],
            );
            result.unwrap();
            assert_eq!(
                seen.calls()[1..],
                [
                    call("AudioSource[\"mic\"]", "getModel", json!([])),
                    call("AudioSource[\"mic\"]", "setDeflection", json!([expected])),
                ]
            );
        }
    }

    #[test]
    fn audio_mute_toggles_from_the_model_or_sets_directly() {
        let (result, seen) = run(
            audio_action(StreamlabsAudioOp::Mute(ObsMuteOp::Toggle)),
            vec![ok(audio()), ok(json!({"muted":true})), ok(Value::Null)],
        );
        result.unwrap();
        assert_eq!(
            seen.calls()[1..],
            [
                call("AudioSource[\"mic\"]", "getModel", json!([])),
                call("AudioSource[\"mic\"]", "setMuted", json!([false])),
            ]
        );
        for (mode, muted) in [(ObsMuteOp::Mute, true), (ObsMuteOp::Unmute, false)] {
            let (result, seen) = run(
                audio_action(StreamlabsAudioOp::Mute(mode)),
                vec![ok(audio()), ok(Value::Null)],
            );
            result.unwrap();
            assert_eq!(
                seen.calls()[1..],
                [call("AudioSource[\"mic\"]", "setMuted", json!([muted]))]
            );
        }
    }

    #[test]
    fn an_unknown_audio_source_fails_without_its_name_or_a_write() {
        let (result, seen) = run(
            Action::StreamlabsAudio {
                source: "Hidden mic".into(),
                op: StreamlabsAudioOp::Set(10),
            },
            vec![ok(audio())],
        );
        let error = result.unwrap_err();
        assert!(error.contains("no audio source with that name"), "{error}");
        assert!(!error.contains("Hidden"), "{error}");
        assert_eq!(seen.calls().len(), 1);
    }

    #[test]
    fn refusals_show_the_code_and_only_text_of_argument_free_requests() {
        let (result, _) = run(
            Action::StreamlabsRecord,
            vec![Reply::Error(-32000, "not\u{7}ready")],
        );
        let error = result.unwrap_err();
        assert!(
            error.ends_with("Streamlabs refused toggleRecording (code -32000): not\u{FFFD}ready"),
            "{error}"
        );
        let (result, _) = run(
            Action::StreamlabsScene {
                scene: "Live".into(),
                collection: None,
            },
            vec![ok(scenes()), Reply::Error(-32000, "scene-a Live missing")],
        );
        let error = result.unwrap_err();
        assert!(
            error.ends_with("Streamlabs refused makeSceneActive (code -32000)"),
            "{error}"
        );
    }

    #[test]
    fn events_are_skipped_and_another_id_is_rejected() {
        let event = frame(br#"{"jsonrpc":"2.0","id":null,"result":{"_type":"EVENT"}}"#);
        let mut bytes = event.clone();
        bytes.extend(frame(br#"{"jsonrpc":"2.0","id":2,"result":null}"#));
        let (result, _) = run(Action::StreamlabsRecord, vec![Reply::Raw(bytes)]);
        result.unwrap();
        let (result, _) = run(
            Action::StreamlabsRecord,
            vec![Reply::Raw(frame(
                br#"{"jsonrpc":"2.0","id":7,"result":null}"#,
            ))],
        );
        assert!(result.unwrap_err().contains("answered a different request"));
        let (result, _) = run(Action::StreamlabsRecord, vec![Reply::Raw(frame(b"o"))]);
        assert!(result.unwrap_err().contains("not JSON-RPC"));
    }

    #[test]
    fn frames_beyond_the_limit_are_rejected_before_their_payload() {
        let mut header = vec![0x81, 127];
        header.extend_from_slice(&(MAX_FRAME_BYTES as u64 + 1).to_be_bytes());
        let (result, _) = run(Action::StreamlabsRecord, vec![Reply::Raw(header)]);
        let error = result.unwrap_err();
        assert!(
            error.contains(&format!(
                "Streamlabs sent a frame larger than {MAX_FRAME_BYTES} bytes"
            )),
            "{error}"
        );
    }

    #[test]
    fn a_silent_server_times_out_within_the_deadline() {
        let (endpoint, server) = serve(vec![Reply::Silent]);
        let started = Instant::now();
        let error = client(endpoint)
            .with_timeout(Duration::from_millis(300))
            .run(&Action::StreamlabsRecord)
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(800));
        assert!(
            error.contains("timed out waiting for Streamlabs"),
            "{error}"
        );
        server.join().unwrap();
    }

    #[test]
    fn a_chosen_subprotocol_is_refused() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut byte = [0];
            while !head.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            let key = head
                .lines()
                .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
                .unwrap();
            let response = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: x\r\n\r\n",
                obs::tests_support::accept(key)
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let error = client(StreamlabsEndpoint::new("127.0.0.1", port).unwrap())
            .run(&Action::StreamlabsRecord)
            .unwrap_err();
        assert!(
            error.contains("subprotocol that was not requested"),
            "{error}"
        );
        server.join().unwrap();
    }

    #[test]
    fn endpoints_are_loopback_only_with_the_streamlabs_default() {
        let default = StreamlabsEndpoint::default();
        assert_eq!(default.to_string(), "127.0.0.1:59650");
        assert_eq!((default.host(), default.port()), ("127.0.0.1", 59650));
        for good in ["127.0.0.1:59650", "[::1]:1", "localhost:65535"] {
            assert_eq!(StreamlabsEndpoint::parse(good).unwrap().to_string(), good);
        }
        for bad in [
            "192.168.1.10:59650",
            "127.0.0.2:59650",
            "0.0.0.0:59650",
            "example.com:59650",
            "http://127.0.0.1:59650",
            "127.0.0.1:59650/api",
            "127.0.0.1",
            "127.0.0.1:0",
            "",
        ] {
            let error = StreamlabsEndpoint::parse(bad).unwrap_err();
            assert!(error.starts_with("Streamlabs endpoint"), "{bad}: {error}");
        }
        assert!(StreamlabsEndpoint::new("10.0.0.1", 59650).is_err());
        assert!(StreamlabsEndpoint::new("127.0.0.1", 0).is_err());
        assert!(StreamlabsEndpoint::new("::1", 59650).is_ok());
    }

    #[test]
    fn non_streamlabs_actions_and_cancelled_runs_never_connect() {
        // Nothing listens on this endpoint; a connect attempt would say so.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint =
            StreamlabsEndpoint::new("127.0.0.1", closed.local_addr().unwrap().port()).unwrap();
        drop(closed);
        let client = client(endpoint);
        for action in [
            Action::ToggleMute,
            Action::ObsTransition,
            Action::MultiAction {
                actions: vec![Action::ToggleMute, Action::StreamlabsRecord],
            },
        ] {
            let error = client.run(&action).unwrap_err();
            assert!(error.contains("does not talk to Streamlabs"), "{error}");
        }
        let flag = Arc::new(AtomicBool::new(false));
        flag.store(true, Ordering::SeqCst);
        let error = client
            .with_cancellation(flag)
            .run(&Action::StreamlabsRecord)
            .unwrap_err();
        assert!(
            error.contains("cancelled before Streamlabs answered"),
            "{error}"
        );
    }

    #[test]
    fn nothing_listening_names_streamlabs() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint =
            StreamlabsEndpoint::new("127.0.0.1", closed.local_addr().unwrap().port()).unwrap();
        drop(closed);
        let error = client(endpoint).run(&Action::StreamlabsRecord).unwrap_err();
        assert!(error.contains("is Streamlabs Desktop running"), "{error}");
        assert_no_secret(&error);
    }

    #[test]
    fn the_token_is_read_from_one_stdin_line_and_written_back_the_same_way() {
        let read = |bytes: &[u8]| StreamlabsToken::read_line(std::io::BufReader::new(bytes));
        assert_eq!(
            read(format!("{TOKEN}\n").as_bytes()).unwrap(),
            StreamlabsToken::new(TOKEN.into()).unwrap()
        );
        assert!(read(format!("{TOKEN}\r\n").as_bytes()).is_ok());
        assert!(read(TOKEN.as_bytes()).is_ok(), "end of input ends the line");
        let token = StreamlabsToken::new("tok en 🎉".into()).unwrap();
        assert_eq!(read(&token.stdin_line()).unwrap(), token);
        for bad in [
            b"\n".to_vec(),
            Vec::new(),
            b"\xff\xfe\n".to_vec(),
            vec![b'x'; MAX_TOKEN_BYTES + 1],
            [vec![b'x'; MAX_TOKEN_BYTES + 1], b"\n".to_vec()].concat(),
        ] {
            let error = read(&bad).unwrap_err();
            assert!(error.contains("Streamlabs token"), "{error}");
            assert!(!error.contains("xxx"), "{error}");
        }
        // Two secrets share one stdin: each read takes exactly its own line.
        let mut stdin = std::io::BufReader::new(&b"obs-password\ns3cret-token\nrest"[..]);
        crate::obs::ObsPassword::read_line(&mut stdin).unwrap();
        assert_eq!(
            StreamlabsToken::read_line(&mut stdin).unwrap(),
            StreamlabsToken::new("s3cret-token".into()).unwrap()
        );
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut stdin, &mut rest).unwrap();
        assert_eq!(rest, "rest");
    }

    #[test]
    fn a_probe_authenticates_and_reads_only_the_streaming_model() {
        let (endpoint, server) = serve(vec![ok(json!({"streamingStatus":"offline"}))]);
        assert_eq!(client(endpoint).probe(), StreamlabsStatus::Connected);
        let seen = server.join().unwrap();
        assert_eq!(seen.requests[0]["method"], "auth");
        assert_eq!(
            seen.calls(),
            [call("StreamingService", "getModel", json!([]))]
        );

        for (reply, expected) in [
            (
                Reply::Error(-32600, "Invalid token s3cret-token-0123456789abcdef"),
                StreamlabsStatus::TokenRefused,
            ),
            (Reply::Result(json!(false)), StreamlabsStatus::TokenRefused),
            (
                Reply::Raw(frame(b"<html>not json</html>")),
                StreamlabsStatus::NotStreamlabs,
            ),
            (Reply::Silent, StreamlabsStatus::Failed),
        ] {
            let (endpoint, server) = serve_raw(vec![reply]);
            let client = client(endpoint);
            assert_eq!(client.probe(), expected);
            let seen = server.join().unwrap();
            assert!(seen.requests.len() <= 1, "nothing after a failed auth");
            assert_no_secret(&format!("{expected:?} {client:?}"));
        }
        // Authenticated, but the read is refused.
        let (endpoint, server) = serve(vec![Reply::Error(-32601, "no such method")]);
        assert_eq!(client(endpoint).probe(), StreamlabsStatus::Failed);
        server.join().unwrap();
    }

    #[test]
    fn a_probe_tells_nothing_listening_from_something_else() {
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint =
            StreamlabsEndpoint::new("127.0.0.1", closed.local_addr().unwrap().port()).unwrap();
        drop(closed);
        assert_eq!(client(endpoint).probe(), StreamlabsStatus::Unreachable);

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = [0; 1024];
            let _ = std::io::Read::read(&mut stream, &mut head);
            std::io::Write::write_all(&mut stream, b"HTTP/1.1 404 Not Found\r\n\r\n").unwrap();
        });
        let endpoint = StreamlabsEndpoint::new("127.0.0.1", port).unwrap();
        assert_eq!(client(endpoint).probe(), StreamlabsStatus::NotStreamlabs);
        server.join().unwrap();
    }

    // ---- Listings for the editor name fields ----

    fn catalog(replies: Vec<Reply>) -> (Result<StreamlabsCatalog, String>, Seen) {
        let (endpoint, server) = serve(replies);
        let result = client(endpoint).catalog();
        (result, server.join().unwrap())
    }

    fn collections() -> Value {
        json!([{"id":"col-a","name":"Main"},{"id":"col-b","name":"Podcast"}])
    }

    #[test]
    fn the_catalog_reads_four_lists_on_one_connection_and_changes_nothing() {
        let (result, seen) = catalog(vec![
            ok(scenes()),
            ok(collections()),
            ok(json!({"id":"col-a","name":"Main"})),
            ok(audio()),
        ]);
        assert_eq!(
            result.unwrap(),
            StreamlabsCatalog {
                scenes: vec!["Live".into(), "Be right back".into()],
                scene_collections: vec!["Main".into(), "Podcast".into()],
                active_collection: Some("Main".into()),
                audio_sources: vec!["Mic".into(), "Desktop Audio".into()],
            }
        );
        assert_eq!(seen.requests[0]["method"], "auth");
        assert_eq!(
            seen.calls(),
            [
                call("ScenesService", "getScenes", json!([])),
                call("SceneCollectionsService", "collections", json!([])),
                call("SceneCollectionsService", "activeCollection", json!([])),
                call("AudioService", "getSources", json!([])),
            ]
        );
        assert_eq!(
            serde_json::to_value(StreamlabsCatalog::default()).unwrap(),
            json!({"scenes":[],"scene_collections":[],"active_collection":null,"audio_sources":[]})
        );
    }

    #[test]
    fn listed_names_are_savable_and_listed_once() {
        let long = "x".repeat(Action::MAX_OBS_NAME_BYTES + 1);
        let fits = "y".repeat(Action::MAX_OBS_NAME_BYTES);
        let (result, _) = catalog(vec![
            ok(json!([
                {"id":"1","name":"Live"},
                {"id":"2","name":"Live"},
                {"id":"3","name":long},
                {"id":"4","name":"Bell\u{7}"},
                {"id":"5","name":"Zero\u{200B}width"},
                {"id":"6","name":"   "},
                {"id":"7","name":fits},
                {"id":"8","name":"Live 🎬 \"2\""}
            ])),
            ok(json!([{"id":"c","name":"Main"},{"id":"d","name":"Main"}])),
            // An active collection a profile could not name is left out.
            ok(json!({"id":"c","name":"Bad\nname"})),
            ok(json!([])),
        ]);
        let catalog = result.unwrap();
        assert_eq!(
            catalog.scenes,
            ["Live".to_string(), fits, "Live 🎬 \"2\"".to_string()]
        );
        assert_eq!(catalog.scene_collections, ["Main".to_string()]);
        assert_eq!(catalog.active_collection, None);
        assert!(catalog.audio_sources.is_empty());
        // A reply without a name keeps the field empty too: it only steers a warning.
        let (result, _) = catalog_with_active(json!(null));
        assert_eq!(result.unwrap().active_collection, None);
    }

    fn catalog_with_active(active: Value) -> (Result<StreamlabsCatalog, String>, Seen) {
        catalog(vec![
            ok(json!([])),
            ok(json!([])),
            ok(active),
            ok(json!([])),
        ])
    }

    #[test]
    fn a_malformed_or_oversized_list_fails_the_catalog_without_inventing_one() {
        let many: Vec<_> = (0..=obs::MAX_LISTED_NAMES)
            .map(|index| json!({"name":format!("S{index}")}))
            .collect();
        for (reply, expected) in [
            (json!({"scenes":[]}), "is not a list of scenes"),
            (json!([{"id":"1"}]), "entry without a name in scenes"),
            (json!([{"name":5}]), "entry without a name in scenes"),
            (json!(["Live"]), "entry without a name in scenes"),
            (json!(many), "lists more than 1000 scenes"),
        ] {
            let (result, seen) = catalog(vec![ok(reply)]);
            let error = result.unwrap_err();
            assert!(error.starts_with("Streamlabs at 127.0.0.1:"), "{error}");
            assert!(error.contains(expected), "{error}");
            assert_no_secret(&error);
            // Nothing is read after the broken list.
            assert_eq!(seen.calls().len(), 1);
        }
        let (result, seen) = catalog(vec![
            ok(scenes()),
            ok(collections()),
            ok(json!({"name":"Main"})),
            ok(json!({"sources":[]})),
        ]);
        assert!(
            result
                .unwrap_err()
                .contains("is not a list of audio sources")
        );
        assert_eq!(seen.calls().len(), 4);
    }

    #[test]
    fn a_refused_unauthenticated_or_silent_catalog_fails_without_the_token() {
        let (result, _) = catalog(vec![ok(scenes()), Reply::Error(-32601, "no such method")]);
        let error = result.unwrap_err();
        assert!(
            error.ends_with("Streamlabs refused collections (code -32601): no such method"),
            "{error}"
        );
        let (endpoint, server) = serve_raw(vec![Reply::Error(
            -32600,
            "Invalid token s3cret-token-0123456789abcdef",
        )]);
        let error = client(endpoint).catalog().unwrap_err();
        let seen = server.join().unwrap();
        assert!(error.contains("rejected the API token"), "{error}");
        assert_no_secret(&error);
        assert_eq!(seen.requests.len(), 1, "nothing after a failed auth");
        let (endpoint, server) = serve(vec![Reply::Silent]);
        let error = client(endpoint)
            .with_timeout(Duration::from_millis(300))
            .catalog()
            .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        server.join().unwrap();
    }

    fn scene_sources(scene: &str, replies: Vec<Reply>) -> (Result<Vec<String>, String>, Seen) {
        let (endpoint, server) = serve(replies);
        let result = client(endpoint).scene_sources(scene);
        (result, server.join().unwrap())
    }

    #[test]
    fn scene_sources_list_items_and_folders_of_the_named_scene() {
        let (result, seen) = scene_sources("Live", vec![ok(scenes())]);
        // Folders, the items inside them and nested folders are all targets.
        assert_eq!(
            result.unwrap(),
            ["Camera", "Overlays", "Logo", "Inner", "Ticker", "Empty"]
        );
        assert_eq!(
            seen.calls(),
            [call("ScenesService", "getScenes", json!([]))]
        );
        let (result, _) = scene_sources("Be right back", vec![ok(scenes())]);
        assert!(result.unwrap().is_empty());
        let twice = json!([{"id":"s","name":"Live","nodes":[
            {"id":"1","name":"Cam"},{"id":"2","name":"Cam"},{"id":"3","name":"x\u{2028}y"}
        ]}]);
        let (result, _) = scene_sources("Live", vec![ok(twice)]);
        assert_eq!(result.unwrap(), ["Cam"]);
    }

    #[test]
    fn scene_sources_of_an_unknown_scene_or_a_broken_reply_fail_without_the_name() {
        let (result, _) = scene_sources("Secret scene", vec![ok(scenes())]);
        let error = result.unwrap_err();
        assert!(error.contains("no scene with that name"), "{error}");
        assert!(!error.contains("Secret"), "{error}");
        let doubled = json!([{"name":"Live","nodes":[]},{"name":"Live","nodes":[]}]);
        let (result, _) = scene_sources("Live", vec![ok(doubled)]);
        assert!(result.unwrap_err().contains("more than one scene"));
        let (result, _) = scene_sources("Live", vec![ok(json!([{"name":"Live"}]))]);
        assert!(result.unwrap_err().contains("is not a list of scene items"));
        let (result, _) = scene_sources(
            "Live",
            vec![ok(json!([{"name":"Live","nodes":[{"id":"1"}]}]))],
        );
        assert!(
            result
                .unwrap_err()
                .contains("entry without a name in scene items")
        );
    }
}
