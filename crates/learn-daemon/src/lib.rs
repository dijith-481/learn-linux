#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, missing_docs)]

//! Application orchestration and local progress persistence.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use learn_challenge::{CompiledChallenge, ContentPack, compile_builtin, compile_directory};
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
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
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
        Self::from_content(content)
    }

    /// Load the self-contained built-in lessons.
    pub fn load_builtin() -> Result<Self, DaemonError> {
        let content = compile_builtin().map_err(DaemonError::Content)?;
        Self::from_content(content)
    }

    fn from_content(content: ContentPack) -> Result<Self, DaemonError> {
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

    /// Return persisted progress for the currently bundled lessons.
    pub fn progress(&self) -> Result<Vec<ChallengeProgress>, DaemonError> {
        let current_challenges = self
            .content
            .challenges
            .iter()
            .map(|challenge| challenge.spec.id.clone())
            .collect::<HashSet<_>>();
        self.with_progress(|progress| {
            progress
                .all()
                .iter()
                .filter(|item| current_challenges.contains(&item.challenge_id))
                .cloned()
                .collect()
        })
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

    /// Serve typed requests over local IPC until the process is stopped.
    #[cfg(unix)]
    pub async fn serve(&self, socket: impl AsRef<Path>) -> Result<(), DaemonError> {
        let socket = socket.as_ref();
        if let Some(parent) = socket.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(DaemonError::Io)?;
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                    .map_err(DaemonError::Io)?;
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

    /// Serve typed requests over a local Windows named pipe.
    #[cfg(windows)]
    pub async fn serve(&self, pipe: impl AsRef<Path>) -> Result<(), DaemonError> {
        let pipe = pipe.as_ref().as_os_str();
        let mut server = ServerOptions::new()
            .first_pipe_instance(true)
            .create(pipe)
            .map_err(DaemonError::Io)?;
        loop {
            server.connect().await.map_err(DaemonError::Io)?;
            let connected = server;
            server = ServerOptions::new().create(pipe).map_err(DaemonError::Io)?;
            let daemon = self.clone();
            tokio::spawn(async move {
                if let Err(error) = daemon.handle_connection(connected).await {
                    tracing::debug!(%error, "daemon client disconnected");
                }
            });
        }
    }

    async fn handle_connection<S>(&self, stream: S) -> Result<(), DaemonError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (mut reader, writer) = tokio::io::split(stream);
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

async fn read_envelope<R: AsyncRead + Unpin + ?Sized>(
    reader: &mut R,
) -> Result<Envelope, DaemonError> {
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

async fn write_envelope<W: AsyncWrite + Unpin>(
    writer: &AsyncMutex<W>,
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
    reader: Box<dyn AsyncRead + Unpin + Send>,
    writer: Box<dyn AsyncWrite + Unpin + Send>,
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
    /// Connect to the platform's local daemon endpoint.
    #[cfg(unix)]
    pub async fn connect(socket: impl AsRef<Path>) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket).await.map_err(ClientError::Io)?;
        Ok(Self::from_stream(stream))
    }

    /// Connect to a Windows named-pipe daemon.
    #[cfg(windows)]
    pub async fn connect(pipe: impl AsRef<Path>) -> Result<Self, ClientError> {
        let stream = ClientOptions::new()
            .open(pipe.as_ref().as_os_str())
            .map_err(ClientError::Io)?;
        Ok(Self::from_stream(stream))
    }

    fn from_stream<S>(stream: S) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (reader, writer) = tokio::io::split(stream);
        Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
            next_message_id: 1,
            pending_events: std::collections::VecDeque::new(),
        }
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

/// Return the configured local daemon endpoint.
#[cfg(unix)]
pub fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|directory| runtime_directory_is_writable(directory))
        .unwrap_or_else(|| std::env::temp_dir().join(format!("learn-{}", userspace_id())))
        .join("learn/daemon.sock")
}

/// Return this user's Windows named-pipe endpoint.
#[cfg(windows)]
pub fn socket_path() -> PathBuf {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".to_owned());
    let user = user
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    PathBuf::from(format!(r"\\.\pipe\learn-linux-{user}"))
}

#[cfg(unix)]
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
#[cfg(unix)]
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

#[cfg(windows)]
/// Return the durable Windows progress database path.
pub fn progress_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("LearnLinux")
        .join("progress.redb")
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
        .or_else(|_| std::env::var("USERNAME"))
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

    #[cfg(unix)]
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
        *daemon.progress.lock().expect("progress lock") = Some(
            ProgressStore::open(directory.join("progress.redb")).expect("isolated test progress"),
        );
        let legacy_id = ChallengeId::new("foundation.orientation").expect("legacy ID");
        daemon
            .complete_challenge(&legacy_id)
            .expect("record old course progress");
        let current_id = ChallengeId::new("relay.01").expect("current ID");
        daemon
            .complete_challenge(&current_id)
            .expect("record current course progress");
        let visible_progress = daemon.progress().expect("filtered progress");
        assert_eq!(
            visible_progress
                .iter()
                .map(|item| item.challenge_id.as_str())
                .collect::<Vec<_>>(),
            vec!["relay.01"]
        );
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
        fs::remove_file(&socket).expect("remove preflight socket");
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
        assert_eq!(hello.content_version, "54 challenges");
        assert!(hello.runtime_available);
        let docs = client
            .search_docs("serch text in fiel", 5)
            .await
            .expect("docs");
        assert!(docs.iter().any(|result| result.id == "command.grep"));
        let opened = client
            .open_session("relay.01", 80, 24)
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

    #[cfg(windows)]
    #[tokio::test]
    async fn windows_named_pipe_answers_a_typed_handshake() {
        let daemon = Daemon::load_builtin().expect("bundled content");
        let pipe = PathBuf::from(format!(r"\\.\pipe\learn-linux-test-{}", std::process::id()));
        let server = tokio::spawn({
            let daemon = daemon.clone();
            let pipe = pipe.clone();
            async move { daemon.serve(pipe).await }
        });
        let mut client = None;
        for _ in 0..50 {
            if let Ok(connected) = DaemonClient::connect(&pipe).await {
                client = Some(connected);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let mut client = client.expect("named pipe became available");
        let hello = client.hello().await.expect("typed handshake");
        assert_eq!(hello.protocol_version, PROTOCOL_VERSION);
        assert!(hello.runtime_available);
        server.abort();
    }
}
