#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, missing_docs)]

//! Application orchestration and local progress persistence.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use learn_challenge::{CompiledChallenge, ContentPack, compile_directory};
use learn_core::{
    ChallengeId, ChallengeProgress, CompletionStatus, GraphError, Skill, SkillGraph, SkillId,
    TrackId, VerifierExpr,
};
use learn_docs::{DocsIndex, SearchResult, starter_index};
use learn_protocol::{
    Envelope, HelloResponse, PROTOCOL_VERSION, Response, StatusResponse, encode_frame, envelope,
    request, response,
};
use learn_runtime::{DoctorReport, WasmSession, start_wasm_session};
use prost::Message;
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::sync::{Mutex as AsyncMutex, mpsc};

/// JSON-backed progress file used by the first local daemon release.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ProgressState {
    /// Per-challenge progress.
    pub challenges: Vec<ChallengeProgress>,
}

const PROGRESS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("progress");

/// Durable progress store.
#[derive(Debug)]
pub struct ProgressStore {
    database: Database,
    state: ProgressState,
}

impl ProgressStore {
    /// Open or initialize a progress file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DaemonError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(DaemonError::Io)?;
        }
        let database =
            Database::create(&path).map_err(|error| DaemonError::Storage(error.to_string()))?;
        let state = {
            let transaction = database
                .begin_write()
                .map_err(|error| DaemonError::Storage(error.to_string()))?;
            let state = {
                let table = transaction
                    .open_table(PROGRESS_TABLE)
                    .map_err(|error| DaemonError::Storage(error.to_string()))?;
                table
                    .get("state")
                    .map_err(|error| DaemonError::Storage(error.to_string()))?
                    .map(|value| serde_json::from_slice(value.value()).map_err(DaemonError::Json))
                    .transpose()?
                    .unwrap_or_default()
            };
            transaction
                .commit()
                .map_err(|error| DaemonError::Storage(error.to_string()))?;
            state
        };
        Ok(Self { database, state })
    }

    /// Mark a challenge as started.
    pub fn start(&mut self, id: ChallengeId) -> Result<(), DaemonError> {
        let progress = self
            .state
            .challenges
            .iter_mut()
            .find(|progress| progress.challenge_id == id);
        if let Some(progress) = progress {
            progress.attempts = progress.attempts.saturating_add(1);
            progress.status = CompletionStatus::InProgress;
        } else {
            self.state.challenges.push(ChallengeProgress {
                challenge_id: id,
                status: CompletionStatus::InProgress,
                attempts: 1,
                hints_used: Vec::new(),
            });
        }
        self.save()
    }

    /// Mark a challenge as complete.
    pub fn complete(&mut self, id: &ChallengeId) -> Result<(), DaemonError> {
        if let Some(progress) = self
            .state
            .challenges
            .iter_mut()
            .find(|progress| &progress.challenge_id == id)
        {
            progress.status = CompletionStatus::Completed;
        } else {
            self.state.challenges.push(ChallengeProgress {
                challenge_id: id.clone(),
                status: CompletionStatus::Completed,
                attempts: 1,
                hints_used: Vec::new(),
            });
        }
        self.save()
    }

    /// Reset one challenge without touching other progress.
    pub fn reset(&mut self, id: &ChallengeId) -> Result<(), DaemonError> {
        self.state
            .challenges
            .retain(|progress| &progress.challenge_id != id);
        self.save()
    }

    /// Borrow all progress.
    pub fn all(&self) -> &[ChallengeProgress] {
        &self.state.challenges
    }

    fn save(&self) -> Result<(), DaemonError> {
        let json = serde_json::to_vec_pretty(&self.state).map_err(DaemonError::Json)?;
        let transaction = self
            .database
            .begin_write()
            .map_err(|error| DaemonError::Storage(error.to_string()))?;
        {
            let mut table = transaction
                .open_table(PROGRESS_TABLE)
                .map_err(|error| DaemonError::Storage(error.to_string()))?;
            table
                .insert("state", json.as_slice())
                .map_err(|error| DaemonError::Storage(error.to_string()))?;
        }
        transaction
            .commit()
            .map_err(|error| DaemonError::Storage(error.to_string()))
    }
}

/// Shared read-only daemon data.
#[derive(Clone, Debug)]
pub struct Daemon {
    /// Compiled content.
    pub content: Arc<ContentPack>,
    /// Local docs index.
    pub docs: Arc<DocsIndex>,
    /// Runtime report.
    pub runtime: DoctorReport,
    /// Derived skill graph.
    pub graph: Arc<SkillGraph>,
    progress: Arc<std::sync::Mutex<Option<ProgressStore>>>,
    sessions: Arc<std::sync::Mutex<HashMap<String, ManagedSession>>>,
}

#[derive(Debug)]
struct ManagedSession {
    challenge_id: String,
    verifier: VerifierExpr,
    session: Arc<std::sync::Mutex<WasmSession>>,
}

impl Daemon {
    /// Load a daemon from repository content.
    pub fn load(content_directory: impl AsRef<Path>) -> Result<Self, DaemonError> {
        let content = compile_directory(content_directory).map_err(DaemonError::Content)?;
        let graph = build_skill_graph(&content)?;
        Ok(Self {
            content: Arc::new(content),
            docs: Arc::new(starter_index()),
            runtime: learn_runtime::portable_doctor(),
            graph: Arc::new(graph),
            progress: Arc::new(std::sync::Mutex::new(None)),
            sessions: Arc::new(std::sync::Mutex::new(HashMap::new())),
        })
    }

    /// Return a snapshot of all persisted learner progress.
    pub fn progress(&self) -> Result<Vec<ChallengeProgress>, DaemonError> {
        self.with_progress(|progress| progress.all().to_vec())
    }

    /// Record a challenge attempt.
    pub fn start_challenge(&self, id: ChallengeId) -> Result<(), DaemonError> {
        self.with_progress_mut(|progress| progress.start(id))
    }

    /// Record a verified challenge completion.
    pub fn complete_challenge(&self, id: &ChallengeId) -> Result<(), DaemonError> {
        self.with_progress_mut(|progress| progress.complete(id))
    }

    /// Reset one challenge's progress.
    pub fn reset_challenge(&self, id: &ChallengeId) -> Result<(), DaemonError> {
        self.with_progress_mut(|progress| progress.reset(id))
    }

    fn with_progress<T>(
        &self,
        operation: impl FnOnce(&ProgressStore) -> T,
    ) -> Result<T, DaemonError> {
        let mut progress = self
            .progress
            .lock()
            .map_err(|_| DaemonError::Storage("progress store is poisoned".to_owned()))?;
        if progress.is_none() {
            *progress = Some(open_progress_store()?);
        }
        Ok(operation(
            progress.as_ref().expect("progress store initialized"),
        ))
    }

    fn with_progress_mut<T>(
        &self,
        operation: impl FnOnce(&mut ProgressStore) -> Result<T, DaemonError>,
    ) -> Result<T, DaemonError> {
        let mut progress = self
            .progress
            .lock()
            .map_err(|_| DaemonError::Storage("progress store is poisoned".to_owned()))?;
        if progress.is_none() {
            *progress = Some(open_progress_store()?);
        }
        operation(progress.as_mut().expect("progress store initialized"))
    }

    /// Search the local docs.
    pub fn search_docs(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        self.docs.search(query, limit)
    }

    /// Find a challenge by stable ID.
    pub fn challenge(&self, id: &str) -> Option<&CompiledChallenge> {
        self.content.get(id)
    }

    /// Serve typed requests over a local Unix socket until the process is stopped.
    pub async fn serve(&self, socket: impl AsRef<Path>) -> Result<(), DaemonError> {
        let socket = socket.as_ref();
        if let Some(parent) = socket.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(DaemonError::Io)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                        .map_err(DaemonError::Io)?;
                }
            }
        }
        if socket.exists() {
            fs::remove_file(socket).map_err(DaemonError::Io)?;
        }
        let listener = UnixListener::bind(socket).map_err(DaemonError::Io)?;
        loop {
            let (stream, _) = listener.accept().await.map_err(DaemonError::Io)?;
            let daemon = self.clone();
            tokio::spawn(async move {
                if let Err(error) = daemon.handle_connection(stream).await {
                    tracing::debug!(%error, "daemon client disconnected");
                }
            });
        }
    }

    /// Serve the browser bridge on a loopback WebSocket listener.
    ///
    /// The bridge translates JSON messages to the same typed request handlers
    /// used by the Unix-socket TUI. It is intentionally loopback-only.
    pub async fn serve_websocket(&self, address: SocketAddr) -> Result<(), DaemonError> {
        let listener = TcpListener::bind(address).await.map_err(DaemonError::Io)?;
        loop {
            let (stream, _) = listener.accept().await.map_err(DaemonError::Io)?;
            let daemon = self.clone();
            tokio::spawn(async move {
                if let Err(error) = daemon.handle_websocket(stream).await {
                    tracing::debug!(%error, "web bridge client disconnected");
                }
            });
        }
    }

    async fn handle_websocket(&self, stream: TcpStream) -> Result<(), DaemonError> {
        let (mut reader, writer) = stream.into_split();
        let handshake = read_websocket_handshake(&mut reader).await?;
        if !handshake.target_is_websocket() {
            return Err(DaemonError::Protocol("web bridge expects /ws".to_owned()));
        }
        let Some(key) = handshake.header("sec-websocket-key") else {
            return Err(DaemonError::Protocol(
                "websocket handshake omitted Sec-WebSocket-Key".to_owned(),
            ));
        };
        let accept = websocket_accept_key(key);
        let writer = Arc::new(AsyncMutex::new(writer));
        write_http_handshake(&writer, &accept).await?;

        let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
        let event_writer = Arc::clone(&writer);
        let event_task = tokio::spawn(async move {
            while let Some(event) = event_receiver.recv().await {
                let Some(payload) = websocket_event_json(&event) else {
                    continue;
                };
                if write_websocket_text(&event_writer, &payload).await.is_err() {
                    break;
                }
            }
        });

        let result = loop {
            let Some((opcode, payload)) = read_websocket_frame(&mut reader).await? else {
                break Ok(());
            };
            match opcode {
                0x1 => {
                    let request: WebRequest = serde_json::from_slice(&payload)
                        .map_err(|error| DaemonError::Protocol(error.to_string()))?;
                    let request_type = request.kind.clone();
                    let request_id = request.request_id;
                    let envelope =
                        bridge_request_envelope(&request).map_err(DaemonError::Protocol)?;
                    let (response, _) = self.handle_request(envelope, &event_sender);
                    let json = self.bridge_response_json(request_id, &request_type, &response);
                    write_websocket_text(&writer, &json.to_string()).await?;
                }
                0x8 => break Ok(()),
                0x9 => write_websocket_frame(&writer, 0xA, &payload).await?,
                _ => {}
            }
        };
        drop(event_sender);
        event_task.abort();
        result
    }

    fn bridge_response_json(
        &self,
        request_id: u64,
        request_type: &str,
        envelope: &Envelope,
    ) -> serde_json::Value {
        let Some(envelope::Body::Response(response)) = envelope.body.as_ref() else {
            return serde_json::json!({
                "type": "response",
                "requestId": request_id,
                "ok": false,
                "error": "daemon returned an invalid response envelope",
            });
        };
        if !response.ok {
            return serde_json::json!({
                "type": "response",
                "requestId": request_id,
                "ok": false,
                "error": response.error,
            });
        }
        let mut json = serde_json::json!({
            "type": "response",
            "requestId": request_id,
            "ok": true,
        });
        match (request_type, response.kind.as_ref()) {
            ("hello", Some(response::Kind::Hello(hello))) => {
                json["snapshot"] = self.web_snapshot(&hello.server_version);
            }
            ("open_session", Some(response::Kind::Challenge(challenge))) => {
                json["session"] = serde_json::json!({
                    "id": challenge.session_id,
                    "challengeId": challenge.id,
                    "title": challenge.title,
                    "objective": challenge.objective,
                });
            }
            ("search_docs", Some(response::Kind::Docs(docs))) => {
                json["results"] = serde_json::Value::Array(
                    docs.results
                        .iter()
                        .map(|result| {
                            serde_json::json!({
                                "id": result.id,
                                "title": result.title,
                                "snippet": result.snippet,
                            })
                        })
                        .collect(),
                );
            }
            ("status", Some(response::Kind::Status(status))) => {
                json["status"] = serde_json::json!({
                    "challengeCount": status.challenge_count,
                    "completedCount": status.completed_count,
                    "runtimeSummary": status.runtime_summary,
                });
            }
            _ => {}
        }
        json
    }

    fn web_snapshot(&self, server_version: &str) -> serde_json::Value {
        let progress = self.progress().unwrap_or_default();
        let completed_count = progress
            .iter()
            .filter(|item| item.status == CompletionStatus::Completed)
            .count();
        let challenges = self
            .content
            .challenges
            .iter()
            .enumerate()
            .map(|(index, challenge)| {
                let challenge_id = challenge.spec.id.to_string();
                let saved = progress
                    .iter()
                    .find(|item| item.challenge_id.as_str() == challenge_id);
                let state = saved.map_or("available", |item| match item.status {
                    CompletionStatus::Completed => "completed",
                    CompletionStatus::InProgress => "in_progress",
                    CompletionStatus::Locked => "available",
                });
                serde_json::json!({
                    "id": challenge_id,
                    "title": challenge.spec.meta.title,
                    "objective": challenge.spec.objective.text,
                    "track": challenge.spec.skills.teaches.first().map_or("FOUNDATIONS", |track| track.as_str()).to_ascii_uppercase(),
                    "lesson": format!("{:02} / {:02}", index + 1, self.content.challenges.len()),
                    "state": state,
                    "attempts": saved.map_or(0, |item| item.attempts),
                    "hintCount": challenge.spec.hints.len(),
                })
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "protocolVersion": PROTOCOL_VERSION,
            "serverVersion": server_version,
            "runtime": {
                "ready": self.runtime.usable(),
                "summary": self.runtime.render(),
                "challengeCount": self.content.challenges.len(),
                "completedCount": completed_count,
            },
            "challenges": challenges,
            "progress": progress.iter().map(|item| serde_json::json!({
                "challengeId": item.challenge_id,
                "state": match item.status {
                    CompletionStatus::Completed => "completed",
                    CompletionStatus::InProgress => "in_progress",
                    CompletionStatus::Locked => "available",
                },
                "attempts": item.attempts,
                "hintsUsed": item.hints_used.len(),
            })).collect::<Vec<_>>(),
        })
    }

    async fn handle_connection(&self, stream: UnixStream) -> Result<(), DaemonError> {
        let (mut reader, writer) = stream.into_split();
        let writer = Arc::new(AsyncMutex::new(writer));
        let (event_sender, mut event_receiver) = mpsc::unbounded_channel();
        let event_writer = Arc::clone(&writer);
        let event_task = tokio::spawn(async move {
            while let Some(event) = event_receiver.recv().await {
                if write_envelope(&event_writer, &event).await.is_err() {
                    break;
                }
            }
        });
        let mut owned_sessions = Vec::new();
        let result = loop {
            let envelope = match read_envelope(&mut reader).await {
                Ok(envelope) => envelope,
                Err(DaemonError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    break Ok(());
                }
                Err(error) => break Err(error),
            };
            let (response, opened_session) = self.handle_request(envelope, &event_sender);
            if let Some(session_id) = opened_session {
                owned_sessions.push(session_id);
            }
            if let Err(error) = write_envelope(&writer, &response).await {
                break Err(error);
            }
        };
        for session_id in owned_sessions {
            self.close_session(&session_id);
        }
        drop(event_sender);
        event_task.abort();
        result
    }

    #[allow(clippy::too_many_lines)]
    fn handle_request(
        &self,
        envelope: Envelope,
        event_sender: &mpsc::UnboundedSender<Envelope>,
    ) -> (Envelope, Option<String>) {
        let message_id = envelope.message_id;
        if envelope.protocol_version != PROTOCOL_VERSION {
            return (
                error_response(message_id, "unsupported protocol version"),
                None,
            );
        }
        let Some(envelope::Body::Request(request)) = envelope.body else {
            return (
                error_response(message_id, "expected a request envelope"),
                None,
            );
        };
        let Some(kind) = request.kind else {
            return (error_response(message_id, "request has no kind"), None);
        };
        let mut opened_session = None;
        let response = match kind {
            request::Kind::Hello(_) => Response {
                ok: true,
                error: String::new(),
                kind: Some(response::Kind::Hello(HelloResponse {
                    server_version: env!("CARGO_PKG_VERSION").to_owned(),
                    protocol_version: PROTOCOL_VERSION,
                    runtime_available: self.runtime.usable(),
                    content_version: format!("{} challenges", self.content.challenges.len()),
                })),
            },
            request::Kind::Status(_) => Response {
                ok: true,
                error: String::new(),
                kind: Some(response::Kind::Status(StatusResponse {
                    challenge_count: u32::try_from(self.content.challenges.len())
                        .unwrap_or(u32::MAX),
                    completed_count: u32::try_from(
                        self.progress()
                            .map(|progress| {
                                progress
                                    .iter()
                                    .filter(|item| item.status == CompletionStatus::Completed)
                                    .count()
                            })
                            .unwrap_or_default(),
                    )
                    .unwrap_or(u32::MAX),
                    runtime_summary: self.runtime.render(),
                })),
            },
            request::Kind::SearchDocs(search) => Response {
                ok: true,
                error: String::new(),
                kind: Some(response::Kind::Docs(learn_protocol::DocsResponse {
                    results: self
                        .search_docs(&search.query, usize::try_from(search.limit).unwrap_or(10))
                        .into_iter()
                        .map(|result| learn_protocol::DocResult {
                            id: result.document.id,
                            title: result.document.title,
                            snippet: result.snippet,
                        })
                        .collect(),
                })),
            },
            request::Kind::Graph(_) => Response {
                ok: true,
                error: String::new(),
                kind: Some(response::Kind::Graph(learn_protocol::GraphResponse {
                    skills: self
                        .graph
                        .skills()
                        .map(|skill| learn_protocol::SkillNode {
                            id: skill.id.to_string(),
                            title: skill.title.clone(),
                            prerequisites: skill
                                .prerequisites
                                .iter()
                                .map(ToString::to_string)
                                .collect(),
                        })
                        .collect(),
                })),
            },
            request::Kind::OpenSession(open) => {
                self.open_request(&open, event_sender, &mut opened_session)
            }
            request::Kind::TerminalInput(input) => {
                if let Err(error) = self.write_terminal_input(&input, event_sender) {
                    error_response_payload(&error)
                } else {
                    Response {
                        ok: true,
                        error: String::new(),
                        kind: None,
                    }
                }
            }
            request::Kind::TerminalResize(resize) => {
                if let Err(error) = self.resize_terminal(&resize) {
                    error_response_payload(&error)
                } else {
                    Response {
                        ok: true,
                        error: String::new(),
                        kind: None,
                    }
                }
            }
            request::Kind::CloseSession(close) => {
                if self.close_session(&close.session_id) {
                    Response {
                        ok: true,
                        error: String::new(),
                        kind: None,
                    }
                } else {
                    error_response_payload("unknown session")
                }
            }
            request::Kind::Reset(reset) => {
                if self.challenge(&reset.challenge_id).is_none() {
                    error_response_payload("unknown challenge")
                } else {
                    self.reset_sessions(&reset.challenge_id);
                    match ChallengeId::new(reset.challenge_id) {
                        Ok(id) => match self.reset_challenge(&id) {
                            Ok(()) => Response {
                                ok: true,
                                error: String::new(),
                                kind: None,
                            },
                            Err(error) => error_response_payload(&error.to_string()),
                        },
                        Err(error) => error_response_payload(&error.to_string()),
                    }
                }
            }
        };
        (
            Envelope {
                protocol_version: PROTOCOL_VERSION,
                message_id,
                body: Some(envelope::Body::Response(response)),
            },
            opened_session,
        )
    }

    fn open_request(
        &self,
        open: &learn_protocol::OpenSessionRequest,
        event_sender: &mpsc::UnboundedSender<Envelope>,
        opened_session: &mut Option<String>,
    ) -> Response {
        if !self.runtime.usable() {
            return error_response_payload(
                "embedded WASM runtime is unavailable; run learn doctor for details",
            );
        }
        match self.open_session(open, event_sender) {
            Ok(response) => {
                *opened_session = Some(response.session_id.clone());
                Response {
                    ok: true,
                    error: String::new(),
                    kind: Some(response::Kind::Challenge(response)),
                }
            }
            Err(error) => error_response_payload(&error),
        }
    }

    fn open_session(
        &self,
        open: &learn_protocol::OpenSessionRequest,
        event_sender: &mpsc::UnboundedSender<Envelope>,
    ) -> Result<learn_protocol::ChallengeResponse, String> {
        let challenge = self
            .challenge(&open.challenge_id)
            .ok_or_else(|| "unknown challenge".to_owned())?
            .clone();
        let columns = u16::try_from(open.columns).unwrap_or(100).max(1);
        let rows = u16::try_from(open.rows).unwrap_or(30).max(1);
        let mut session = start_wasm_session(
            challenge.spec.id.as_str(),
            &challenge.spec.setup,
            &challenge.spec.environment,
            columns,
            rows,
        )
        .map_err(|error| error.to_string())?;
        let session_id = session.id.clone();
        let mut reader = session
            .take_reader()
            .ok_or_else(|| "sandbox PTY reader was unavailable".to_owned())?;
        let managed = ManagedSession {
            challenge_id: challenge.spec.id.to_string(),
            verifier: challenge.spec.verifier.clone(),
            session: Arc::new(std::sync::Mutex::new(session)),
        };
        self.start_challenge(challenge.spec.id.clone())
            .map_err(|error| error.to_string())?;
        self.sessions
            .lock()
            .map_err(|_| "session table is poisoned".to_owned())?
            .insert(session_id.clone(), managed);
        let output_sender = event_sender.clone();
        let output_session_id = session_id.clone();
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        let event = Envelope {
                            protocol_version: PROTOCOL_VERSION,
                            message_id: 0,
                            body: Some(envelope::Body::Event(learn_protocol::Event {
                                kind: Some(learn_protocol::event::Kind::TerminalOutput(
                                    learn_protocol::TerminalOutput {
                                        session_id: output_session_id.clone(),
                                        data: buffer[..count].to_vec(),
                                    },
                                )),
                            })),
                        };
                        if output_sender.send(event).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Ok(learn_protocol::ChallengeResponse {
            id: challenge.spec.id.to_string(),
            title: challenge.spec.meta.title,
            objective: challenge.spec.objective.text,
            session_id,
        })
    }

    fn write_terminal_input(
        &self,
        input: &learn_protocol::TerminalInput,
        event_sender: &mpsc::UnboundedSender<Envelope>,
    ) -> Result<(), String> {
        let managed = self
            .sessions
            .lock()
            .map_err(|_| "session table is poisoned".to_owned())?
            .get(&input.session_id)
            .map(|managed| ManagedSession {
                challenge_id: managed.challenge_id.clone(),
                verifier: managed.verifier.clone(),
                session: Arc::clone(&managed.session),
            })
            .ok_or_else(|| "unknown session".to_owned())?;
        let session = managed
            .session
            .lock()
            .map_err(|_| "session is poisoned".to_owned())?;
        session
            .write(&input.data)
            .map_err(|error| error.to_string())?;
        if session
            .verify(&managed.verifier)
            .map_err(|error| error.to_string())?
        {
            let challenge_id = ChallengeId::new(managed.challenge_id.clone())
                .map_err(|error| error.to_string())?;
            self.complete_challenge(&challenge_id)
                .map_err(|error| error.to_string())?;
            let event = Envelope {
                protocol_version: PROTOCOL_VERSION,
                message_id: 0,
                body: Some(envelope::Body::Event(learn_protocol::Event {
                    kind: Some(learn_protocol::event::Kind::ChallengeState(
                        learn_protocol::ChallengeStateEvent {
                            challenge_id: managed.challenge_id,
                            state: "completed".to_owned(),
                        },
                    )),
                })),
            };
            let _ = event_sender.send(event);
        }
        Ok(())
    }

    fn resize_terminal(&self, resize: &learn_protocol::TerminalResize) -> Result<(), String> {
        let session = self
            .sessions
            .lock()
            .map_err(|_| "session table is poisoned".to_owned())?
            .get(&resize.session_id)
            .map(|managed| Arc::clone(&managed.session))
            .ok_or_else(|| "unknown session".to_owned())?;
        let session = session
            .lock()
            .map_err(|_| "session is poisoned".to_owned())?;
        session
            .resize(
                u16::try_from(resize.columns).unwrap_or(u16::MAX).max(1),
                u16::try_from(resize.rows).unwrap_or(u16::MAX).max(1),
            )
            .map_err(|error| error.to_string())
    }

    fn close_session(&self, session_id: &str) -> bool {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(session_id).is_some()
        } else {
            false
        }
    }

    fn reset_sessions(&self, challenge_id: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.retain(|_, managed| managed.challenge_id != challenge_id);
        }
    }
}

#[derive(Debug, Deserialize)]
struct WebRequest {
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "requestId")]
    request_id: u64,
    #[serde(rename = "challengeId")]
    challenge_id: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    client_kind: Option<String>,
    data: Option<String>,
    query: Option<String>,
    limit: Option<u32>,
    columns: Option<u32>,
    rows: Option<u32>,
}

fn bridge_request_envelope(request: &WebRequest) -> Result<Envelope, String> {
    use learn_protocol::{
        CloseSession, GraphRequest, HelloRequest, OpenSessionRequest, ResetRequest,
        SearchDocsRequest, StatusRequest, TerminalInput, TerminalResize,
    };
    let kind = match request.kind.as_str() {
        "hello" => request::Kind::Hello(HelloRequest {
            client_version: "web".to_owned(),
            client_kind: request
                .client_kind
                .clone()
                .unwrap_or_else(|| "web".to_owned()),
        }),
        "status" => request::Kind::Status(StatusRequest {}),
        "open_session" => request::Kind::OpenSession(OpenSessionRequest {
            challenge_id: required_field(&request.challenge_id, "challengeId")?,
            columns: request.columns.unwrap_or(100),
            rows: request.rows.unwrap_or(32),
        }),
        "terminal_input" => request::Kind::TerminalInput(TerminalInput {
            session_id: required_field(&request.session_id, "sessionId")?,
            data: request.data.clone().unwrap_or_default().into_bytes(),
        }),
        "terminal_resize" => request::Kind::TerminalResize(TerminalResize {
            session_id: required_field(&request.session_id, "sessionId")?,
            columns: request.columns.unwrap_or(100),
            rows: request.rows.unwrap_or(32),
        }),
        "close_session" => request::Kind::CloseSession(CloseSession {
            session_id: required_field(&request.session_id, "sessionId")?,
        }),
        "reset" => request::Kind::Reset(ResetRequest {
            challenge_id: required_field(&request.challenge_id, "challengeId")?,
        }),
        "search_docs" => request::Kind::SearchDocs(SearchDocsRequest {
            query: request.query.clone().unwrap_or_default(),
            limit: request.limit.unwrap_or(5),
        }),
        "graph" => request::Kind::Graph(GraphRequest {}),
        other => return Err(format!("unknown web request type `{other}`")),
    };
    Ok(Envelope {
        protocol_version: PROTOCOL_VERSION,
        message_id: request.request_id,
        body: Some(envelope::Body::Request(learn_protocol::Request {
            kind: Some(kind),
        })),
    })
}

fn required_field(value: &Option<String>, name: &str) -> Result<String, String> {
    value
        .clone()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing `{name}`"))
}

fn websocket_event_json(envelope: &Envelope) -> Option<String> {
    let Some(envelope::Body::Event(event)) = envelope.body.as_ref() else {
        return None;
    };
    let json = match event.kind.as_ref()? {
        learn_protocol::event::Kind::TerminalOutput(output) => serde_json::json!({
            "type": "terminal_output",
            "sessionId": output.session_id,
            "data": String::from_utf8_lossy(&output.data),
        }),
        learn_protocol::event::Kind::ChallengeState(state) => serde_json::json!({
            "type": "challenge_state",
            "challengeId": state.challenge_id,
            "state": state.state,
        }),
    };
    Some(json.to_string())
}

#[derive(Debug)]
struct WebSocketHandshake {
    request_line: String,
    headers: Vec<(String, String)>,
}

impl WebSocketHandshake {
    fn target_is_websocket(&self) -> bool {
        self.request_line
            .split_whitespace()
            .nth(1)
            .is_some_and(|target| target == "/ws")
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

async fn read_websocket_handshake(
    reader: &mut tokio::net::tcp::OwnedReadHalf,
) -> Result<WebSocketHandshake, DaemonError> {
    let mut bytes = Vec::with_capacity(1024);
    loop {
        let mut chunk = [0_u8; 1024];
        let count = reader.read(&mut chunk).await.map_err(DaemonError::Io)?;
        if count == 0 {
            return Err(DaemonError::Protocol(
                "websocket handshake ended early".to_owned(),
            ));
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
        if bytes.len() > 16 * 1024 {
            return Err(DaemonError::Protocol(
                "websocket handshake is too large".to_owned(),
            ));
        }
    }
    let request = String::from_utf8(bytes)
        .map_err(|_| DaemonError::Protocol("websocket handshake was not UTF-8".to_owned()))?;
    let mut lines = request.split("\r\n");
    let request_line = lines.next().unwrap_or_default().to_owned();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect();
    Ok(WebSocketHandshake {
        request_line,
        headers,
    })
}

async fn write_http_handshake(
    writer: &AsyncMutex<tokio::net::tcp::OwnedWriteHalf>,
    accept: &str,
) -> Result<(), DaemonError> {
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    writer
        .lock()
        .await
        .write_all(response.as_bytes())
        .await
        .map_err(DaemonError::Io)
}

async fn write_websocket_text(
    writer: &AsyncMutex<tokio::net::tcp::OwnedWriteHalf>,
    text: &str,
) -> Result<(), DaemonError> {
    write_websocket_frame(writer, 0x1, text.as_bytes()).await
}

async fn write_websocket_frame(
    writer: &AsyncMutex<tokio::net::tcp::OwnedWriteHalf>,
    opcode: u8,
    payload: &[u8],
) -> Result<(), DaemonError> {
    let mut frame = Vec::with_capacity(payload.len() + 10);
    frame.push(0x80 | (opcode & 0x0f));
    match payload.len() {
        length @ 0..=125 => frame.push(length as u8),
        length @ 126..=65_535 => {
            frame.push(126);
            frame.extend_from_slice(&(length as u16).to_be_bytes());
        }
        length => {
            frame.push(127);
            frame.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    writer
        .lock()
        .await
        .write_all(&frame)
        .await
        .map_err(DaemonError::Io)
}

async fn read_websocket_frame(
    reader: &mut tokio::net::tcp::OwnedReadHalf,
) -> Result<Option<(u8, Vec<u8>)>, DaemonError> {
    let mut header = [0_u8; 2];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(DaemonError::Io(error)),
    }
    let opcode = header[0] & 0x0f;
    if header[0] & 0x80 == 0 {
        return Err(DaemonError::Protocol(
            "fragmented websocket messages are unsupported".to_owned(),
        ));
    }
    let masked = header[1] & 0x80 != 0;
    let mut length = u64::from(header[1] & 0x7f);
    if length == 126 {
        let mut bytes = [0_u8; 2];
        reader
            .read_exact(&mut bytes)
            .await
            .map_err(DaemonError::Io)?;
        length = u64::from(u16::from_be_bytes(bytes));
    } else if length == 127 {
        let mut bytes = [0_u8; 8];
        reader
            .read_exact(&mut bytes)
            .await
            .map_err(DaemonError::Io)?;
        length = u64::from_be_bytes(bytes);
    }
    if length > learn_protocol::MAX_FRAME_SIZE as u64 {
        return Err(DaemonError::Protocol(
            "websocket payload exceeds 8 MiB limit".to_owned(),
        ));
    }
    let mut mask = [0_u8; 4];
    if masked {
        reader
            .read_exact(&mut mask)
            .await
            .map_err(DaemonError::Io)?;
    }
    let mut payload = vec![0_u8; length as usize];
    reader
        .read_exact(&mut payload)
        .await
        .map_err(DaemonError::Io)?;
    if masked {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    Ok(Some((opcode, payload)))
}

fn websocket_accept_key(key: &str) -> String {
    let mut input = key.as_bytes().to_vec();
    input.extend_from_slice(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    BASE64.encode(sha1_digest(&input))
}

fn sha1_digest(input: &[u8]) -> [u8; 20] {
    let mut message = input.to_vec();
    let bit_length = (message.len() as u64).saturating_mul(8);
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_length.to_be_bytes());
    let mut state = [
        0x6745_2301_u32,
        0xEFCD_AB89,
        0x98BA_DCFE,
        0x1032_5476,
        0xC3D2_E1F0,
    ];
    for chunk in message.chunks_exact(64) {
        let mut words = [0_u32; 80];
        for (index, bytes) in chunk.chunks_exact(4).enumerate() {
            words[index] = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) =
            (state[0], state[1], state[2], state[3], state[4]);
        for (index, word) in words.iter().enumerate() {
            let (function, constant) = match index {
                0..=19 => ((b & c) | ((!b) & d), 0x5A82_7999),
                20..=39 => (b ^ c ^ d, 0x6ED9_EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC),
                _ => (b ^ c ^ d, 0xCA62_C1D6),
            };
            let temporary = a
                .rotate_left(5)
                .wrapping_add(function)
                .wrapping_add(e)
                .wrapping_add(constant)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temporary;
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
    }
    let mut digest = [0_u8; 20];
    for (index, word) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

async fn read_envelope(reader: &mut OwnedReadHalf) -> Result<Envelope, DaemonError> {
    let frame_length = reader.read_u32().await.map_err(DaemonError::Io)? as usize;
    if frame_length > learn_protocol::MAX_FRAME_SIZE {
        return Err(DaemonError::Protocol(
            "frame exceeds 8 MiB limit".to_owned(),
        ));
    }
    let mut payload = vec![0; frame_length];
    reader
        .read_exact(&mut payload)
        .await
        .map_err(DaemonError::Io)?;
    Envelope::decode(payload.as_slice()).map_err(|error| DaemonError::Protocol(error.to_string()))
}

async fn write_envelope(
    writer: &AsyncMutex<OwnedWriteHalf>,
    envelope: &Envelope,
) -> Result<(), DaemonError> {
    let frame = encode_frame(envelope).map_err(|error| DaemonError::Protocol(error.to_string()))?;
    writer
        .lock()
        .await
        .write_all(&frame)
        .await
        .map_err(DaemonError::Io)
}

/// A client connection to the local daemon.
pub struct DaemonClient {
    reader: OwnedReadHalf,
    writer: OwnedWriteHalf,
    next_message_id: u64,
    pending_events: std::collections::VecDeque<Envelope>,
}

impl std::fmt::Debug for DaemonClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DaemonClient")
            .field("next_message_id", &self.next_message_id)
            .field("pending_events", &self.pending_events.len())
            .finish_non_exhaustive()
    }
}

impl DaemonClient {
    /// Connect to a daemon Unix socket.
    pub async fn connect(socket: impl AsRef<Path>) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket).await.map_err(ClientError::Io)?;
        let (reader, writer) = stream.into_split();
        Ok(Self {
            reader,
            writer,
            next_message_id: 1,
            pending_events: std::collections::VecDeque::new(),
        })
    }

    /// Perform the version handshake.
    pub async fn hello(&mut self) -> Result<HelloResponse, ClientError> {
        let response = self
            .request(request::Kind::Hello(learn_protocol::HelloRequest {
                client_version: env!("CARGO_PKG_VERSION").to_owned(),
                client_kind: "tui".to_owned(),
            }))
            .await?;
        match response.kind {
            Some(response::Kind::Hello(hello)) => Ok(hello),
            _ => Err(ClientError::Protocol(
                "hello response has the wrong payload".to_owned(),
            )),
        }
    }

    /// Open a challenge session owned by this connection.
    pub async fn open_session(
        &mut self,
        challenge_id: &str,
        columns: u16,
        rows: u16,
    ) -> Result<learn_protocol::ChallengeResponse, ClientError> {
        let response = self
            .request(request::Kind::OpenSession(
                learn_protocol::OpenSessionRequest {
                    challenge_id: challenge_id.to_owned(),
                    columns: u32::from(columns),
                    rows: u32::from(rows),
                },
            ))
            .await?;
        match response.kind {
            Some(response::Kind::Challenge(challenge)) => Ok(challenge),
            _ => Err(ClientError::Protocol(
                "open-session response has the wrong payload".to_owned(),
            )),
        }
    }

    /// Send raw terminal bytes to an active session.
    pub async fn send_terminal_input(
        &mut self,
        session_id: &str,
        data: Vec<u8>,
    ) -> Result<(), ClientError> {
        self.request(request::Kind::TerminalInput(
            learn_protocol::TerminalInput {
                session_id: session_id.to_owned(),
                data,
            },
        ))
        .await
        .map(|_| ())
    }

    /// Resize an active terminal session.
    pub async fn resize_terminal(
        &mut self,
        session_id: &str,
        columns: u16,
        rows: u16,
    ) -> Result<(), ClientError> {
        self.request(request::Kind::TerminalResize(
            learn_protocol::TerminalResize {
                session_id: session_id.to_owned(),
                columns: u32::from(columns),
                rows: u32::from(rows),
            },
        ))
        .await
        .map(|_| ())
    }

    /// Close an active terminal session.
    pub async fn close_session(&mut self, session_id: &str) -> Result<(), ClientError> {
        self.request(request::Kind::CloseSession(learn_protocol::CloseSession {
            session_id: session_id.to_owned(),
        }))
        .await
        .map(|_| ())
    }

    /// Search the daemon's local documentation index.
    pub async fn search_docs(
        &mut self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<learn_protocol::DocResult>, ClientError> {
        let response = self
            .request(request::Kind::SearchDocs(
                learn_protocol::SearchDocsRequest {
                    query: query.to_owned(),
                    limit: u32::try_from(limit).unwrap_or(u32::MAX),
                },
            ))
            .await?;
        match response.kind {
            Some(response::Kind::Docs(docs)) => Ok(docs.results),
            _ => Err(ClientError::Protocol(
                "docs response has the wrong payload".to_owned(),
            )),
        }
    }

    /// Read one queued daemon event, waiting up to `timeout` milliseconds.
    pub async fn poll_event(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<Option<Envelope>, ClientError> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(Some(event));
        }
        let envelope = match tokio::time::timeout(timeout, read_envelope(&mut self.reader)).await {
            Ok(result) => result.map_err(ClientError::from)?,
            Err(_) => return Ok(None),
        };
        if envelope.protocol_version != PROTOCOL_VERSION {
            return Err(ClientError::Protocol(
                "daemon event used an unsupported protocol version".to_owned(),
            ));
        }
        if matches!(envelope.body, Some(envelope::Body::Event(_))) {
            Ok(Some(envelope))
        } else {
            Err(ClientError::Protocol(
                "daemon sent a non-event while polling".to_owned(),
            ))
        }
    }

    async fn request(&mut self, kind: request::Kind) -> Result<Response, ClientError> {
        let message_id = self.next_message_id;
        self.next_message_id = self.next_message_id.saturating_add(1);
        let envelope = Envelope {
            protocol_version: PROTOCOL_VERSION,
            message_id,
            body: Some(envelope::Body::Request(learn_protocol::Request {
                kind: Some(kind),
            })),
        };
        let frame = encode_frame(&envelope).map_err(ClientError::Frame)?;
        self.writer
            .write_all(&frame)
            .await
            .map_err(ClientError::Io)?;
        loop {
            let received = read_envelope(&mut self.reader)
                .await
                .map_err(ClientError::from)?;
            if matches!(received.body, Some(envelope::Body::Event(_))) {
                self.pending_events.push_back(received);
                continue;
            }
            if received.message_id != message_id {
                return Err(ClientError::Protocol(
                    "daemon response message ID did not match request".to_owned(),
                ));
            }
            if received.protocol_version != PROTOCOL_VERSION {
                return Err(ClientError::Protocol(
                    "daemon response used an unsupported protocol version".to_owned(),
                ));
            }
            let Some(envelope::Body::Response(response)) = received.body else {
                return Err(ClientError::Protocol(
                    "daemon response has the wrong envelope body".to_owned(),
                ));
            };
            if response.ok {
                return Ok(response);
            }
            return Err(ClientError::Server(response.error));
        }
    }
}

/// Failure while communicating with the daemon client.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Socket I/O failed.
    #[error("daemon client I/O failed: {0}")]
    Io(#[source] io::Error),
    /// Protocol framing failed.
    #[error(transparent)]
    Frame(#[from] learn_protocol::FrameError),
    /// The daemon returned an application error.
    #[error("daemon rejected the request: {0}")]
    Server(String),
    /// The client or server violated the protocol contract.
    #[error("daemon protocol error: {0}")]
    Protocol(String),
}

impl From<DaemonError> for ClientError {
    fn from(error: DaemonError) -> Self {
        match error {
            DaemonError::Io(error) => Self::Io(error),
            other => Self::Protocol(other.to_string()),
        }
    }
}

fn error_response(message_id: u64, message: &str) -> Envelope {
    Envelope {
        protocol_version: PROTOCOL_VERSION,
        message_id,
        body: Some(envelope::Body::Response(error_response_payload(message))),
    }
}

fn error_response_payload(message: &str) -> Response {
    Response {
        ok: false,
        error: message.to_owned(),
        kind: None,
    }
}

fn build_skill_graph(content: &ContentPack) -> Result<SkillGraph, DaemonError> {
    let track = TrackId::new("foundation")?;
    let mut requirements = std::collections::BTreeMap::<SkillId, Vec<SkillId>>::new();
    for challenge in &content.challenges {
        for taught in &challenge.spec.skills.teaches {
            let taught = SkillId::new(taught.clone())?;
            let prerequisites = challenge
                .spec
                .skills
                .requires
                .iter()
                .map(|required| SkillId::new(required.clone()))
                .collect::<Result<Vec<_>, _>>()?;
            requirements
                .entry(taught)
                .or_default()
                .extend(prerequisites);
        }
    }
    let mut graph = SkillGraph::new();
    for (id, prerequisites) in requirements {
        let mut prerequisites = prerequisites;
        prerequisites.sort();
        prerequisites.dedup();
        graph.insert(Skill {
            title: id
                .as_str()
                .rsplit('.')
                .next()
                .unwrap_or(id.as_str())
                .replace('-', " "),
            description: format!("Practice {} through short shell challenges.", id.as_str()),
            id,
            prerequisites,
            track: track.clone(),
        })?;
    }
    graph.validate()?;
    Ok(graph)
}

/// Return the configured local daemon socket path.
pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|directory| runtime_directory_is_writable(directory))
        .unwrap_or_else(|| PathBuf::from(format!("/tmp/learn-{}", std::process::id())))
        .join("learn/daemon.sock")
}

/// Return the loopback address used by the browser WebSocket bridge.
pub fn websocket_address() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 8787))
}

fn runtime_directory_is_writable(directory: &Path) -> bool {
    if !directory.is_dir() {
        return false;
    }
    let probe = directory.join(format!(".learn-probe-{}", std::process::id()));
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

/// Return the durable local progress database path.
pub fn progress_path() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map_or_else(
            || {
                std::env::var_os("HOME")
                    .map_or_else(|| PathBuf::from("."), PathBuf::from)
                    .join(".local/state")
            },
            PathBuf::from,
        )
        .join("learn/progress.redb")
}

fn open_progress_store() -> Result<ProgressStore, DaemonError> {
    match ProgressStore::open(progress_path()) {
        Ok(progress) => Ok(progress),
        Err(DaemonError::Io(error))
            if matches!(
                error.kind(),
                io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
            ) =>
        {
            ProgressStore::open(
                std::env::temp_dir()
                    .join(format!("learn-{}", userspace_id()))
                    .join("progress.redb"),
            )
        }
        Err(DaemonError::Storage(error))
            if error.to_ascii_lowercase().contains("read-only")
                || error.to_ascii_lowercase().contains("permission denied") =>
        {
            ProgressStore::open(
                std::env::temp_dir()
                    .join(format!("learn-{}", userspace_id()))
                    .join("progress.redb"),
            )
        }
        Err(error) => Err(error),
    }
}

fn userspace_id() -> String {
    std::env::var("USER")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| std::process::id().to_string())
}

/// Daemon failure.
#[derive(Debug, Error)]
pub enum DaemonError {
    /// Content compile error.
    #[error(transparent)]
    Content(#[from] learn_challenge::CompileError),
    /// Invalid graph identifier.
    #[error(transparent)]
    Identifier(#[from] learn_core::IdentifierError),
    /// Filesystem error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON persistence error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// Embedded database failure.
    #[error("storage error: {0}")]
    Storage(String),
    /// Invalid graph content.
    #[error(transparent)]
    Graph(#[from] GraphError),
    /// Protocol failure.
    #[error("protocol error: {0}")]
    Protocol(String),
    /// Protobuf encode failure.
    #[error(transparent)]
    Encode(#[from] prost::EncodeError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_survives_reopen_and_reset_is_scoped() {
        let path = std::env::temp_dir().join(format!(
            "learn-progress-test-{}-{}.redb",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        let first = ChallengeId::new("first").expect("ID");
        let second = ChallengeId::new("second").expect("ID");
        {
            let mut store = ProgressStore::open(&path).expect("open");
            store.start(first.clone()).expect("start");
            store.complete(&first).expect("complete");
            store.start(second.clone()).expect("start");
        }
        {
            let mut store = ProgressStore::open(&path).expect("reopen");
            assert_eq!(store.all().len(), 2);
            assert_eq!(store.all()[0].status, CompletionStatus::Completed);
            store.reset(&first).expect("reset");
            assert_eq!(store.all().len(), 1);
            assert_eq!(store.all()[0].challenge_id, second);
        }
        let _ = fs::remove_file(path);
    }

    #[test]
    fn websocket_accept_key_matches_rfc_example() {
        assert_eq!(
            websocket_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    #[tokio::test]
    async fn daemon_answers_a_typed_handshake() {
        let content_path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/challenges");
        let daemon = Daemon::load(content_path).expect("content");
        let directory = std::env::temp_dir().join(format!(
            "learn-daemon-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("test directory");
        let socket = directory.join("daemon.sock");
        match std::os::unix::net::UnixListener::bind(&socket) {
            Ok(listener) => drop(listener),
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                println!("skipping Unix socket test: environment denies AF_UNIX bind");
                let _ = fs::remove_dir_all(directory);
                return;
            }
            Err(error) => panic!("socket preflight: {error}"),
        }
        let server = tokio::spawn({
            let daemon = daemon.clone();
            let socket = socket.clone();
            async move { daemon.serve(socket).await }
        });
        for _ in 0..20 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mut client = DaemonClient::connect(&socket).await.expect("connect");
        let hello = client.hello().await.expect("hello");
        assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
        assert_eq!(hello.content_version, "10 challenges");
        assert!(hello.runtime_available);
        let docs = client
            .search_docs("serch text in fiel", 5)
            .await
            .expect("docs");
        assert!(docs.iter().any(|result| result.id == "command.grep"));
        let opened = client
            .open_session("foundation.orientation", 80, 24)
            .await
            .expect("open simulated session");
        client
            .send_terminal_input(&opened.session_id, b"cat ~/welcome.txt\r".to_vec())
            .await
            .expect("send simulated command");
        let mut completed = false;
        for _ in 0..20 {
            if let Some(event) = client
                .poll_event(std::time::Duration::from_millis(25))
                .await
                .expect("poll event")
                && let Some(envelope::Body::Event(event)) = event.body
                && let Some(learn_protocol::event::Kind::ChallengeState(state)) = event.kind
                && state.state == "completed"
            {
                completed = true;
                break;
            }
        }
        assert!(completed, "simulated canonical command completed");
        client
            .close_session(&opened.session_id)
            .await
            .expect("close simulated session");
        server.abort();
        let _ = fs::remove_dir_all(directory);
    }
}
