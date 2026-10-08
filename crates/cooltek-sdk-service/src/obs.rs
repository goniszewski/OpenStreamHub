//! OBS Studio control over obs-websocket v5 (built into OBS 28+).
//!
//! One action is one short connection: TCP connect to a loopback endpoint,
//! WebSocket upgrade, `Hello`, `Identify` (with challenge authentication when
//! OBS asks for it, and `eventSubscriptions` 0), one or more requests (a
//! read-then-write toggle sends two on the same connection), each answered by
//! its `RequestResponse`, then a close frame. Nothing is cached or retried.
//!
//! Security properties:
//!
//! - **Loopback only.** [`ObsEndpoint`] accepts exactly `127.0.0.1:PORT`,
//!   `[::1]:PORT` or `localhost:PORT`. `localhost` is mapped to the loopback
//!   addresses without DNS. After connecting, the peer address must equal the
//!   loopback address that was dialled. No TLS, no redirects (anything other
//!   than `101 Switching Protocols` fails), no proxy environment variables.
//!   The endpoint comes from the command line, never from a profile.
//! - **The password never leaves memory.** [`ObsPassword`] has no `Display`
//!   or `Serialize`, its `Debug` prints `<redacted>`, and no error message
//!   contains it or the derived authentication string.
//! - **Bounded input.** Handshake headers are limited to 8 KiB, every frame
//!   to [`MAX_FRAME_BYTES`], and a whole action to one deadline (at most
//!   [`DEFAULT_TIMEOUT`]) covering connect, handshake and request. Server
//!   frames must be unmasked, final (no fragmentation), use no reserved bits
//!   and carry only text, ping, pong or close. Text from OBS that ends up in an
//!   error is cleaned of control and invisible characters and cut to
//!   120 characters.
//! - **Names are data.** A scene, input or source name is sent only as a JSON
//!   string value built by `serde_json`, never formatted into a string. The
//!   request type is a `&'static str` fixed in code.
//!
//! The WebSocket client is hand-written and minimal: it only sends small
//! masked text, pong and close frames and reads the frames listed above.

use crate::profile::{Action, ObsOutputOp, ObsReplayBufferOp, ObsSwitchOp};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use serde::Deserialize;
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    hash::{BuildHasher, Hasher, RandomState},
    io::{BufRead, ErrorKind, Read, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, Shutdown, SocketAddr, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

/// Upper bound on one OBS action: connect, handshake, every request and response.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);
/// Largest WebSocket frame payload accepted from OBS.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// Largest accepted OBS WebSocket password, in UTF-8 bytes.
pub const MAX_PASSWORD_BYTES: usize = 1024;
/// `action_unavailable` reason for an `obs_scene` action in a run without an
/// OBS endpoint.
pub const OBS_UNAVAILABLE: &str = "OBS is not configured for this run (turn on OBS in OpenStreamHub Settings, or pass --obs-endpoint); no action was taken";

const MAX_HANDSHAKE_BYTES: usize = 8 * 1024;
/// Frames read while waiting for one message (pings, pongs, OBS events).
const MAX_FRAMES_PER_MESSAGE: usize = 16;
/// Messages skipped while waiting for the request's response.
const MAX_SKIPPED_MESSAGES: usize = 16;
const MAX_AUTH_FIELD_BYTES: usize = 256;
const MAX_ERROR_CHARS: usize = 120;
/// Read timeout slice, so cancellation is noticed while waiting on OBS.
const POLL: Duration = Duration::from_millis(50);
const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
const SUBPROTOCOL: &str = "obswebsocket.json";
const RPC_VERSION: u64 = 1;

const OP_HELLO: u64 = 0;
const OP_IDENTIFY: u64 = 1;
const OP_IDENTIFIED: u64 = 2;
const OP_EVENT: u64 = 5;
const OP_REQUEST: u64 = 6;
const OP_REQUEST_RESPONSE: u64 = 7;

const OPCODE_CONTINUATION: u8 = 0x0;
const OPCODE_TEXT: u8 = 0x1;
const OPCODE_BINARY: u8 = 0x2;
const OPCODE_CLOSE: u8 = 0x8;
const OPCODE_PING: u8 = 0x9;
const OPCODE_PONG: u8 = 0xA;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Host {
    Ipv4Loopback,
    Ipv6Loopback,
    Localhost,
}

/// A loopback obs-websocket endpoint: `127.0.0.1:PORT`, `[::1]:PORT` or
/// `localhost:PORT`, with a port from 1 to 65535.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObsEndpoint {
    host: Host,
    port: u16,
}

impl ObsEndpoint {
    /// The obs-websocket default port.
    pub const DEFAULT_PORT: u16 = 4455;

    /// Parses exactly `127.0.0.1:PORT`, `[::1]:PORT` or `localhost:PORT`.
    ///
    /// Anything else is refused: other addresses (even other loopback
    /// addresses such as `127.0.0.2`), host names, schemes, paths, user
    /// information, whitespace, uppercase, a missing port, port 0, a port with
    /// a leading zero or above 65535.
    pub fn parse(value: &str) -> Result<Self, String> {
        let error = || {
            "OBS endpoint must be 127.0.0.1:PORT, [::1]:PORT or localhost:PORT with a port from 1 to 65535".to_string()
        };
        let (host, port) = if let Some(port) = value.strip_prefix("127.0.0.1:") {
            (Host::Ipv4Loopback, port)
        } else if let Some(port) = value.strip_prefix("[::1]:") {
            (Host::Ipv6Loopback, port)
        } else if let Some(port) = value.strip_prefix("localhost:") {
            (Host::Localhost, port)
        } else {
            return Err(error());
        };
        Ok(Self {
            host,
            port: parse_port(port).ok_or_else(error)?,
        })
    }

    /// Builds an endpoint from a settings host (`127.0.0.1`, `::1` or
    /// `localhost`) and port.
    pub fn new(host: &str, port: u16) -> Result<Self, String> {
        let host = match host {
            "127.0.0.1" => Host::Ipv4Loopback,
            "::1" => Host::Ipv6Loopback,
            "localhost" => Host::Localhost,
            _ => return Err("OBS host must be 127.0.0.1, ::1 or localhost".into()),
        };
        if port == 0 {
            return Err("OBS port must be between 1 and 65535".into());
        }
        Ok(Self { host, port })
    }

    /// The settings form of the host: `127.0.0.1`, `::1` or `localhost`.
    pub fn host(&self) -> &'static str {
        match self.host {
            Host::Ipv4Loopback => "127.0.0.1",
            Host::Ipv6Loopback => "::1",
            Host::Localhost => "localhost",
        }
    }

    /// The TCP port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Addresses to dial, in order. `localhost` never goes through DNS.
    fn candidates(&self) -> Vec<SocketAddr> {
        let v4 = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port);
        let v6 = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), self.port);
        match self.host {
            Host::Ipv4Loopback => vec![v4],
            Host::Ipv6Loopback => vec![v6],
            Host::Localhost => vec![v4, v6],
        }
    }
}

impl fmt::Display for ObsEndpoint {
    /// The canonical form [`ObsEndpoint::parse`] accepts, also used as the
    /// HTTP `Host` header.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.host {
            Host::Ipv6Loopback => write!(formatter, "[::1]:{}", self.port),
            _ => write!(formatter, "{}:{}", self.host(), self.port),
        }
    }
}

fn parse_port(port: &str) -> Option<u16> {
    if port.is_empty()
        || port.len() > 5
        || port.starts_with('0')
        || !port.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    port.parse::<u16>().ok().filter(|port| *port != 0)
}

/// The OBS WebSocket password, held only in memory.
///
/// It has no `Display` or `Serialize`; `Debug` prints `<redacted>`. It is not
/// zeroed on drop: plaintext copies stay in process memory (including the
/// stdin buffer it was read through) for the life of the process, and a core
/// dump would contain them.
#[derive(Clone, Eq, PartialEq)]
pub struct ObsPassword(String);

impl ObsPassword {
    /// Accepts a nonempty password of at most [`MAX_PASSWORD_BYTES`] bytes
    /// without control characters. Errors never contain the password.
    pub fn new(password: String) -> Result<Self, String> {
        if password.is_empty() || password.len() > MAX_PASSWORD_BYTES {
            return Err(format!(
                "OBS password must be nonempty and at most {MAX_PASSWORD_BYTES} bytes"
            ));
        }
        if password.chars().any(char::is_control) {
            return Err("OBS password must not contain control characters".into());
        }
        Ok(Self(password))
    }

    /// Reads the password from the first line of `reader` (the
    /// `--obs-password-stdin` channel): at most [`MAX_PASSWORD_BYTES`] bytes
    /// of UTF-8 followed by an optional `\n` (or `\r\n`) or end of input.
    pub fn read_line(reader: impl BufRead) -> Result<Self, String> {
        let mut bytes = Vec::new();
        reader
            .take(MAX_PASSWORD_BYTES as u64 + 3)
            .read_until(b'\n', &mut bytes)
            .map_err(|error| format!("could not read the OBS password: {}", error.kind()))?;
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
        } else if bytes.len() > MAX_PASSWORD_BYTES {
            return Err(format!(
                "OBS password must be nonempty and at most {MAX_PASSWORD_BYTES} bytes"
            ));
        }
        let password =
            String::from_utf8(bytes).map_err(|_| "OBS password must be valid UTF-8".to_string())?;
        Self::new(password)
    }

    /// The exact bytes [`ObsPassword::read_line`] accepts: the password and a
    /// newline. The only way to read the password outside this module, for a
    /// supervisor that writes it to a child's stdin pipe.
    pub fn stdin_line(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.0.len() + 1);
        bytes.extend_from_slice(self.0.as_bytes());
        bytes.push(b'\n');
        bytes
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ObsPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ObsPassword(<redacted>)")
    }
}

/// Talks to OBS over loopback, one connection per call.
#[derive(Clone, Debug)]
pub struct ObsClient {
    endpoint: ObsEndpoint,
    password: Option<ObsPassword>,
    timeout: Duration,
    cancellation: Option<Arc<AtomicBool>>,
}

impl ObsClient {
    /// A client for `endpoint`. Constructing it performs no I/O.
    pub fn new(endpoint: ObsEndpoint, password: Option<ObsPassword>) -> Self {
        Self {
            endpoint,
            password,
            timeout: DEFAULT_TIMEOUT,
            cancellation: None,
        }
    }

    /// Shortens the per-action deadline; it never exceeds [`DEFAULT_TIMEOUT`].
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout.min(DEFAULT_TIMEOUT);
        self
    }

    /// Stops before connecting, and while waiting on OBS, once `flag` is set.
    pub fn with_cancellation(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancellation = Some(flag);
        self
    }

    /// The configured endpoint.
    pub fn endpoint(&self) -> ObsEndpoint {
        self.endpoint
    }

    /// Whether a password is configured (never the password itself).
    pub fn has_password(&self) -> bool {
        self.password.is_some()
    }

    /// Makes `scene` the current program scene. No retry; on an error the
    /// switch may or may not have happened.
    pub fn set_current_program_scene(&self, scene: &str) -> Result<(), String> {
        self.request(&ObsRequest::with_data(
            "SetCurrentProgramScene",
            json!({"sceneName":scene}),
        ))
        .map(drop)
    }

    /// Sends one request on a fresh connection and returns its
    /// `responseData` (`Value::Null` when OBS sends none). No retry; on an
    /// error the request may or may not have taken effect.
    pub fn request(&self, request: &ObsRequest) -> Result<Value, String> {
        self.session(|session| session.request(request))
    }

    /// Runs an OBS action on one connection. Actions that need OBS's answer
    /// first (a toggle of studio mode, a filter or a source's visibility) read
    /// it and write the opposite on the same connection and deadline. An
    /// action that does not talk to OBS fails without connecting.
    pub fn run(&self, action: &Action) -> Result<(), String> {
        if !action.uses_obs() || matches!(action, Action::MultiAction { .. }) {
            return Err("this action does not talk to OBS; no action was taken".into());
        }
        self.session(|session| run_action(session, action))
    }

    /// Runs `script` on one connection: handshake once, then as many
    /// [`ObsSession::request`] calls as it makes, then close. One deadline
    /// covers the whole script, so a read-then-write toggle costs one
    /// handshake, not two. The script sees OBS's earlier answers and decides
    /// what to send next. No retry; after an error an earlier request may
    /// already have taken effect.
    pub fn session<T>(
        &self,
        script: impl FnOnce(&mut ObsSession<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let deadline = Instant::now() + self.timeout;
        let mut connection = Connection::open(self.endpoint, deadline, self.cancellation.clone())
            .map_err(|error| self.error(error))?;
        let result = self.identify(&mut connection).and_then(|()| {
            script(&mut ObsSession {
                connection: &mut connection,
            })
        });
        connection.close();
        result.map_err(|error| self.error(error))
    }

    fn error(&self, detail: String) -> String {
        format!("OBS at {}: {detail}", self.endpoint)
    }

    fn identify(&self, connection: &mut Connection) -> Result<(), String> {
        connection.handshake(&self.endpoint.to_string())?;
        let hello = connection.read_message()?;
        let authentication = identify_authentication(&hello, self.password.as_ref())?;
        let mut identify = json!({"rpcVersion":RPC_VERSION, "eventSubscriptions":0});
        if let Some(authentication) = authentication {
            identify["authentication"] = json!(authentication);
        }
        connection.send_message(&json!({"op":OP_IDENTIFY, "d":identify}))?;
        let identified = connection.read_message()?;
        check_identified(&identified)
    }
}

/// One obs-websocket request: a fixed request type and optional data.
///
/// The type is a `&'static str` chosen in code, never profile text. Names
/// (scenes, inputs, sources) belong in `data`, where `serde_json` builds the
/// JSON string; nothing is formatted into a request.
#[derive(Clone, Debug, PartialEq)]
pub struct ObsRequest {
    request_type: &'static str,
    data: Option<Value>,
}

impl ObsRequest {
    /// A request without `requestData`.
    pub fn new(request_type: &'static str) -> Self {
        Self {
            request_type,
            data: None,
        }
    }

    /// A request with `requestData`.
    pub fn with_data(request_type: &'static str, data: Value) -> Self {
        Self {
            request_type,
            data: Some(data),
        }
    }

    /// The request type, such as `ToggleRecord`.
    pub fn request_type(&self) -> &'static str {
        self.request_type
    }
}

/// The state an on/off action should set: `on` and `off` are fixed, `toggle`
/// is the opposite of what `read` says OBS has now.
fn target_state(
    op: ObsSwitchOp,
    read: impl FnOnce() -> Result<bool, String>,
) -> Result<bool, String> {
    Ok(match op {
        ObsSwitchOp::On => true,
        ObsSwitchOp::Off => false,
        ObsSwitchOp::Toggle => !read()?,
    })
}

fn response_flag(data: &Value, field: &str) -> Result<bool, String> {
    data.get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("OBS response has no {field}"))
}

fn run_action(session: &mut ObsSession<'_>, action: &Action) -> Result<(), String> {
    match action {
        Action::ObsStudioMode { op } => {
            let enabled = target_state(*op, || {
                let current = session.request(&ObsRequest::new("GetStudioModeEnabled"))?;
                response_flag(&current, "studioModeEnabled")
            })?;
            session
                .request(&ObsRequest::with_data(
                    "SetStudioModeEnabled",
                    json!({"studioModeEnabled":enabled}),
                ))
                .map(drop)
        }
        Action::ObsFilter { source, filter, op } => {
            let enabled = target_state(*op, || {
                let current = session.request(&ObsRequest::with_data(
                    "GetSourceFilter",
                    json!({"sourceName":source, "filterName":filter}),
                ))?;
                response_flag(&current, "filterEnabled")
            })?;
            session
                .request(&ObsRequest::with_data(
                    "SetSourceFilterEnabled",
                    json!({"sourceName":source, "filterName":filter, "filterEnabled":enabled}),
                ))
                .map(drop)
        }
        Action::ObsSourceVisibility { scene, source, op } => {
            let found = session.request(&ObsRequest::with_data(
                "GetSceneItemId",
                json!({"sceneName":scene, "sourceName":source}),
            ))?;
            let item = found
                .get("sceneItemId")
                .and_then(Value::as_u64)
                .ok_or("OBS response has no sceneItemId")?;
            let enabled = target_state(*op, || {
                let current = session.request(&ObsRequest::with_data(
                    "GetSceneItemEnabled",
                    json!({"sceneName":scene, "sceneItemId":item}),
                ))?;
                response_flag(&current, "sceneItemEnabled")
            })?;
            session
                .request(&ObsRequest::with_data(
                    "SetSceneItemEnabled",
                    json!({"sceneName":scene, "sceneItemId":item, "sceneItemEnabled":enabled}),
                ))
                .map(drop)
        }
        other => {
            let request = ObsRequest::for_action(other)
                .ok_or("this action does not talk to OBS; no action was taken")?;
            session.request(&request).map(drop)
        }
    }
}

impl ObsRequest {
    /// The request a single-request OBS action sends, or `None` for an action
    /// that needs several requests ([`ObsClient::run`] handles those), does not
    /// talk to OBS, or is a `multi_action`.
    pub fn for_action(action: &Action) -> Option<Self> {
        Some(match action {
            Action::ObsScene { scene } => {
                Self::with_data("SetCurrentProgramScene", json!({"sceneName":scene}))
            }
            Action::ObsVirtualCamera { op } => Self::new(match op {
                ObsOutputOp::Toggle => "ToggleVirtualCam",
                ObsOutputOp::Start => "StartVirtualCam",
                ObsOutputOp::Stop => "StopVirtualCam",
            }),
            Action::ObsTransition => Self::new("TriggerStudioModeTransition"),
            Action::ObsSceneCollection { name } => Self::with_data(
                "SetCurrentSceneCollection",
                json!({"sceneCollectionName":name}),
            ),
            Action::ObsProfile { name } => {
                Self::with_data("SetCurrentProfile", json!({"profileName":name}))
            }
            Action::ObsReplayBuffer { op } => Self::new(match op {
                ObsReplayBufferOp::Toggle => "ToggleReplayBuffer",
                ObsReplayBufferOp::Start => "StartReplayBuffer",
                ObsReplayBufferOp::Stop => "StopReplayBuffer",
                ObsReplayBufferOp::Save => "SaveReplayBuffer",
            }),
            _ => return None,
        })
    }
}

/// An identified connection inside [`ObsClient::session`].
pub struct ObsSession<'a> {
    connection: &'a mut Connection,
}

impl ObsSession<'_> {
    /// Sends `request` and returns its `responseData` (`Value::Null` when OBS
    /// sends none). Events and pings that arrive first are skipped.
    pub fn request(&mut self, request: &ObsRequest) -> Result<Value, String> {
        let request_id = format!("osh-{}", hex(&random_bytes::<8>()));
        let mut d = json!({"requestType":request.request_type, "requestId":request_id});
        if let Some(data) = &request.data {
            d["requestData"] = data.clone();
        }
        self.connection
            .send_message(&json!({"op":OP_REQUEST, "d":d}))?;
        for _ in 0..=MAX_SKIPPED_MESSAGES {
            let message = self.connection.read_message()?;
            match message.op {
                OP_EVENT => continue,
                OP_REQUEST_RESPONSE => {
                    return check_response(&message.d, &request_id, request);
                }
                op => {
                    return Err(format!(
                        "unexpected OBS message (op {op}) instead of the response"
                    ));
                }
            }
        }
        Err("OBS sent too many messages before the response".into())
    }
}

/// One decoded obs-websocket message.
#[derive(Debug, Deserialize)]
struct Message {
    op: u64,
    d: Value,
}

#[derive(Deserialize)]
struct Hello {
    #[serde(rename = "rpcVersion")]
    rpc_version: u64,
    authentication: Option<Challenge>,
}

#[derive(Deserialize)]
struct Challenge {
    challenge: String,
    salt: String,
}

/// The `Identify` authentication string for a `Hello`, or `None` when OBS
/// does not require authentication.
fn identify_authentication(
    hello: &Message,
    password: Option<&ObsPassword>,
) -> Result<Option<String>, String> {
    if hello.op != OP_HELLO {
        return Err(format!("expected OBS Hello (op 0), got op {}", hello.op));
    }
    let hello: Hello = serde_json::from_value(hello.d.clone())
        .map_err(|_| "OBS Hello is malformed".to_string())?;
    if hello.rpc_version < RPC_VERSION {
        return Err("OBS WebSocket does not support RPC version 1".into());
    }
    let Some(challenge) = hello.authentication else {
        return Ok(None);
    };
    if challenge.challenge.len() > MAX_AUTH_FIELD_BYTES
        || challenge.salt.len() > MAX_AUTH_FIELD_BYTES
    {
        return Err("OBS authentication challenge is too long".into());
    }
    let Some(password) = password else {
        return Err("OBS requires a WebSocket password, but none is set for this run (set it in OpenStreamHub Settings)".into());
    };
    Ok(Some(authentication_string(
        password,
        &challenge.salt,
        &challenge.challenge,
    )))
}

/// obs-websocket v5 authentication:
/// `base64(sha256(base64(sha256(password + salt)) + challenge))`.
fn authentication_string(password: &ObsPassword, salt: &str, challenge: &str) -> String {
    let mut secret = Sha256::new();
    secret.update(password.expose().as_bytes());
    secret.update(salt.as_bytes());
    let secret = BASE64.encode(secret.finalize());
    let mut response = Sha256::new();
    response.update(secret.as_bytes());
    response.update(challenge.as_bytes());
    BASE64.encode(response.finalize())
}

fn check_identified(message: &Message) -> Result<(), String> {
    if message.op != OP_IDENTIFIED {
        return Err(format!(
            "expected OBS Identified (op 2), got op {}",
            message.op
        ));
    }
    match message
        .d
        .get("negotiatedRpcVersion")
        .and_then(Value::as_u64)
    {
        Some(RPC_VERSION) => Ok(()),
        _ => Err("OBS did not agree on RPC version 1".into()),
    }
}

fn check_response(data: &Value, request_id: &str, request: &ObsRequest) -> Result<Value, String> {
    let request_type = request.request_type;
    if data.get("requestId").and_then(Value::as_str) != Some(request_id)
        || data.get("requestType").and_then(Value::as_str) != Some(request_type)
    {
        return Err("OBS answered a different request".into());
    }
    let status = data
        .get("requestStatus")
        .ok_or("OBS response has no request status")?;
    let code = status.get("code").and_then(Value::as_u64);
    match (status.get("result").and_then(Value::as_bool), code) {
        (Some(true), Some(100)) => Ok(data.get("responseData").cloned().unwrap_or(Value::Null)),
        (Some(false), Some(code)) => {
            // OBS often repeats the name it was given ("No source was found by
            // the name of X"). A request that carried data may carry a scene,
            // source or filter name, which must stay out of logs and
            // diagnostics, so its comment is never shown; the reason above
            // already names the cause.
            let comment = if request.data.is_some() {
                String::new()
            } else {
                status
                    .get("comment")
                    .and_then(Value::as_str)
                    .map(|comment| format!(": {}", sanitize(comment)))
                    .unwrap_or_default()
            };
            Err(format!(
                "OBS refused {} ({}){comment}",
                refused_what(request_type),
                refusal_reason(request_type, code)
            ))
        }
        _ => Err("OBS response has a malformed request status".into()),
    }
}

/// What the refused request was, for the error text. Scene switches keep the
/// wording the app and the hardware checklist already show.
fn refused_what(request_type: &str) -> String {
    match request_type {
        "SetCurrentProgramScene" => "the scene switch".into(),
        "SetCurrentSceneCollection" => "the scene collection switch".into(),
        "SetCurrentProfile" => "the profile switch".into(),
        other => other.into(),
    }
}

/// A short reason for a failed request status code (obs-websocket v5
/// `RequestStatus`). Unknown codes show as the number only.
fn refusal_reason(request_type: &str, code: u64) -> String {
    match (request_type, code) {
        ("SetCurrentProgramScene", 600) => "no scene with that name".into(),
        ("SetCurrentSceneCollection", 600) => "no scene collection with that name".into(),
        ("SetCurrentProfile", 600) => "no profile with that name".into(),
        (_, 207) => "OBS is not ready yet".into(),
        (_, 500) => "the output is already running".into(),
        (_, 501) => "the output is not running".into(),
        (_, 502) => "the output is paused".into(),
        (_, 503) => "the output is not paused".into(),
        (_, 504) => "the output is disabled".into(),
        (_, 505) => "studio mode is active".into(),
        (_, 506) => "studio mode is not active".into(),
        ("GetSceneItemId", 600) => "no such scene, or no such source in it".into(),
        ("GetSourceFilter" | "SetSourceFilterEnabled", 600) => {
            "no such source, or no such filter on it".into()
        }
        ("GetSceneItemEnabled" | "SetSceneItemEnabled", 600) => {
            "the source is no longer in that scene".into()
        }
        (_, 602) => "that name belongs to a different kind of item".into(),
        (_, 600) => "not found".into(),
        (_, 601) => "already exists".into(),
        (_, 604) => "the resource is in the wrong state".into(),
        _ => format!("code {code}"),
    }
}

/// Untrusted text for an error message: control and invisible characters
/// become U+FFFD and the result is cut to 120 characters.
fn sanitize(text: &str) -> String {
    let mut cleaned: String = text
        .chars()
        .take(MAX_ERROR_CHARS)
        .map(|c| {
            if c.is_control() || crate::is_invisible_text_character(c) {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect();
    if text.chars().nth(MAX_ERROR_CHARS).is_some() {
        cleaned.push('…');
    }
    cleaned
}

/// Why OBS closed the connection, from the close code only (the close
/// reason text is never shown).
fn close_error(payload: &[u8]) -> String {
    match payload {
        [high, low, ..] => match u16::from_be_bytes([*high, *low]) {
            4009 => "OBS rejected the password (authentication failed); check the OBS WebSocket password".into(),
            4010 => "OBS WebSocket does not support RPC version 1".into(),
            code => format!("OBS closed the connection (code {code})"),
        },
        _ => "OBS closed the connection".into(),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Unpredictable bytes for the WebSocket key, masking keys and request ids.
///
/// Built from `std::hash::RandomState` (SipHash with per-thread keys seeded
/// from the operating system, varied by a counter and the time). The values
/// are only WebSocket masking keys, the handshake nonce and request ids; they
/// need no secrecy and protect nothing, so this is not a cryptographic RNG.
fn random_bytes<const N: usize>() -> [u8; N] {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut bytes = [0; N];
    for chunk in bytes.chunks_mut(8) {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos()),
        );
        chunk.copy_from_slice(&hasher.finish().to_le_bytes()[..chunk.len()]);
    }
    bytes
}

/// Checks that the connected peer is the loopback address that was dialled.
fn verify_peer(peer: SocketAddr, dialled: SocketAddr) -> Result<(), String> {
    if peer.ip().is_loopback() && peer == dialled {
        Ok(())
    } else {
        Err("the connected peer is not the loopback address that was dialled".into())
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// One WebSocket connection with a shared deadline.
struct Connection {
    stream: TcpStream,
    buffer: Vec<u8>,
    deadline: Instant,
    cancellation: Option<Arc<AtomicBool>>,
}

impl Connection {
    fn open(
        endpoint: ObsEndpoint,
        deadline: Instant,
        cancellation: Option<Arc<AtomicBool>>,
    ) -> Result<Self, String> {
        let mut last_error = None;
        for address in endpoint.candidates() {
            check_cancelled(cancellation.as_deref())?;
            let remaining = remaining(deadline)?;
            match TcpStream::connect_timeout(&address, remaining) {
                Ok(stream) => {
                    let peer = stream
                        .peer_addr()
                        .map_err(|error| format!("could not read the peer address: {error}"))?;
                    verify_peer(peer, address)?;
                    let _ = stream.set_nodelay(true);
                    return Ok(Self {
                        stream,
                        buffer: Vec::new(),
                        deadline,
                        cancellation,
                    });
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(match last_error {
            Some(error) if error.kind() == ErrorKind::TimedOut => {
                "timed out connecting; is OBS running with its WebSocket server on?".into()
            }
            Some(error) => {
                format!("could not connect ({error}); is OBS running with its WebSocket server on?")
            }
            None => "could not connect".into(),
        })
    }

    fn handshake(&mut self, host: &str) -> Result<(), String> {
        let key = BASE64.encode(random_bytes::<16>());
        let request = format!(
            "GET / HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: {SUBPROTOCOL}\r\n\r\n"
        );
        self.write_all(request.as_bytes())?;
        let end = loop {
            if let Some(end) = find(&self.buffer, b"\r\n\r\n") {
                break end;
            }
            if self.buffer.len() > MAX_HANDSHAKE_BYTES {
                return Err(format!(
                    "the WebSocket upgrade response exceeds {MAX_HANDSHAKE_BYTES} bytes"
                ));
            }
            self.read_more()?;
        };
        if end + 4 > MAX_HANDSHAKE_BYTES {
            return Err(format!(
                "the WebSocket upgrade response exceeds {MAX_HANDSHAKE_BYTES} bytes"
            ));
        }
        let head: Vec<u8> = self.buffer.drain(..end + 4).collect();
        check_upgrade(&head[..end], &key)
    }

    fn read_message(&mut self) -> Result<Message, String> {
        for _ in 0..MAX_FRAMES_PER_MESSAGE {
            let (opcode, payload) = self.read_frame()?;
            match opcode {
                OPCODE_TEXT => {
                    let text = std::str::from_utf8(&payload)
                        .map_err(|_| "OBS sent a text frame that is not UTF-8".to_string())?;
                    return serde_json::from_str(text)
                        .map_err(|_| "OBS sent a message that is not obs-websocket JSON".into());
                }
                OPCODE_PING => self.send_frame(OPCODE_PONG, &payload)?,
                OPCODE_PONG => {}
                OPCODE_CLOSE => return Err(close_error(&payload)),
                _ => unreachable!("read_frame returns only supported opcodes"),
            }
        }
        Err("OBS sent too many control frames".into())
    }

    /// Reads one complete, unfragmented, unmasked frame of a supported type.
    fn read_frame(&mut self) -> Result<(u8, Vec<u8>), String> {
        self.fill(2)?;
        let (first, second) = (self.buffer[0], self.buffer[1]);
        if first & 0x70 != 0 {
            return Err("OBS sent a frame with reserved bits set".into());
        }
        if second & 0x80 != 0 {
            return Err("OBS sent a masked frame".into());
        }
        let opcode = first & 0x0F;
        match opcode {
            OPCODE_TEXT | OPCODE_CLOSE | OPCODE_PING | OPCODE_PONG => {}
            OPCODE_CONTINUATION => {
                return Err("OBS sent a fragmented message, which is not supported".into());
            }
            OPCODE_BINARY => return Err("OBS sent a binary frame".into()),
            other => return Err(format!("OBS sent a frame with reserved opcode {other}")),
        }
        if first & 0x80 == 0 {
            return Err("OBS sent a fragmented message, which is not supported".into());
        }
        let (header, length) = match second & 0x7F {
            126 => {
                self.fill(4)?;
                (
                    4,
                    u64::from(u16::from_be_bytes([self.buffer[2], self.buffer[3]])),
                )
            }
            127 => {
                self.fill(10)?;
                let mut bytes = [0; 8];
                bytes.copy_from_slice(&self.buffer[2..10]);
                (10, u64::from_be_bytes(bytes))
            }
            short => (2, u64::from(short)),
        };
        if length > MAX_FRAME_BYTES as u64 {
            return Err(format!(
                "OBS sent a frame larger than {MAX_FRAME_BYTES} bytes"
            ));
        }
        if (header == 4 && length < 126) || (header == 10 && length <= u64::from(u16::MAX)) {
            return Err("OBS sent a frame with a non-minimal length".into());
        }
        if opcode >= OPCODE_CLOSE && length > 125 {
            return Err("OBS sent a control frame longer than 125 bytes".into());
        }
        let total = header + length as usize;
        self.fill(total)?;
        let payload = self.buffer[header..total].to_vec();
        self.buffer.drain(..total);
        Ok((opcode, payload))
    }

    fn send_message(&mut self, message: &Value) -> Result<(), String> {
        let text = serde_json::to_vec(message)
            .map_err(|_| "could not encode the OBS message".to_string())?;
        self.send_frame(OPCODE_TEXT, &text)
    }

    /// Writes one final, masked frame.
    fn send_frame(&mut self, opcode: u8, payload: &[u8]) -> Result<(), String> {
        let frame = encode_frame(opcode, payload, random_bytes::<4>())?;
        self.write_all(&frame)
    }

    /// Best effort: a normal-closure frame, then shutdown. Never waits.
    fn close(&mut self) {
        if let Ok(frame) = encode_frame(OPCODE_CLOSE, &1000u16.to_be_bytes(), random_bytes::<4>()) {
            let _ = self.stream.set_write_timeout(Some(POLL));
            let _ = self.stream.write_all(&frame);
        }
        let _ = self.stream.shutdown(Shutdown::Both);
    }

    fn fill(&mut self, length: usize) -> Result<(), String> {
        while self.buffer.len() < length {
            self.read_more()?;
        }
        Ok(())
    }

    fn read_more(&mut self) -> Result<(), String> {
        let mut chunk = [0; 4096];
        loop {
            check_cancelled(self.cancellation.as_deref())?;
            let wait = remaining(self.deadline)?.min(POLL);
            self.stream
                .set_read_timeout(Some(wait))
                .map_err(|error| format!("could not set a read timeout: {error}"))?;
            match self.stream.read(&mut chunk) {
                Ok(0) => return Err("OBS closed the connection early".into()),
                Ok(read) => {
                    self.buffer.extend_from_slice(&chunk[..read]);
                    return Ok(());
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(format!("reading from OBS failed: {error}")),
            }
        }
    }

    fn write_all(&mut self, bytes: &[u8]) -> Result<(), String> {
        check_cancelled(self.cancellation.as_deref())?;
        let wait = remaining(self.deadline)?;
        self.stream
            .set_write_timeout(Some(wait))
            .map_err(|error| format!("could not set a write timeout: {error}"))?;
        self.stream
            .write_all(bytes)
            .map_err(|error| match error.kind() {
                ErrorKind::WouldBlock | ErrorKind::TimedOut => "timed out writing to OBS".into(),
                _ => format!("writing to OBS failed: {error}"),
            })
    }
}

fn encode_frame(opcode: u8, payload: &[u8], mask: [u8; 4]) -> Result<Vec<u8>, String> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(format!("an OBS message exceeds {MAX_FRAME_BYTES} bytes"));
    }
    let mut frame = Vec::with_capacity(payload.len() + 8);
    frame.push(0x80 | opcode);
    match payload.len() {
        length @ 0..=125 => frame.push(0x80 | length as u8),
        length => {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(length as u16).to_be_bytes());
        }
    }
    frame.extend_from_slice(&mask);
    frame.extend(
        payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4]),
    );
    Ok(frame)
}

/// Validates the `101 Switching Protocols` response head (without its final
/// blank line) for the request made with `key`.
fn check_upgrade(head: &[u8], key: &str) -> Result<(), String> {
    let head = std::str::from_utf8(head)
        .map_err(|_| "the WebSocket upgrade response is not text".to_string())?;
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or_default();
    let mut parts = status.splitn(3, ' ');
    let (version, code) = (parts.next(), parts.next());
    if version != Some("HTTP/1.1") || code != Some("101") {
        let shown = code
            .filter(|code| code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_digit()))
            .unwrap_or("unknown");
        return Err(format!(
            "OBS did not accept the WebSocket upgrade (HTTP status {shown}); redirects are not followed"
        ));
    }
    let (mut upgrade, mut connection, mut accept, mut protocol) = (None, None, None, None);
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or("the WebSocket upgrade response has a malformed header")?;
        let value = value.trim_matches([' ', '\t']);
        let slot = match name.to_ascii_lowercase().as_str() {
            "upgrade" => &mut upgrade,
            "connection" => &mut connection,
            "sec-websocket-accept" => &mut accept,
            "sec-websocket-protocol" => &mut protocol,
            "sec-websocket-extensions" => {
                return Err("OBS negotiated a WebSocket extension that was not requested".into());
            }
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return Err("the WebSocket upgrade response repeats a header".into());
        }
    }
    if !upgrade.is_some_and(|value| value.eq_ignore_ascii_case("websocket"))
        || !connection.is_some_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        })
    {
        return Err("the WebSocket upgrade response is missing Upgrade or Connection".into());
    }
    if accept != Some(expected_accept(key).as_str()) {
        return Err("the WebSocket upgrade response has a wrong Sec-WebSocket-Accept".into());
    }
    if protocol.is_some_and(|protocol| protocol != SUBPROTOCOL) {
        return Err("OBS chose a WebSocket subprotocol other than obswebsocket.json".into());
    }
    Ok(())
}

fn expected_accept(key: &str) -> String {
    let mut hash = Sha1::new();
    hash.update(key.as_bytes());
    hash.update(WEBSOCKET_GUID.as_bytes());
    BASE64.encode(hash.finalize())
}

fn remaining(deadline: Instant) -> Result<Duration, String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(format!(
            "timed out waiting for OBS (at most {} s per action, less when a multi_action's deadline is near)",
            DEFAULT_TIMEOUT.as_secs()
        ))
    } else {
        Ok(remaining)
    }
}

fn check_cancelled(cancellation: Option<&AtomicBool>) -> Result<(), String> {
    if cancellation.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
        Err("cancelled before OBS answered".into())
    } else {
        Ok(())
    }
}

/// Minimal server-side helpers for fake OBS servers in other modules' tests.
#[cfg(test)]
pub(crate) mod tests_support {
    use std::{io::Read, net::TcpStream};

    /// The `Sec-WebSocket-Accept` value for a client key.
    pub(crate) fn accept(key: &str) -> String {
        super::expected_accept(key)
    }

    /// Reads one small masked client text frame and unmasks it.
    pub(crate) fn read_text(stream: &mut TcpStream) -> String {
        let mut header = [0; 2];
        stream.read_exact(&mut header).unwrap();
        assert_eq!(header[0], 0x81, "a final text frame");
        assert_eq!(header[1] & 0x80, 0x80, "client frames are masked");
        let length = match header[1] & 0x7F {
            126 => {
                let mut bytes = [0; 2];
                stream.read_exact(&mut bytes).unwrap();
                u16::from_be_bytes(bytes) as usize
            }
            short => short as usize,
        };
        let mut mask = [0; 4];
        stream.read_exact(&mut mask).unwrap();
        let mut payload = vec![0; length];
        stream.read_exact(&mut payload).unwrap();
        payload
            .iter_mut()
            .enumerate()
            .for_each(|(index, byte)| *byte ^= mask[index % 4]);
        String::from_utf8(payload).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::BufReader,
        net::TcpListener,
        sync::mpsc,
        thread::{self, JoinHandle},
    };

    const PASSWORD: &str = "hunter2-correct-horse";
    const FAST: Duration = Duration::from_millis(600);

    // ---- A scripted fake OBS server on an ephemeral loopback port. ----

    struct Peer {
        stream: TcpStream,
        buffer: Vec<u8>,
    }

    impl Peer {
        fn read_more(&mut self) -> bool {
            let mut chunk = [0; 4096];
            match self.stream.read(&mut chunk) {
                Ok(0) | Err(_) => false,
                Ok(read) => {
                    self.buffer.extend_from_slice(&chunk[..read]);
                    true
                }
            }
        }

        /// Reads the upgrade request and returns its headers (lowercase names).
        fn read_request(&mut self) -> Vec<(String, String)> {
            let end = loop {
                if let Some(end) = find(&self.buffer, b"\r\n\r\n") {
                    break end;
                }
                assert!(self.read_more(), "client closed before the request ended");
            };
            let head = String::from_utf8(self.buffer.drain(..end + 4).collect()).unwrap();
            let mut lines = head.trim_end().split("\r\n");
            assert_eq!(lines.next(), Some("GET / HTTP/1.1"));
            lines
                .map(|line| {
                    let (name, value) = line.split_once(": ").unwrap();
                    (name.to_ascii_lowercase(), value.to_owned())
                })
                .collect()
        }

        fn accept_upgrade(&mut self) -> Vec<(String, String)> {
            let headers = self.read_request();
            let key = &headers
                .iter()
                .find(|(name, _)| name == "sec-websocket-key")
                .unwrap()
                .1;
            let response = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\nSec-WebSocket-Protocol: obswebsocket.json\r\n\r\n",
                expected_accept(key)
            );
            self.stream.write_all(response.as_bytes()).unwrap();
            headers
        }

        fn send_raw(&mut self, bytes: &[u8]) {
            let _ = self.stream.write_all(bytes);
        }

        fn send_frame(&mut self, first: u8, payload: &[u8]) {
            let mut frame = vec![first];
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
            self.send_raw(&frame);
        }

        fn send_json(&mut self, value: Value) {
            self.send_frame(0x81, value.to_string().as_bytes());
        }

        /// Reads one client frame; asserts it is final and masked.
        fn read_frame(&mut self) -> Option<(u8, Vec<u8>)> {
            while self.buffer.len() < 2 {
                if !self.read_more() {
                    return None;
                }
            }
            assert_eq!(self.buffer[0] & 0xF0, 0x80, "client frames are final");
            assert_eq!(self.buffer[1] & 0x80, 0x80, "client frames are masked");
            let (header, length) = match self.buffer[1] & 0x7F {
                126 => {
                    while self.buffer.len() < 4 {
                        assert!(self.read_more());
                    }
                    (
                        4,
                        u16::from_be_bytes([self.buffer[2], self.buffer[3]]) as usize,
                    )
                }
                127 => panic!("client frames stay small"),
                short => (2, short as usize),
            };
            while self.buffer.len() < header + 4 + length {
                assert!(self.read_more());
            }
            let mask: [u8; 4] = self.buffer[header..header + 4].try_into().unwrap();
            let payload = self.buffer[header + 4..header + 4 + length]
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ mask[index % 4])
                .collect();
            let opcode = self.buffer[0] & 0x0F;
            self.buffer.drain(..header + 4 + length);
            Some((opcode, payload))
        }

        fn read_json(&mut self) -> Value {
            let (opcode, payload) = self.read_frame().expect("a client message");
            assert_eq!(opcode, OPCODE_TEXT);
            serde_json::from_slice(&payload).unwrap()
        }

        fn close(&mut self, code: u16, reason: &str) {
            let mut payload = code.to_be_bytes().to_vec();
            payload.extend_from_slice(reason.as_bytes());
            self.send_frame(0x88, &payload);
        }
    }

    /// Everything the fake server saw, for assertions in the test thread.
    #[derive(Default, Debug)]
    struct Seen {
        frames: Vec<(u8, Vec<u8>)>,
        messages: Vec<Value>,
        headers: Vec<(String, String)>,
    }

    fn serve(
        script: impl FnOnce(&mut Peer, &mut Seen) + Send + 'static,
    ) -> (ObsEndpoint, JoinHandle<Seen>) {
        serve_on("127.0.0.1:0", script).unwrap()
    }

    fn serve_on(
        bind: &str,
        script: impl FnOnce(&mut Peer, &mut Seen) + Send + 'static,
    ) -> Option<(ObsEndpoint, JoinHandle<Seen>)> {
        let listener = TcpListener::bind(bind).ok()?;
        let address = listener.local_addr().unwrap();
        let endpoint = match address {
            SocketAddr::V4(_) => ObsEndpoint::parse(&format!("127.0.0.1:{}", address.port())),
            SocketAddr::V6(_) => ObsEndpoint::parse(&format!("[::1]:{}", address.port())),
        }
        .unwrap();
        let join = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut peer = Peer {
                stream,
                buffer: Vec::new(),
            };
            let mut seen = Seen::default();
            script(&mut peer, &mut seen);
            // Whatever the client sends afterwards (its close frame).
            while let Some(frame) = peer.read_frame_lenient() {
                seen.frames.push(frame);
            }
            seen
        });
        Some((endpoint, join))
    }

    impl Peer {
        /// Like `read_frame`, but returns `None` for anything that is not a
        /// well-formed client frame (such as an unread HTTP request) or after
        /// 300 ms without data.
        fn read_frame_lenient(&mut self) -> Option<(u8, Vec<u8>)> {
            let _ = self
                .stream
                .set_read_timeout(Some(Duration::from_millis(300)));
            while self.buffer.len() < 2 {
                if !self.read_more() {
                    return None;
                }
            }
            if self.buffer[0] & 0xF0 != 0x80 || self.buffer[1] & 0x80 == 0 {
                return None;
            }
            self.read_frame()
        }
    }

    fn hello(authentication: Option<(&str, &str)>) -> Value {
        let mut d = json!({"obsWebSocketVersion":"5.5.0","rpcVersion":1});
        if let Some((challenge, salt)) = authentication {
            d["authentication"] = json!({"challenge":challenge,"salt":salt});
        }
        json!({"op":0,"d":d})
    }

    const SALT: &str = "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=";
    const CHALLENGE: &str = "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=";

    /// A well-behaved OBS: optional authentication, then one scene switch.
    fn obs_server(
        authentication: Option<&'static str>,
        result: Value,
    ) -> impl FnOnce(&mut Peer, &mut Seen) + Send + 'static {
        move |peer, seen| {
            seen.headers = peer.accept_upgrade();
            peer.send_json(hello(authentication.map(|_| (CHALLENGE, SALT))));
            let identify = peer.read_json();
            seen.messages.push(identify.clone());
            if let Some(password) = authentication {
                let expected =
                    authentication_string(&ObsPassword(password.into()), SALT, CHALLENGE);
                if identify["d"]["authentication"] != json!(expected) {
                    peer.close(4009, "Authentication failed.");
                    return;
                }
            }
            peer.send_json(json!({"op":2,"d":{"negotiatedRpcVersion":1}}));
            let request = peer.read_json();
            seen.messages.push(request.clone());
            let mut response = json!({"op":7,"d":{
                "requestType":"SetCurrentProgramScene",
                "requestId":request["d"]["requestId"],
                "requestStatus":result,
            }});
            if response["d"]["requestStatus"]["result"] == json!(true) {
                response["d"]["requestStatus"]["code"] = json!(100);
            }
            peer.send_json(response);
        }
    }

    fn client(endpoint: ObsEndpoint, password: Option<&str>) -> ObsClient {
        ObsClient::new(
            endpoint,
            password.map(|password| ObsPassword::new(password.into()).unwrap()),
        )
        .with_timeout(FAST)
    }

    fn assert_no_secret(text: &str) {
        assert!(!text.contains(PASSWORD), "password leaked: {text}");
        assert!(!text.contains("hunter2"), "password leaked: {text}");
    }

    // ---- Endpoint validation ----

    #[test]
    fn endpoints_are_loopback_only_with_a_strict_port() {
        for (value, host, port, shown) in [
            ("127.0.0.1:4455", "127.0.0.1", 4455, "127.0.0.1:4455"),
            ("[::1]:4455", "::1", 4455, "[::1]:4455"),
            ("localhost:1", "localhost", 1, "localhost:1"),
            ("127.0.0.1:65535", "127.0.0.1", 65535, "127.0.0.1:65535"),
        ] {
            let endpoint = ObsEndpoint::parse(value).unwrap();
            assert_eq!((endpoint.host(), endpoint.port()), (host, port));
            assert_eq!(endpoint.to_string(), shown);
            assert_eq!(ObsEndpoint::new(host, port).unwrap(), endpoint);
            for address in endpoint.candidates() {
                assert!(address.ip().is_loopback());
            }
        }
        for invalid in [
            "",
            "127.0.0.1",
            "127.0.0.1:",
            "127.0.0.1:0",
            "127.0.0.1:65536",
            "127.0.0.1:04455",
            "127.0.0.1:+4455",
            "127.0.0.1:44 55",
            "127.0.0.1:4455 ",
            " 127.0.0.1:4455",
            "127.0.0.2:4455",
            "127.1:4455",
            "0.0.0.0:4455",
            "192.168.1.10:4455",
            "10.0.0.1:4455",
            "::1:4455",
            "[::]:4455",
            "[::ffff:127.0.0.1]:4455",
            "[0:0:0:0:0:0:0:1]:4455",
            "LOCALHOST:4455",
            "localhost.:4455",
            "localhost.example.com:4455",
            "obs.local:4455",
            "example.com:4455",
            "ws://127.0.0.1:4455",
            "127.0.0.1:4455/path",
            "user@127.0.0.1:4455",
            "127.0.0.1:4455\n",
        ] {
            assert!(ObsEndpoint::parse(invalid).is_err(), "{invalid:?}");
        }
        for (host, port) in [
            ("127.0.0.1", 0),
            ("0.0.0.0", 4455),
            ("192.168.0.2", 4455),
            ("[::1]", 4455),
            ("Localhost", 4455),
            ("", 4455),
        ] {
            assert!(ObsEndpoint::new(host, port).is_err(), "{host}:{port}");
        }
    }

    #[test]
    fn the_connected_peer_must_be_the_dialled_loopback_address() {
        let dialled: SocketAddr = "127.0.0.1:4455".parse().unwrap();
        assert!(verify_peer(dialled, dialled).is_ok());
        for peer in [
            "192.168.1.2:4455",
            "127.0.0.2:4455",
            "127.0.0.1:4456",
            "[::1]:4455",
            "[::ffff:127.0.0.1]:4455",
        ] {
            assert!(
                verify_peer(peer.parse().unwrap(), dialled).is_err(),
                "{peer}"
            );
        }
        let v6: SocketAddr = "[::1]:4455".parse().unwrap();
        assert!(verify_peer(v6, v6).is_ok());
    }

    // ---- Password handling ----

    #[test]
    fn passwords_are_bounded_and_never_printed() {
        let password = ObsPassword::new(PASSWORD.into()).unwrap();
        assert_no_secret(&format!("{password:?}"));
        let client = ObsClient::new(
            ObsEndpoint::parse("127.0.0.1:4455").unwrap(),
            Some(password),
        );
        assert_no_secret(&format!("{client:?}"));
        assert!(client.has_password());
        for invalid in [
            String::new(),
            "x".repeat(MAX_PASSWORD_BYTES + 1),
            "a\nb".into(),
            "a\0b".into(),
            "tab\t".into(),
        ] {
            let error = ObsPassword::new(invalid.clone()).unwrap_err();
            assert!(!error.contains(&invalid) || invalid.is_empty(), "{error}");
        }
        assert!(ObsPassword::new("x".repeat(MAX_PASSWORD_BYTES)).is_ok());
        assert!(ObsPassword::new("zażółć 🎉".into()).is_ok());
    }

    #[test]
    fn the_stdin_channel_reads_one_bounded_line() {
        let read = |bytes: &[u8]| ObsPassword::read_line(BufReader::new(bytes));
        assert_eq!(read(b"secret\n").unwrap().expose(), "secret");
        assert_eq!(read(b"secret\r\n").unwrap().expose(), "secret");
        assert_eq!(read(b"secret").unwrap().expose(), "secret");
        assert_eq!(read(b"first\nsecond\n").unwrap().expose(), "first");
        // The supervisor's write is exactly what the service reads back.
        let password = ObsPassword::new("pass word 🎉".into()).unwrap();
        assert_eq!(read(&password.stdin_line()).unwrap(), password);
        assert_eq!(
            read(&[b"x".repeat(MAX_PASSWORD_BYTES), b"\n".to_vec()].concat())
                .unwrap()
                .expose()
                .len(),
            MAX_PASSWORD_BYTES
        );
        for bad in [
            b"".to_vec(),
            b"\n".to_vec(),
            b"x".repeat(MAX_PASSWORD_BYTES + 1),
            [b"x".repeat(MAX_PASSWORD_BYTES + 1), b"\n".to_vec()].concat(),
            b"\xff\xfe\n".to_vec(),
            b"a\x07b\n".to_vec(),
        ] {
            assert!(read(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn authentication_matches_the_obs_websocket_v5_formula() {
        // Computed independently with Python's hashlib and base64.
        assert_eq!(
            authentication_string(&ObsPassword("supersecretpassword".into()), SALT, CHALLENGE),
            "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4="
        );
        // RFC 6455 section 1.3 example.
        assert_eq!(
            expected_accept("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    // ---- Handshake and request against the fake server ----

    #[test]
    fn switches_scene_without_authentication() {
        let (endpoint, server) = serve(obs_server(None, json!({"result":true})));
        client(endpoint, None)
            .set_current_program_scene("Live \"cam\" 🎥 {x}")
            .unwrap();
        let seen = server.join().unwrap();
        let header = |name: &str| {
            seen.headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(header("host"), Some(endpoint.to_string().as_str()));
        assert_eq!(header("sec-websocket-protocol"), Some("obswebsocket.json"));
        assert_eq!(header("sec-websocket-version"), Some("13"));
        assert_eq!(
            BASE64
                .decode(header("sec-websocket-key").unwrap())
                .unwrap()
                .len(),
            16
        );
        assert_eq!(
            seen.messages[0],
            json!({"op":1,"d":{"rpcVersion":1,"eventSubscriptions":0}})
        );
        let request = &seen.messages[1];
        assert_eq!(request["op"], 6);
        assert_eq!(request["d"]["requestType"], "SetCurrentProgramScene");
        assert_eq!(
            request["d"]["requestData"],
            json!({"sceneName":"Live \"cam\" 🎥 {x}"})
        );
        // The client closes with a normal-closure frame.
        assert_eq!(
            seen.frames,
            [(OPCODE_CLOSE, 1000u16.to_be_bytes().to_vec())]
        );
    }

    #[test]
    fn switches_scene_with_challenge_authentication() {
        let (endpoint, server) = serve(obs_server(Some(PASSWORD), json!({"result":true})));
        client(endpoint, Some(PASSWORD))
            .set_current_program_scene("Scene")
            .unwrap();
        let seen = server.join().unwrap();
        let identify = &seen.messages[0];
        assert_eq!(identify["d"]["eventSubscriptions"], 0);
        assert_eq!(
            identify["d"]["authentication"],
            json!(authentication_string(
                &ObsPassword(PASSWORD.into()),
                SALT,
                CHALLENGE
            ))
        );
        // Neither the password nor anything derived from it beyond the
        // challenge answer goes over the wire.
        for message in &seen.messages {
            assert_no_secret(&message.to_string());
        }
    }

    #[test]
    fn a_wrong_password_fails_without_echoing_it() {
        let (endpoint, server) = serve(obs_server(Some("the-real-one"), json!({"result":true})));
        let error = client(endpoint, Some(PASSWORD))
            .set_current_program_scene("Scene")
            .unwrap_err();
        server.join().unwrap();
        assert!(error.contains("authentication failed"), "{error}");
        assert!(
            !error.contains("Authentication failed."),
            "close reason shown: {error}"
        );
        assert_no_secret(&error);
    }

    #[test]
    fn a_required_password_that_is_missing_fails_before_identify() {
        let (endpoint, server) = serve(|peer, seen| {
            peer.accept_upgrade();
            peer.send_json(hello(Some((CHALLENGE, SALT))));
            if let Some(frame) = peer.read_frame_lenient() {
                seen.frames.push(frame);
            }
        });
        let error = client(endpoint, None)
            .set_current_program_scene("Scene")
            .unwrap_err();
        assert!(error.contains("requires a WebSocket password"), "{error}");
        let seen = server.join().unwrap();
        assert!(
            seen.frames
                .iter()
                .all(|(opcode, _)| *opcode == OPCODE_CLOSE),
            "no Identify was sent: {seen:?}"
        );
    }

    #[test]
    fn a_refused_request_reports_a_sanitized_bounded_reason() {
        // A request without data cannot echo a name, so OBS's comment is
        // shown, cleaned of control and invisible characters and cut short.
        let comment = format!(
            "Output is not running\u{1b}[31m\u{202E}\n{}",
            "x".repeat(500)
        );
        let (endpoint, server) = serve(scripted_obs(vec![(
            json!({"result":false,"code":501,"comment":comment}),
            None,
        )]));
        let error = client(endpoint, None)
            .request(&ObsRequest::new("StopRecord"))
            .unwrap_err();
        server.join().unwrap();
        assert!(error.contains("the output is not running"), "{error}");
        assert!(error.contains("Output is not running"), "{error}");
        assert!(!error.chars().any(|c| c.is_control()), "{error:?}");
        assert!(!error.contains('\u{202E}'), "{error:?}");
        assert!(error.chars().count() < 250, "{error}");
        assert!(error.ends_with('…'));
    }

    #[test]
    fn a_refused_request_that_carried_names_never_shows_obs_comment_text() {
        // OBS repeats the name it was given; the comment is dropped for any
        // request that carried data, so a name cannot reach logs through it.
        let (endpoint, server) = serve(obs_server(
            None,
            json!({"result":false,"code":600,"comment":"No scene was found by the name of Private call"}),
        ));
        let error = client(endpoint, None)
            .set_current_program_scene("Private call")
            .unwrap_err();
        server.join().unwrap();
        assert!(error.contains("no scene with that name"), "{error}");
        assert!(!error.contains("Private call"), "{error}");
        assert!(!error.contains("No scene was found"), "{error}");
    }

    #[test]
    fn pings_and_events_before_the_response_are_handled() {
        let (endpoint, server) = serve(|peer, seen| {
            peer.accept_upgrade();
            peer.send_frame(0x89, b"hb");
            peer.send_json(hello(None));
            // The client answers the ping while it waits for Hello.
            seen.frames.push(peer.read_frame().unwrap());
            seen.messages.push(peer.read_json());
            peer.send_json(json!({"op":2,"d":{"negotiatedRpcVersion":1}}));
            let request = peer.read_json();
            peer.send_json(json!({"op":5,"d":{"eventType":"X","eventIntent":1}}));
            peer.send_frame(0x8A, b"");
            peer.send_json(json!({"op":7,"d":{"requestType":"SetCurrentProgramScene","requestId":request["d"]["requestId"],"requestStatus":{"result":true,"code":100}}}));
        });
        client(endpoint, None)
            .set_current_program_scene("Scene")
            .unwrap();
        let seen = server.join().unwrap();
        assert_eq!(seen.frames[0], (OPCODE_PONG, b"hb".to_vec()));
        assert_eq!(seen.messages[0]["op"], 1);
    }

    // ---- Generic requests and sessions ----

    /// A well-behaved unauthenticated OBS that answers the next requests in
    /// order with `(status, responseData)`, then waits for the client to
    /// close. The requests it read are in `seen.messages` after the Identify.
    fn scripted_obs(
        answers: Vec<(Value, Option<Value>)>,
    ) -> impl FnOnce(&mut Peer, &mut Seen) + Send + 'static {
        move |peer, seen| {
            peer.accept_upgrade();
            peer.send_json(hello(None));
            seen.messages.push(peer.read_json());
            peer.send_json(json!({"op":2,"d":{"negotiatedRpcVersion":1}}));
            for (mut status, data) in answers {
                let request = peer.read_json();
                seen.messages.push(request.clone());
                if status["result"] == json!(true) {
                    status["code"] = json!(100);
                }
                let mut d = json!({
                    "requestType":request["d"]["requestType"],
                    "requestId":request["d"]["requestId"],
                    "requestStatus":status,
                });
                if let Some(data) = data {
                    d["responseData"] = data;
                }
                peer.send_json(json!({"op":7,"d":d}));
            }
        }
    }

    #[test]
    fn a_request_returns_its_response_data_and_omits_absent_request_data() {
        let (endpoint, server) = serve(scripted_obs(vec![(
            json!({"result":true}),
            Some(json!({"outputActive":true,"outputPaused":false})),
        )]));
        let data = client(endpoint, None)
            .request(&ObsRequest::new("GetRecordStatus"))
            .unwrap();
        assert_eq!(data, json!({"outputActive":true,"outputPaused":false}));
        let seen = server.join().unwrap();
        assert_eq!(seen.messages[1]["d"]["requestType"], "GetRecordStatus");
        assert!(seen.messages[1]["d"].get("requestData").is_none());
        assert_eq!(
            seen.frames,
            [(OPCODE_CLOSE, 1000u16.to_be_bytes().to_vec())]
        );
    }

    #[test]
    fn a_response_without_data_is_null() {
        let (endpoint, server) = serve(scripted_obs(vec![(json!({"result":true}), None)]));
        let data = client(endpoint, None)
            .request(&ObsRequest::new("ToggleRecord"))
            .unwrap();
        assert_eq!(data, Value::Null);
        server.join().unwrap();
    }

    #[test]
    fn a_session_runs_read_then_write_on_one_connection() {
        let (endpoint, server) = serve(scripted_obs(vec![
            (
                json!({"result":true}),
                Some(json!({"studioModeEnabled":false})),
            ),
            (json!({"result":true}), None),
        ]));
        client(endpoint, None)
            .session(|session| {
                let current = session.request(&ObsRequest::new("GetStudioModeEnabled"))?;
                let enabled = current["studioModeEnabled"].as_bool().ok_or("no flag")?;
                session.request(&ObsRequest::with_data(
                    "SetStudioModeEnabled",
                    json!({"studioModeEnabled":!enabled}),
                ))
            })
            .unwrap();
        let seen = server.join().unwrap();
        // One Identify, then both requests, on the one connection.
        let ops: Vec<_> = seen.messages.iter().map(|m| m["op"].clone()).collect();
        assert_eq!(ops, [json!(1), json!(6), json!(6)]);
        assert_eq!(
            seen.messages[2]["d"]["requestData"],
            json!({"studioModeEnabled":true})
        );
        assert_ne!(
            seen.messages[1]["d"]["requestId"],
            seen.messages[2]["d"]["requestId"]
        );
        assert_eq!(seen.frames.len(), 1, "one close frame: {seen:?}");
    }

    #[test]
    fn a_failed_request_ends_the_session_and_later_requests_are_not_sent() {
        let (endpoint, server) = serve(scripted_obs(vec![(
            json!({"result":false,"code":501,"comment":"Output is not running."}),
            None,
        )]));
        let error = client(endpoint, None)
            .session(|session| {
                session.request(&ObsRequest::new("StopRecord"))?;
                session.request(&ObsRequest::new("ToggleRecord"))
            })
            .unwrap_err();
        assert!(error.starts_with("OBS at 127.0.0.1:"), "{error}");
        assert!(error.contains("refused StopRecord"), "{error}");
        assert!(error.contains("the output is not running"), "{error}");
        let seen = server.join().unwrap();
        assert_eq!(seen.messages.len(), 2, "no second request: {seen:?}");
    }

    #[test]
    fn refusal_text_for_other_requests_is_generic_and_scene_switches_keep_theirs() {
        let (endpoint, server) = serve(scripted_obs(vec![(
            json!({"result":false,"code":600}),
            None,
        )]));
        let error = client(endpoint, None)
            .request(&ObsRequest::with_data(
                "SetInputMute",
                json!({"inputName":"Mic","inputMuted":true}),
            ))
            .unwrap_err();
        server.join().unwrap();
        assert!(
            error.contains("refused SetInputMute (not found)"),
            "{error}"
        );
        assert!(!error.contains("scene"), "{error}");
        assert!(!error.contains("Mic"), "{error}");
        assert_eq!(
            refusal_reason("SetCurrentProgramScene", 600),
            "no scene with that name"
        );
        assert_eq!(refusal_reason("X", 9999), "code 9999");
    }

    #[test]
    fn a_response_for_another_request_type_or_id_is_rejected() {
        let wrong_type = |peer: &mut Peer, _: &mut Seen| {
            peer.accept_upgrade();
            peer.send_json(hello(None));
            peer.read_json();
            peer.send_json(json!({"op":2,"d":{"negotiatedRpcVersion":1}}));
            let request = peer.read_json();
            peer.send_json(json!({"op":7,"d":{"requestType":"ToggleStream","requestId":request["d"]["requestId"],"requestStatus":{"result":true,"code":100}}}));
        };
        let (endpoint, server) = serve(wrong_type);
        let error = client(endpoint, None)
            .request(&ObsRequest::new("ToggleRecord"))
            .unwrap_err();
        server.join().unwrap();
        assert!(error.contains("answered a different request"), "{error}");

        let wrong_id = |peer: &mut Peer, _: &mut Seen| {
            peer.accept_upgrade();
            peer.send_json(hello(None));
            peer.read_json();
            peer.send_json(json!({"op":2,"d":{"negotiatedRpcVersion":1}}));
            peer.read_json();
            peer.send_json(json!({"op":7,"d":{"requestType":"ToggleRecord","requestId":"someone-else","requestStatus":{"result":true,"code":100}}}));
        };
        let (endpoint, server) = serve(wrong_id);
        let error = client(endpoint, None)
            .request(&ObsRequest::new("ToggleRecord"))
            .unwrap_err();
        server.join().unwrap();
        assert!(error.contains("answered a different request"), "{error}");
    }

    #[test]
    fn one_deadline_covers_every_request_in_a_session() {
        // The first answer arrives; the second never does.
        let (endpoint, server) = serve(|peer, _| {
            peer.accept_upgrade();
            peer.send_json(hello(None));
            peer.read_json();
            peer.send_json(json!({"op":2,"d":{"negotiatedRpcVersion":1}}));
            let request = peer.read_json();
            peer.send_json(json!({"op":7,"d":{"requestType":"GetRecordStatus","requestId":request["d"]["requestId"],"requestStatus":{"result":true,"code":100}}}));
            peer.read_json();
            thread::sleep(Duration::from_millis(1500));
        });
        let started = Instant::now();
        let error = client(endpoint, None)
            .session(|session| {
                session.request(&ObsRequest::new("GetRecordStatus"))?;
                session.request(&ObsRequest::new("ToggleRecord"))
            })
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(1400), "{error}");
        assert!(
            error.contains("timed out") || error.contains("deadline"),
            "{error}"
        );
        server.join().unwrap();
    }

    #[test]
    fn cancelling_between_requests_stops_the_session() {
        let flag = Arc::new(AtomicBool::new(false));
        let (endpoint, server) = serve(|peer, _| {
            peer.accept_upgrade();
            peer.send_json(hello(None));
            peer.read_json();
            peer.send_json(json!({"op":2,"d":{"negotiatedRpcVersion":1}}));
            let request = peer.read_json();
            peer.send_json(json!({"op":7,"d":{"requestType":"GetRecordStatus","requestId":request["d"]["requestId"],"requestStatus":{"result":true,"code":100}}}));
            let _ = peer.read_frame_lenient();
            thread::sleep(Duration::from_millis(1500));
        });
        let started = Instant::now();
        let error = ObsClient::new(endpoint, None)
            .with_cancellation(flag.clone())
            .session(|session| {
                session.request(&ObsRequest::new("GetRecordStatus"))?;
                flag.store(true, Ordering::SeqCst);
                session.request(&ObsRequest::new("ToggleRecord"))
            })
            .unwrap_err();
        assert!(started.elapsed() < Duration::from_millis(1000), "{error}");
        assert!(
            error.contains("cancel") || error.contains("stopp"),
            "{error}"
        );
        server.join().unwrap();
    }

    // ---- Multi-request actions (`ObsClient::run`) ----

    use crate::profile::ObsSwitchOp;

    fn ok(data: Value) -> (Value, Option<Value>) {
        (json!({"result":true}), Some(data))
    }

    fn done() -> (Value, Option<Value>) {
        (json!({"result":true}), None)
    }

    /// The `(requestType, requestData)` pairs OBS saw after the Identify.
    fn requests(seen: &Seen) -> Vec<(String, Value)> {
        seen.messages[1..]
            .iter()
            .map(|message| {
                (
                    message["d"]["requestType"].as_str().unwrap().to_string(),
                    message["d"]["requestData"].clone(),
                )
            })
            .collect()
    }

    #[test]
    fn studio_mode_toggle_reads_then_writes_the_opposite_and_on_off_write_directly() {
        for (op, answers, expected) in [
            (
                ObsSwitchOp::Toggle,
                vec![ok(json!({"studioModeEnabled":false})), done()],
                vec![
                    ("GetStudioModeEnabled", Value::Null),
                    ("SetStudioModeEnabled", json!({"studioModeEnabled":true})),
                ],
            ),
            (
                ObsSwitchOp::Toggle,
                vec![ok(json!({"studioModeEnabled":true})), done()],
                vec![
                    ("GetStudioModeEnabled", Value::Null),
                    ("SetStudioModeEnabled", json!({"studioModeEnabled":false})),
                ],
            ),
            (
                ObsSwitchOp::On,
                vec![done()],
                vec![("SetStudioModeEnabled", json!({"studioModeEnabled":true}))],
            ),
            (
                ObsSwitchOp::Off,
                vec![done()],
                vec![("SetStudioModeEnabled", json!({"studioModeEnabled":false}))],
            ),
        ] {
            let (endpoint, server) = serve(scripted_obs(answers));
            client(endpoint, None)
                .run(&Action::ObsStudioMode { op })
                .unwrap();
            let seen = server.join().unwrap();
            let expected: Vec<_> = expected
                .into_iter()
                .map(|(kind, data)| (kind.to_string(), data))
                .collect();
            assert_eq!(requests(&seen), expected, "{op:?}");
        }
    }

    #[test]
    fn the_transition_is_one_request_and_is_refused_outside_studio_mode() {
        let (endpoint, server) = serve(scripted_obs(vec![done()]));
        client(endpoint, None).run(&Action::ObsTransition).unwrap();
        let seen = server.join().unwrap();
        assert_eq!(
            requests(&seen),
            [("TriggerStudioModeTransition".to_string(), Value::Null)]
        );
        let (endpoint, server) = serve(scripted_obs(vec![(
            json!({"result":false,"code":506}),
            None,
        )]));
        let error = client(endpoint, None)
            .run(&Action::ObsTransition)
            .unwrap_err();
        server.join().unwrap();
        assert!(error.contains("studio mode is not active"), "{error}");
    }

    #[test]
    fn collection_and_profile_switches_are_one_request_with_specific_refusals() {
        let (endpoint, server) = serve(scripted_obs(vec![done()]));
        client(endpoint, None)
            .run(&Action::ObsSceneCollection {
                name: "Show \"A\"".into(),
            })
            .unwrap();
        let seen = server.join().unwrap();
        assert_eq!(
            requests(&seen),
            [(
                "SetCurrentSceneCollection".to_string(),
                json!({"sceneCollectionName":"Show \"A\""})
            )]
        );
        let (endpoint, server) = serve(scripted_obs(vec![done()]));
        client(endpoint, None)
            .run(&Action::ObsProfile {
                name: "Work".into(),
            })
            .unwrap();
        let seen = server.join().unwrap();
        assert_eq!(
            requests(&seen),
            [(
                "SetCurrentProfile".to_string(),
                json!({"profileName":"Work"})
            )]
        );
        for (action, reason) in [
            (
                Action::ObsSceneCollection {
                    name: "Secret show".into(),
                },
                "no scene collection with that name",
            ),
            (
                Action::ObsProfile {
                    name: "Secret".into(),
                },
                "no profile with that name",
            ),
        ] {
            let (endpoint, server) = serve(scripted_obs(vec![(
                json!({"result":false,"code":600,"comment":"No profile called Secret"}),
                None,
            )]));
            let error = client(endpoint, None).run(&action).unwrap_err();
            server.join().unwrap();
            assert!(error.contains(reason), "{error}");
            assert!(!error.contains("Secret"), "{error}");
        }
    }

    #[test]
    fn a_filter_toggle_reads_the_filter_and_sets_the_opposite() {
        let action = |op| Action::ObsFilter {
            source: "Webcam \"HD\"".into(),
            filter: "Color {x}".into(),
            op,
        };
        let names = json!({"sourceName":"Webcam \"HD\"", "filterName":"Color {x}"});
        let (endpoint, server) = serve(scripted_obs(vec![
            ok(json!({"filterEnabled":true})),
            done(),
        ]));
        client(endpoint, None)
            .run(&action(ObsSwitchOp::Toggle))
            .unwrap();
        let seen = server.join().unwrap();
        assert_eq!(
            requests(&seen),
            [
                ("GetSourceFilter".to_string(), names.clone()),
                (
                    "SetSourceFilterEnabled".to_string(),
                    json!({"sourceName":"Webcam \"HD\"", "filterName":"Color {x}", "filterEnabled":false})
                ),
            ]
        );
        let (endpoint, server) = serve(scripted_obs(vec![done()]));
        client(endpoint, None)
            .run(&action(ObsSwitchOp::On))
            .unwrap();
        let seen = server.join().unwrap();
        assert_eq!(requests(&seen).len(), 1);
        assert_eq!(requests(&seen)[0].0, "SetSourceFilterEnabled");
    }

    #[test]
    fn source_visibility_looks_up_the_item_id_then_reads_and_writes_it() {
        let action = |op| Action::ObsSourceVisibility {
            scene: "Live".into(),
            source: "Camera".into(),
            op,
        };
        let (endpoint, server) = serve(scripted_obs(vec![
            ok(json!({"sceneItemId":7})),
            ok(json!({"sceneItemEnabled":true})),
            done(),
        ]));
        client(endpoint, None)
            .run(&action(ObsSwitchOp::Toggle))
            .unwrap();
        let seen = server.join().unwrap();
        assert_eq!(
            requests(&seen),
            [
                (
                    "GetSceneItemId".to_string(),
                    json!({"sceneName":"Live", "sourceName":"Camera"})
                ),
                (
                    "GetSceneItemEnabled".to_string(),
                    json!({"sceneName":"Live", "sceneItemId":7})
                ),
                (
                    "SetSceneItemEnabled".to_string(),
                    json!({"sceneName":"Live", "sceneItemId":7, "sceneItemEnabled":false})
                ),
            ]
        );
        // `on` needs the id but not the current state.
        let (endpoint, server) = serve(scripted_obs(vec![ok(json!({"sceneItemId":7})), done()]));
        client(endpoint, None)
            .run(&action(ObsSwitchOp::On))
            .unwrap();
        let seen = server.join().unwrap();
        let kinds: Vec<_> = requests(&seen).into_iter().map(|(kind, _)| kind).collect();
        assert_eq!(kinds, ["GetSceneItemId", "SetSceneItemEnabled"]);
    }

    #[test]
    fn a_missing_source_stops_before_any_write_and_names_the_cause_without_the_name() {
        let (endpoint, server) = serve(scripted_obs(vec![(
            json!({"result":false,"code":600,"comment":"No scene item was found by Secret cam"}),
            None,
        )]));
        let error = client(endpoint, None)
            .run(&Action::ObsSourceVisibility {
                scene: "Live".into(),
                source: "Secret cam".into(),
                op: ObsSwitchOp::Off,
            })
            .unwrap_err();
        let seen = server.join().unwrap();
        assert_eq!(requests(&seen).len(), 1, "no write after a failed lookup");
        assert!(
            error.contains("no such scene, or no such source in it"),
            "{error}"
        );
        assert!(
            !error.contains("Secret cam") && !error.contains("Live"),
            "{error}"
        );
    }

    #[test]
    fn a_response_without_the_expected_field_fails_instead_of_guessing() {
        let (endpoint, server) = serve(scripted_obs(vec![ok(json!({"somethingElse":true}))]));
        let error = client(endpoint, None)
            .run(&Action::ObsStudioMode {
                op: ObsSwitchOp::Toggle,
            })
            .unwrap_err();
        let seen = server.join().unwrap();
        assert!(error.contains("no studioModeEnabled"), "{error}");
        assert_eq!(
            requests(&seen).len(),
            1,
            "no write without knowing the state"
        );
    }

    #[test]
    fn run_rejects_actions_that_do_not_talk_to_obs_without_connecting() {
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        closed.set_nonblocking(true).unwrap();
        let endpoint = ObsEndpoint::new("127.0.0.1", closed.local_addr().unwrap().port()).unwrap();
        for action in [
            Action::ToggleMute,
            Action::MultiAction {
                actions: vec![Action::ToggleMute, Action::ObsTransition],
            },
        ] {
            let error = client(endpoint, None).run(&action).unwrap_err();
            assert!(error.contains("does not talk to OBS"), "{error}");
        }
        assert!(closed.accept().is_err(), "OBS was contacted");
    }

    fn expect_failure(script: impl FnOnce(&mut Peer, &mut Seen) + Send + 'static, expected: &str) {
        let (endpoint, server) = serve(script);
        let started = Instant::now();
        let error = client(endpoint, Some(PASSWORD))
            .set_current_program_scene("Scene")
            .unwrap_err();
        assert!(
            started.elapsed() < FAST + Duration::from_millis(400),
            "{error}"
        );
        assert!(
            error.contains(expected),
            "expected {expected:?} in {error:?}"
        );
        assert!(
            error.starts_with(&format!("OBS at {endpoint}: ")),
            "{error}"
        );
        assert_no_secret(&error);
        let _ = server.join();
    }

    fn after_hello(frame: Vec<u8>) -> impl FnOnce(&mut Peer, &mut Seen) + Send + 'static {
        move |peer, _| {
            peer.accept_upgrade();
            peer.send_raw(&frame);
        }
    }

    #[test]
    fn malformed_frames_are_rejected() {
        for (frame, expected) in [
            (
                vec![0x81, 0x82, 1, 2, 3, 4, b'{' ^ 1, b'}' ^ 2],
                "masked frame",
            ),
            (vec![0xC1, 0x02, b'{', b'}'], "reserved bits"),
            (vec![0x82, 0x02, b'{', b'}'], "binary frame"),
            (vec![0x01, 0x02, b'{', b'}'], "fragmented"),
            (vec![0x80, 0x02, b'{', b'}'], "fragmented"),
            (vec![0x83, 0x00], "reserved opcode"),
            (vec![0x8B, 0x00], "reserved opcode"),
            (vec![0x81, 0x02, 0xFF, 0xFE], "not UTF-8"),
            (vec![0x81, 0x03, b'a', b'b', b'c'], "not obs-websocket JSON"),
            (
                vec![0x81, 126, 0x00, 0x05, b'1', b'2', b'3', b'4', b'5'],
                "non-minimal",
            ),
            (
                [vec![0x89, 126, 0x00, 126], vec![0; 126]].concat(),
                "control frame longer",
            ),
        ] {
            expect_failure(after_hello(frame.clone()), expected);
        }
    }

    #[test]
    fn oversized_frames_are_rejected_before_their_payload_is_read() {
        let mut header = vec![0x81, 127];
        header.extend_from_slice(&((MAX_FRAME_BYTES as u64) + 1).to_be_bytes());
        expect_failure(after_hello(header), "larger than 65536 bytes");
        let mut huge = vec![0x81, 127];
        huge.extend_from_slice(&u64::MAX.to_be_bytes());
        expect_failure(after_hello(huge), "larger than 65536 bytes");
        // Exactly the limit is accepted as a frame (and then fails as JSON).
        let payload = vec![b' '; MAX_FRAME_BYTES];
        let mut frame = vec![0x81, 127];
        frame.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        frame.extend_from_slice(&payload);
        expect_failure(after_hello(frame), "not obs-websocket JSON");
    }

    #[test]
    fn handshake_responses_are_checked_strictly() {
        let respond = |response: &'static str| {
            move |peer: &mut Peer, _: &mut Seen| {
                peer.read_request();
                peer.send_raw(response.as_bytes());
            }
        };
        expect_failure(
            respond("HTTP/1.1 302 Found\r\nLocation: http://192.168.1.2:4455/\r\n\r\n"),
            "HTTP status 302",
        );
        expect_failure(respond("HTTP/1.1 200 OK\r\n\r\n"), "HTTP status 200");
        expect_failure(respond("SSH-2.0-OpenSSH\r\n\r\n"), "HTTP status unknown");
        expect_failure(
            respond(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: AAAAAAAAAAAAAAAAAAAAAAAAAAA=\r\n\r\n",
            ),
            "wrong Sec-WebSocket-Accept",
        );
        expect_failure(
            respond("HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\n\r\n"),
            "missing Upgrade",
        );
        let with_header = |extra: &'static str| {
            move |peer: &mut Peer, _: &mut Seen| {
                let headers = peer.read_request();
                let key = &headers
                    .iter()
                    .find(|(name, _)| name == "sec-websocket-key")
                    .unwrap()
                    .1;
                peer.send_raw(
                    format!(
                        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n{extra}\r\n",
                        expected_accept(key)
                    )
                    .as_bytes(),
                );
            }
        };
        expect_failure(
            with_header("Sec-WebSocket-Extensions: permessage-deflate\r\n"),
            "extension",
        );
        expect_failure(
            with_header("Sec-WebSocket-Protocol: obswebsocket.msgpack\r\n"),
            "subprotocol",
        );
        expect_failure(with_header("Upgrade: websocket\r\n"), "repeats a header");
        expect_failure(
            move |peer: &mut Peer, _: &mut Seen| {
                peer.read_request();
                peer.send_raw(b"HTTP/1.1 101 Switching Protocols\r\n");
                peer.send_raw(&vec![b'a'; MAX_HANDSHAKE_BYTES + 10]);
            },
            "exceeds 8192 bytes",
        );
    }

    #[test]
    fn protocol_violations_after_the_upgrade_fail() {
        expect_failure(
            |peer, _| {
                peer.accept_upgrade();
                peer.send_json(json!({"op":2,"d":{"negotiatedRpcVersion":1}}));
            },
            "expected OBS Hello",
        );
        expect_failure(
            |peer, _| {
                peer.accept_upgrade();
                peer.send_json(json!({"op":0,"d":{"rpcVersion":"1"}}));
            },
            "Hello is malformed",
        );
        expect_failure(
            |peer, _| {
                peer.accept_upgrade();
                peer.send_json(hello(Some((&"c".repeat(300), SALT))));
            },
            "challenge is too long",
        );
        expect_failure(
            |peer, _| {
                peer.accept_upgrade();
                peer.send_json(hello(None));
                peer.read_json();
                peer.send_json(json!({"op":2,"d":{"negotiatedRpcVersion":2}}));
            },
            "RPC version 1",
        );
        expect_failure(
            |peer, _| {
                peer.accept_upgrade();
                peer.send_json(hello(None));
                peer.read_json();
                peer.send_json(json!({"op":2,"d":{"negotiatedRpcVersion":1}}));
                peer.read_json();
                peer.send_json(json!({"op":7,"d":{"requestType":"SetCurrentProgramScene","requestId":"someone-else","requestStatus":{"result":true,"code":100}}}));
            },
            "different request",
        );
        expect_failure(
            |peer, _| {
                peer.accept_upgrade();
                peer.send_json(hello(None));
                peer.read_json();
                peer.close(4010, "nope");
            },
            "RPC version 1",
        );
    }

    #[test]
    fn a_server_closing_early_fails_quickly() {
        expect_failure(|_, _| {}, "closed the connection early");
        expect_failure(
            |peer, _| {
                peer.accept_upgrade();
            },
            "closed the connection early",
        );
        expect_failure(
            |peer, _| {
                peer.accept_upgrade();
                peer.send_raw(&[0x81, 0x10, b'{']);
            },
            "closed the connection early",
        );
    }

    #[test]
    fn a_silent_server_times_out_within_the_deadline() {
        let (release, wait) = mpsc::channel::<()>();
        let (endpoint, server) = serve(move |peer, _| {
            peer.read_request();
            let _ = wait.recv_timeout(Duration::from_secs(5));
        });
        let started = Instant::now();
        let error = client(endpoint, None)
            .set_current_program_scene("Scene")
            .unwrap_err();
        let elapsed = started.elapsed();
        release.send(()).unwrap();
        server.join().unwrap();
        assert!(error.contains("timed out"), "{error}");
        assert!(
            elapsed >= FAST && elapsed < FAST + Duration::from_millis(400),
            "{elapsed:?}"
        );
        // The default deadline is the documented 3 s and cannot be raised.
        let endpoint = ObsEndpoint::parse("127.0.0.1:4455").unwrap();
        assert_eq!(ObsClient::new(endpoint, None).timeout, DEFAULT_TIMEOUT);
        assert_eq!(
            ObsClient::new(endpoint, None)
                .with_timeout(Duration::from_secs(60))
                .timeout,
            DEFAULT_TIMEOUT
        );
    }

    #[test]
    fn nothing_listening_fails_without_waiting() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let error = client(ObsEndpoint::new("127.0.0.1", port).unwrap(), None)
            .set_current_program_scene("Scene")
            .unwrap_err();
        assert!(error.contains("is OBS running"), "{error}");
    }

    #[test]
    fn cancellation_stops_before_connecting() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let error = client(ObsEndpoint::new("127.0.0.1", port).unwrap(), None)
            .with_cancellation(Arc::new(AtomicBool::new(true)))
            .set_current_program_scene("Scene")
            .unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert!(listener.accept().is_err(), "no connection was made");
    }

    #[test]
    fn ipv6_loopback_and_localhost_work() {
        // Skipped where the host has no IPv6 loopback.
        if let Some((endpoint, server)) =
            serve_on("[::1]:0", obs_server(None, json!({"result":true})))
        {
            assert_eq!(endpoint.host(), "::1");
            client(endpoint, None)
                .set_current_program_scene("Scene")
                .unwrap();
            let seen = server.join().unwrap();
            assert!(
                seen.headers
                    .iter()
                    .any(|(name, value)| name == "host" && value.starts_with("[::1]:"))
            );
        }
        let (endpoint, server) = serve(obs_server(None, json!({"result":true})));
        let localhost = ObsEndpoint::new("localhost", endpoint.port()).unwrap();
        client(localhost, None)
            .set_current_program_scene("Scene")
            .unwrap();
        server.join().unwrap();
    }

    #[test]
    fn client_frames_are_masked_and_sized_correctly() {
        let mask = [1, 2, 3, 4];
        let frame = encode_frame(OPCODE_TEXT, b"abc", mask).unwrap();
        assert_eq!(frame[..6], [0x81, 0x83, 1, 2, 3, 4]);
        assert_eq!(frame[6..], [b'a' ^ 1, b'b' ^ 2, b'c' ^ 3]);
        let long = encode_frame(OPCODE_TEXT, &[0; 300], mask).unwrap();
        assert_eq!(long[..4], [0x81, 0x80 | 126, 0x01, 0x2C]);
        assert_eq!(long.len(), 4 + 4 + 300);
        assert!(encode_frame(OPCODE_TEXT, &vec![0; MAX_FRAME_BYTES + 1], mask).is_err());
        // Masking keys are not constant.
        assert_ne!(random_bytes::<4>(), random_bytes::<4>());
    }

    #[test]
    fn sanitize_bounds_untrusted_text() {
        assert_eq!(sanitize("plain"), "plain");
        assert_eq!(sanitize("a\nb\u{200B}c"), "a\u{FFFD}b\u{FFFD}c");
        let long = sanitize(&"é".repeat(500));
        assert_eq!(long.chars().count(), MAX_ERROR_CHARS + 1);
    }
}
