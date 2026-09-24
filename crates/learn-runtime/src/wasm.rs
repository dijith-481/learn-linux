#![allow(clippy::module_name_repetitions, dead_code)]

//! Portable, deterministic Linux lessons backed by an embedded WASM guest.
//!
//! The guest bundle provides a versioned runtime ABI, while this module
//! provides the POSIX-like filesystem and command semantics needed by the
//! curriculum. Nothing here reads or writes the host filesystem.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::io::Read;
use std::sync::{Arc, Condvar, Mutex};

use crate::RuntimeError;
use learn_core::{
    EnvironmentSpec, NetworkPolicy, SandboxPath, SetupAction, StateMode, ToolId,
    ToolRestrictionStrength, VerifierExpr, VerifierPredicate,
};
use learn_docs::command_manual;
use serde::{Deserialize, Serialize};

/// The embedded guest module's stable ABI version.
pub const WASM_GUEST_ABI_VERSION: i32 = 1;

const MAX_SESSION_SNAPSHOT_BYTES: usize = 1_048_576;
const MAX_COMMAND_HISTORY: usize = 500;

// A valid WebAssembly module exporting abi_version() -> i32. Keeping this
// bundle in the binary makes the default runtime independent of a rootfs,
// shell executable, dynamic linker, or host command.
const WASM_GUEST_BUNDLE: &[u8] = &[
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f, 0x03,
    0x02, 0x01, 0x00, 0x07, 0x0f, 0x01, 0x0b, b'a', b'b', b'i', b'_', b'v', b'e', b'r', b's', b'i',
    b'o', b'n', 0x00, 0x00, 0x0a, 0x06, 0x01, 0x04, 0x00, 0x41, 0x01, 0x0b,
];

/// Return the embedded guest ABI version after validating its WASM contract.
pub fn wasm_bundle_version() -> Result<i32, RuntimeError> {
    let valid_header =
        WASM_GUEST_BUNDLE.get(..8) == Some(&[0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
    let has_export = WASM_GUEST_BUNDLE
        .windows(b"abi_version".len())
        .any(|window| window == b"abi_version");
    let has_const_one = WASM_GUEST_BUNDLE.ends_with(&[0x41, 0x01, 0x0b]);
    if valid_header && has_export && has_const_one {
        Ok(WASM_GUEST_ABI_VERSION)
    } else {
        Err(RuntimeError::Wasm(
            "embedded guest is not a valid Learn Linux WASM bundle".to_owned(),
        ))
    }
}

/// A deterministic lesson session implemented without host processes.
pub struct WasmSession {
    /// Session ID.
    pub id: String,
    challenge_id: String,
    state: Arc<Mutex<SessionState>>,
    output: Arc<OutputBuffer>,
    reader: Option<Box<dyn Read + Send>>,
}

impl fmt::Debug for WasmSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WasmSession")
            .field("id", &self.id)
            .field("cwd", &self.current_directory())
            .finish_non_exhaustive()
    }
}

impl WasmSession {
    /// Read bytes produced by the simulated terminal.
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, RuntimeError> {
        self.reader
            .as_mut()
            .ok_or(RuntimeError::ReaderTaken)?
            .read(buffer)
            .map_err(RuntimeError::Io)
    }

    /// Take the terminal output reader for a forwarding thread.
    pub fn take_reader(&mut self) -> Option<Box<dyn Read + Send>> {
        self.reader.take()
    }

    /// Send raw terminal bytes to the simulated shell.
    pub fn write(&self, data: &[u8]) -> Result<(), RuntimeError> {
        let mut state = self.state.lock().map_err(|_| RuntimeError::Poisoned)?;
        for byte in data {
            state.receive_byte(*byte, &self.output);
        }
        Ok(())
    }

    /// Terminal resize is accepted for protocol parity; the virtual terminal
    /// has no host window to resize.
    pub fn resize(&self, _columns: u16, _rows: u16) -> Result<(), RuntimeError> {
        Ok(())
    }

    /// Serialize this lesson's virtual files, working directory and shell history.
    pub fn snapshot(&self) -> Result<Vec<u8>, RuntimeError> {
        let state = self.state.lock().map_err(|_| RuntimeError::Poisoned)?;
        serde_json::to_vec(&WasmSessionSnapshot {
            version: 1,
            challenge_id: self.challenge_id.clone(),
            shell: state.shell.clone(),
            input: state.input.clone(),
            history: state.history[state.history.len().saturating_sub(MAX_COMMAND_HISTORY)..]
                .to_vec(),
            cursor: state.cursor,
        })
        .map_err(|error| RuntimeError::Wasm(format!("could not save lesson state: {error}")))
    }

    /// Restore saved lesson state when its challenge matches this session.
    pub fn restore_snapshot(
        &self,
        challenge_id: &str,
        snapshot: &[u8],
    ) -> Result<(), RuntimeError> {
        if snapshot.len() > MAX_SESSION_SNAPSHOT_BYTES {
            return Err(RuntimeError::Wasm(
                "saved lesson state is larger than the supported limit".to_owned(),
            ));
        }
        let snapshot: WasmSessionSnapshot = serde_json::from_slice(snapshot).map_err(|error| {
            RuntimeError::Wasm(format!("saved lesson state is invalid: {error}"))
        })?;
        if snapshot.version != 1
            || snapshot.challenge_id != challenge_id
            || self.challenge_id != challenge_id
        {
            return Err(RuntimeError::Wasm(
                "saved lesson state belongs to a different challenge or version".to_owned(),
            ));
        }
        validate_virtual_environment(&snapshot.shell.environment)?;
        let cwd_is_directory = matches!(
            snapshot.shell.fs.get(&snapshot.shell.cwd),
            Some(Node::Directory)
        );
        if !cwd_is_directory
            || !snapshot.shell.cwd.starts_with("/home/learner")
            || snapshot.history.len() > MAX_COMMAND_HISTORY
            || snapshot.input.len() > 65_536
        {
            return Err(RuntimeError::Wasm(
                "saved lesson state failed validation".to_owned(),
            ));
        }
        let cwd = snapshot.shell.cwd.clone();
        let mut state = self.state.lock().map_err(|_| RuntimeError::Poisoned)?;
        state.shell = snapshot.shell;
        state.input = snapshot.input;
        state.history = snapshot.history;
        state.history_cursor = None;
        state.cursor = snapshot.cursor.min(state.input.len());
        state.escape.clear();
        state.closed = false;
        drop(state);
        self.output
            .push(format!("\r\n[lesson resumed]\r\n{}", prompt(&cwd)).as_bytes());
        Ok(())
    }

    /// The simulated shell remains alive until its session is closed.
    pub fn try_wait(&mut self) -> Result<Option<i32>, RuntimeError> {
        let state = self.state.lock().map_err(|_| RuntimeError::Poisoned)?;
        Ok(state.closed.then_some(0))
    }

    /// Verify predicates against the private virtual filesystem.
    pub fn verify(&self, expression: &VerifierExpr) -> Result<bool, RuntimeError> {
        let state = self.state.lock().map_err(|_| RuntimeError::Poisoned)?;
        verify_virtual_expression(&state.shell, expression)
    }

    /// Commands entered in this lesson, for continuity across challenges.
    pub fn command_history(&self) -> Vec<Vec<u8>> {
        self.state
            .lock()
            .map_or_else(|_| Vec::new(), |state| state.history.clone())
    }

    /// Restore commands from the preceding lesson.
    pub fn set_command_history(&self, history: Vec<Vec<u8>>) -> Result<(), RuntimeError> {
        let mut state = self.state.lock().map_err(|_| RuntimeError::Poisoned)?;
        let first_retained = history.len().saturating_sub(MAX_COMMAND_HISTORY);
        state.history = history.into_iter().skip(first_retained).collect();
        state.history_cursor = None;
        Ok(())
    }

    fn current_directory(&self) -> String {
        self.state.lock().map_or_else(
            |_| "/home/learner".to_owned(),
            |state| state.shell.cwd.clone(),
        )
    }
}

impl Drop for WasmSession {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
        }
        self.output.close();
    }
}

/// Start the default embedded WASM lesson runtime.
pub fn start_wasm_session(
    challenge_id: &str,
    setup: &[SetupAction],
    environment: &EnvironmentSpec,
    columns: u16,
    rows: u16,
) -> Result<WasmSession, RuntimeError> {
    start_wasm_session_with_intro(challenge_id, setup, environment, columns, rows, true)
}

/// Start a later lesson without replaying the initial startup art.
pub fn start_wasm_session_quiet(
    challenge_id: &str,
    setup: &[SetupAction],
    environment: &EnvironmentSpec,
    columns: u16,
    rows: u16,
) -> Result<WasmSession, RuntimeError> {
    start_wasm_session_with_intro(challenge_id, setup, environment, columns, rows, false)
}

fn start_wasm_session_with_intro(
    challenge_id: &str,
    setup: &[SetupAction],
    environment: &EnvironmentSpec,
    _columns: u16,
    _rows: u16,
    show_intro: bool,
) -> Result<WasmSession, RuntimeError> {
    if wasm_bundle_version()? != WASM_GUEST_ABI_VERSION {
        return Err(RuntimeError::Wasm(
            "embedded guest ABI version is unsupported".to_owned(),
        ));
    }
    validate_virtual_environment(environment)?;
    let mut shell = VirtualShell::new(environment.clone());
    shell.apply_setup(setup)?;
    let output = Arc::new(OutputBuffer::default());
    let state = Arc::new(Mutex::new(SessionState {
        shell,
        input: Vec::new(),
        history: Vec::new(),
        history_cursor: None,
        cursor: 0,
        escape: Vec::new(),
        closed: false,
    }));
    let session = WasmSession {
        id: format!("wasm-{challenge_id}-{}", crate::unique_suffix()),
        challenge_id: challenge_id.to_owned(),
        state,
        output: Arc::clone(&output),
        reader: Some(Box::new(OutputReader::new(output))),
    };
    if show_intro {
        session.output.push(startup_fetch(challenge_id).as_bytes());
    }
    session
        .output
        .push(prompt(&session.current_directory()).as_bytes());
    Ok(session)
}

fn validate_virtual_environment(environment: &EnvironmentSpec) -> Result<(), RuntimeError> {
    if environment.state != StateMode::Ephemeral {
        return Err(RuntimeError::UnsupportedStateMode(environment.state));
    }
    if environment.network != NetworkPolicy::None {
        return Err(RuntimeError::UnsupportedNetworkPolicy(environment.network));
    }
    Ok(())
}

#[derive(Debug)]
struct SessionState {
    shell: VirtualShell,
    input: Vec<u8>,
    history: Vec<Vec<u8>>,
    history_cursor: Option<usize>,
    cursor: usize,
    escape: Vec<u8>,
    closed: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct WasmSessionSnapshot {
    version: u8,
    challenge_id: String,
    shell: VirtualShell,
    input: Vec<u8>,
    history: Vec<Vec<u8>>,
    cursor: usize,
}

impl SessionState {
    fn receive_byte(&mut self, byte: u8, output: &OutputBuffer) {
        if self.closed {
            return;
        }
        if !self.escape.is_empty() {
            self.receive_escape_byte(byte, output);
            return;
        }
        match byte {
            b'\r' | b'\n' => {
                if self.input.is_empty() {
                    output.push(b"\r\n");
                    output.push(prompt(&self.shell.cwd).as_bytes());
                } else {
                    let command = String::from_utf8_lossy(&self.input).into_owned();
                    let should_close = command.split_ascii_whitespace().next() == Some("exit");
                    if self
                        .history
                        .last()
                        .is_none_or(|previous| previous != &self.input)
                    {
                        self.history.push(self.input.clone());
                    }
                    if self.history.len() > MAX_COMMAND_HISTORY {
                        self.history.remove(0);
                    }
                    self.history_cursor = None;
                    self.input.clear();
                    self.cursor = 0;
                    output.push(b"\r\n");
                    let result = self.shell.execute_line(&command);
                    if !result.output.is_empty() {
                        output.push(&terminalize_output(&result.output));
                        if !result.output.ends_with(b"\n") && !result.output.starts_with(b"\x1b[2J")
                        {
                            output.push(b"\r\n");
                        }
                    }
                    if result.status != 0 {
                        output.push(format!("[exit {}]\r\n", result.status).as_bytes());
                    }
                    if should_close && result.status == 0 {
                        self.closed = true;
                        output.push(b"\r\n[session closed]\r\n");
                        output.close();
                    } else {
                        output.push(prompt(&self.shell.cwd).as_bytes());
                    }
                }
            }
            0x7f | 0x08 => {
                if self.cursor > 0 {
                    let previous = previous_boundary(&self.input, self.cursor);
                    self.input.drain(previous..self.cursor);
                    self.cursor = previous;
                    self.redraw_input(output);
                }
            }
            0x03 => {
                self.input.clear();
                self.cursor = 0;
                output.push(b"^C\r\n");
                output.push(prompt(&self.shell.cwd).as_bytes());
            }
            0x04 => {
                self.closed = true;
                output.close();
            }
            b'\t' => self.complete(output),
            0x01 => {
                self.cursor = 0;
                self.redraw_input(output);
            }
            0x05 => {
                self.cursor = self.input.len();
                self.redraw_input(output);
            }
            0x1b => self.escape.push(byte),
            byte if byte.is_ascii_control() => {}
            byte => {
                self.input.insert(self.cursor, byte);
                self.cursor += 1;
                if self.cursor == self.input.len() {
                    output.push(&[byte]);
                } else {
                    self.redraw_input(output);
                }
            }
        }
    }

    fn receive_escape_byte(&mut self, byte: u8, output: &OutputBuffer) {
        self.escape.push(byte);
        if self.escape.len() == 2 && byte != b'[' {
            self.escape.clear();
            return;
        }
        if self.escape.len() < 3 {
            return;
        }
        match byte {
            b'D' => self.move_left(output),
            b'C' => self.move_right(output),
            b'A' => self.history_up(output),
            b'B' => self.history_down(output),
            b'H' => {
                self.cursor = 0;
                self.redraw_input(output);
            }
            b'F' => {
                self.cursor = self.input.len();
                self.redraw_input(output);
            }
            _ => {}
        }
        if byte.is_ascii_alphabetic() || byte == b'~' {
            self.escape.clear();
        }
    }

    fn move_left(&mut self, output: &OutputBuffer) {
        self.cursor = previous_boundary(&self.input, self.cursor);
        output.push(b"\x1b[D");
        self.escape.clear();
    }

    fn move_right(&mut self, output: &OutputBuffer) {
        self.cursor = next_boundary(&self.input, self.cursor);
        output.push(b"\x1b[C");
        self.escape.clear();
    }

    fn history_up(&mut self, output: &OutputBuffer) {
        if self.history.is_empty() {
            self.escape.clear();
            return;
        }
        let index = self
            .history_cursor
            .map_or(self.history.len().saturating_sub(1), |index| {
                index.saturating_sub(1)
            });
        self.history_cursor = Some(index);
        self.input.clone_from(&self.history[index]);
        self.cursor = self.input.len();
        self.redraw_input(output);
    }

    fn history_down(&mut self, output: &OutputBuffer) {
        let Some(index) = self.history_cursor else {
            self.escape.clear();
            return;
        };
        if index + 1 < self.history.len() {
            let next = index + 1;
            self.history_cursor = Some(next);
            self.input.clone_from(&self.history[next]);
        } else {
            self.history_cursor = None;
            self.input.clear();
        }
        self.cursor = self.input.len();
        self.redraw_input(output);
    }

    fn complete(&mut self, output: &OutputBuffer) {
        let token_start = self.input[..self.cursor]
            .iter()
            .rposition(u8::is_ascii_whitespace)
            .map_or(0, |index| index + 1);
        let token = String::from_utf8_lossy(&self.input[token_start..self.cursor]);
        let command_position = token_start == 0;
        let candidates = self.shell.completion_candidates(&token, command_position);
        match candidates.as_slice() {
            [] => output.push(b"\x07"),
            [(candidate, is_directory)] => {
                let mut replacement = candidate.clone();
                if *is_directory {
                    replacement.push('/');
                } else {
                    replacement.push(' ');
                }
                self.input.splice(
                    token_start..self.cursor,
                    replacement.as_bytes().iter().copied(),
                );
                self.cursor = token_start + replacement.len();
                self.redraw_input(output);
            }
            _ => {
                output.push(b"\r\n");
                let values = candidates
                    .iter()
                    .map(|(value, directory)| {
                        if *directory {
                            format!("{value}/")
                        } else {
                            value.clone()
                        }
                    })
                    .collect::<Vec<_>>();
                output.push(pretty_columns(&values, 3).as_bytes());
                self.write_input(output);
            }
        }
    }

    fn redraw_input(&self, output: &OutputBuffer) {
        // The prompt occupies two rows. Clear the input row, move to the
        // prompt header, clear it too, then paint the pair exactly once.
        output.push(b"\r\x1b[2K\x1b[1A\r\x1b[2K");
        self.write_input(output);
    }

    fn write_input(&self, output: &OutputBuffer) {
        output.push(prompt(&self.shell.cwd).as_bytes());
        output.push(&self.input);
        output.push(b"\x1b[K");
        let tail = self.input.len().saturating_sub(self.cursor);
        if tail > 0 {
            output.push(format!("\x1b[{tail}D").as_bytes());
        }
    }
}

fn prompt(cwd: &str) -> String {
    let display = cwd.strip_prefix("/home/learner").map_or_else(
        || cwd.to_owned(),
        |suffix| {
            if suffix.is_empty() {
                "~".to_owned()
            } else {
                format!("~{suffix}")
            }
        },
    );
    format!("\r\x1b]777;LEARN_PWD;{cwd}\x07learner@learn  {display}  ◈\r\n❯ ")
}

const SHELL_BANNER: &str = "\r\n  ╭──────────────────────────────────────────────────────╮\r\n  │   ██╗     ███████╗ █████╗ ██████╗ ███╗   ██╗        │\r\n  │   ██║     ██╔════╝██╔══██╗██╔══██╗████╗  ██║        │\r\n  │   ██║     █████╗  ███████║██████╔╝██╔██╗ ██║        │\r\n  │   ██║     ██╔══╝  ██╔══██║██╔══██╗██║╚██╗██║        │\r\n  │   ███████╗███████╗██║  ██║██║  ██║██║ ╚████║        │\r\n  │   ╚══════╝╚══════╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═══╝        │\r\n  │                                                      │\r\n  │   ██╗     ██╗███╗   ██╗██╗   ██╗██╗  ██╗           │\r\n  │   ██║     ██║████╗  ██║██║   ██║╚██╗██╔╝           │\r\n  │   ██║     ██║██╔██╗ ██║██║   ██║ ╚███╔╝            │\r\n  │   ██║     ██║██║╚██╗██║██║   ██║ ██╔██╗            │\r\n  │   ███████╗██║██║ ╚████║╚██████╔╝██╔╝ ██╗           │\r\n  │   ╚══════╝╚═╝╚═╝  ╚═══╝ ╚═════╝ ╚═╝  ╚═╝           │\r\n  ╰──── shell dojo • type a command • make progress ───╯\r\n\r\n";

const COMPACT_SHELL_BANNER: &str = "\r\n  ╭────────────────────────────────────────────╮\r\n  │       L E A R N   L I N U X                │\r\n  │     shell dojo • type help • level up      │\r\n  ╰────────────────────────────────────────────╯\r\n\r\n";

fn terminalize_output(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut previous = None;
    for &byte in bytes {
        if byte == b'\n' && previous != Some(b'\r') {
            output.push(b'\r');
        }
        output.push(byte);
        previous = Some(byte);
    }
    output
}

fn shell_banner(columns: u16) -> &'static str {
    if columns < 72 {
        COMPACT_SHELL_BANNER
    } else {
        SHELL_BANNER
    }
}

fn startup_fetch(challenge_id: &str) -> String {
    format!(
        "\r\n       ▄████▄          │  🐧  LINUX SHELL DOJO\r\n     ▄████████▄        │  runtime  embedded-posix\r\n    ███  ◉  ◉ ███      │  quest    {challenge_id}\r\n    ███   ▾   ███      │  status   offline • verifier ready\r\n     ▀████████▀        │  shell    virtual-posix\r\n       ▀▀  ▀▀          │  docs     Ctrl-L search\r\n\r\n"
    )
}

#[derive(Debug, Default)]
struct OutputBuffer {
    state: Mutex<OutputState>,
    available: Condvar,
}

#[derive(Debug, Default)]
struct OutputState {
    bytes: VecDeque<u8>,
    closed: bool,
}

impl OutputBuffer {
    fn push(&self, bytes: &[u8]) {
        if let Ok(mut state) = self.state.lock() {
            if state.closed {
                return;
            }
            state.bytes.extend(bytes);
            self.available.notify_all();
        }
    }

    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            self.available.notify_all();
        }
    }
}

#[derive(Debug)]
struct OutputReader {
    output: Arc<OutputBuffer>,
}

impl OutputReader {
    fn new(output: Arc<OutputBuffer>) -> Self {
        Self { output }
    }
}

impl Read for OutputReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let mut state = self
            .output
            .state
            .lock()
            .map_err(|_| std::io::Error::other("output lock poisoned"))?;
        while state.bytes.is_empty() && !state.closed {
            state = self
                .output
                .available
                .wait(state)
                .map_err(|_| std::io::Error::other("output lock poisoned"))?;
        }
        let count = buffer.len().min(state.bytes.len());
        for destination in &mut buffer[..count] {
            *destination = state.bytes.pop_front().expect("count matches queue length");
        }
        Ok(count)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
enum Node {
    Directory,
    File { contents: Vec<u8>, mode: u32 },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct VirtualShell {
    fs: BTreeMap<String, Node>,
    cwd: String,
    environment: EnvironmentSpec,
    read_files: std::collections::BTreeSet<String>,
}

impl VirtualShell {
    fn new(environment: EnvironmentSpec) -> Self {
        let mut fs = BTreeMap::new();
        fs.insert("/home/learner".to_owned(), Node::Directory);
        Self {
            fs,
            cwd: "/home/learner".to_owned(),
            environment,
            read_files: std::collections::BTreeSet::new(),
        }
    }

    fn apply_setup(&mut self, setup: &[SetupAction]) -> Result<(), RuntimeError> {
        for action in setup {
            match action {
                SetupAction::CreateDir { path } => self.mkdir(path.as_str(), true)?,
                SetupAction::WriteFile { path, contents } => {
                    let path = self.resolve(path.as_str()).map_err(RuntimeError::Shell)?;
                    self.ensure_parent(&path).map_err(RuntimeError::Shell)?;
                    self.fs.insert(
                        path,
                        Node::File {
                            contents: contents.clone(),
                            mode: 0o644,
                        },
                    );
                }
                SetupAction::SetMode { path, mode } => {
                    let path = self.resolve(path.as_str()).map_err(RuntimeError::Shell)?;
                    let Some(Node::File { mode: current, .. }) = self.fs.get_mut(&path) else {
                        return Err(RuntimeError::Shell(format!(
                            "setup target is not a file: {path}"
                        )));
                    };
                    *current = *mode;
                }
            }
        }
        Ok(())
    }

    fn execute_line(&mut self, line: &str) -> CommandResult {
        let mut input = Vec::new();
        let mut status = 0;
        let mut output = Vec::new();
        let pipelines = split_unquoted(line, '|');
        if pipelines.iter().any(|pipeline| pipeline.is_empty()) {
            return CommandResult::failure("learn: a pipe needs a command on both sides");
        }
        let pipeline_count = pipelines.len();
        for (index, pipeline) in pipelines.into_iter().enumerate() {
            let (argv, stdin_path, stdout_path) = match parse_segment(&pipeline) {
                Ok(value) => value,
                Err(error) => return CommandResult::failure(error),
            };
            if argv.is_empty() {
                return CommandResult::failure("learn: expected a command in the pipeline");
            }
            if let Some(path) = stdin_path {
                match self
                    .resolve(&path)
                    .and_then(|path| self.read_file_path(&path))
                {
                    Ok(bytes) => input = bytes,
                    Err(error) => return CommandResult::failure(error),
                }
            }
            let result = self.run_command(&argv, &input, index + 1 < pipeline_count);
            status = result.status;
            output = result.output;
            if let Some((path, append)) = stdout_path {
                if status == 0 {
                    if let Err(error) = self.write_file_path(&path, &output, append) {
                        return CommandResult::failure(error);
                    }
                    output.clear();
                }
            }
            if status != 0 {
                break;
            }
            input.clone_from(&output);
        }
        CommandResult { output, status }
    }

    fn run_command(&mut self, argv: &[String], input: &[u8], piped_output: bool) -> CommandResult {
        let Some(command) = argv.first() else {
            return CommandResult::success(Vec::new());
        };
        if !is_known_command(command) {
            return CommandResult::failure(format!("learn: command '{command}' not found"));
        }
        if argv.get(1).is_some_and(|arg| arg == "--help") {
            return CommandResult::success(
                command_manual(command).unwrap_or_default().into_bytes(),
            );
        }
        if !self.command_allowed(command) {
            return CommandResult::failure(format!(
                "learn: command '{command}' is disabled for this challenge"
            ));
        }
        match command.as_str() {
            "cd" => self.command_cd(argv),
            "pwd" => CommandResult::success(format!("{}\n", self.cwd).into_bytes()),
            "printf" => CommandResult::success(printf_bytes(&argv[1..])),
            "echo" => CommandResult::success(format!("{}\n", argv[1..].join(" ")).into_bytes()),
            "true" | "exit" => CommandResult::success(Vec::new()),
            "clear" => CommandResult::success(b"\x1b[2J\x1b[H".to_vec()),
            "help" => {
                if let Some(name) = argv.get(1) {
                    command_manual(name).map_or_else(
                        || CommandResult::failure(format!("help: no entry for {name}")),
                        |manual| CommandResult::success(manual.into_bytes()),
                    )
                } else {
                    CommandResult::success(
                        b"Try: man COMMAND  help COMMAND  COMMAND --help\n".to_vec(),
                    )
                }
            }
            "man" => argv.get(1).map_or_else(
                || CommandResult::failure("man: expected a command name"),
                |name| {
                    command_manual(name).map_or_else(
                        || CommandResult::failure(format!("man: no entry for {name}")),
                        |manual| CommandResult::success(manual.into_bytes()),
                    )
                },
            ),
            "mkdir" => self.command_mkdir(&argv[1..]),
            "touch" => self.command_touch(&argv[1..]),
            "cat" | "less" => self.command_cat(&argv[1..], input),
            "mv" => self.command_mv(&argv[1..]),
            "cp" => self.command_cp(&argv[1..]),
            "rm" => self.command_rm(&argv[1..]),
            "grep" => self.command_grep(&argv[1..], input),
            "find" => self.command_find(&argv[1..]),
            "ls" => self.command_ls(&argv[1..], piped_output),
            "head" => self.command_head(&argv[1..], input),
            "tail" => self.command_tail(&argv[1..], input),
            "sort" => self.command_sort(&argv[1..], input),
            "uniq" => self.command_uniq(&argv[1..], input),
            "chmod" => self.command_chmod(&argv[1..]),
            "stat" => self.command_stat(&argv[1..]),
            "wc" => CommandResult::success(format!("{}\n", input.len()).into_bytes()),
            _ => CommandResult::failure(format!("learn: command '{command}' not found")),
        }
    }

    fn command_allowed(&self, command: &str) -> bool {
        if self.environment.restriction == ToolRestrictionStrength::Guidance
            || is_virtual_builtin(command)
            || is_basic_command(command)
        {
            return true;
        }
        let Ok(tool) = ToolId::new(command) else {
            return false;
        };
        self.environment.allowed_tools.contains(&tool)
    }

    fn completion_candidates(&self, token: &str, command_position: bool) -> Vec<(String, bool)> {
        if command_position {
            return [
                "cat", "cd", "chmod", "clear", "cp", "echo", "exit", "find", "grep", "head",
                "help", "less", "ls", "man", "mkdir", "mv", "printf", "pwd", "rm", "sort", "stat",
                "tail", "touch", "true", "uniq", "wc",
            ]
            .into_iter()
            .filter(|command| self.command_allowed(command) && command.starts_with(token))
            .map(|command| (command.to_owned(), false))
            .collect();
        }
        let typed_parent = token
            .rsplit_once('/')
            .map_or_else(String::new, |(parent, _)| format!("{parent}/"));
        let typed_leaf = token.rsplit('/').next().unwrap_or(token);
        let mut candidates = self
            .fs
            .iter()
            .filter_map(|(path, node)| {
                let directory = matches!(node, Node::Directory);
                let display = completion_path(path, token, &self.cwd)?;
                let remainder = display.strip_prefix(&typed_parent)?;
                let (component, has_more) = remainder
                    .split_once('/')
                    .map_or((remainder, false), |(component, _)| (component, true));
                component
                    .starts_with(typed_leaf)
                    .then(|| (format!("{typed_parent}{component}"), directory || has_more))
            })
            .collect::<Vec<_>>();
        candidates.sort();
        candidates.dedup();
        candidates
    }

    fn command_cd(&mut self, argv: &[String]) -> CommandResult {
        let destination = argv.get(1).map_or("~", String::as_str);
        match self.resolve(destination) {
            Ok(path) if matches!(self.fs.get(&path), Some(Node::Directory)) => {
                self.cwd = path;
                CommandResult::success(Vec::new())
            }
            Ok(path) => CommandResult::failure(format!("cd: not a directory: {path}")),
            Err(error) => CommandResult::failure(error),
        }
    }

    fn command_mkdir(&mut self, argv: &[String]) -> CommandResult {
        let mut parents = false;
        let mut paths = Vec::new();
        for value in argv {
            if value == "-p" {
                parents = true;
            } else {
                paths.push(value.as_str());
            }
        }
        for path in paths {
            if let Err(error) = self.mkdir(path, parents) {
                return CommandResult::failure(error.to_string());
            }
        }
        CommandResult::success(Vec::new())
    }

    fn command_touch(&mut self, argv: &[String]) -> CommandResult {
        for value in argv {
            match self.resolve(value).and_then(|path| {
                if matches!(self.fs.get(&path), Some(Node::Directory)) {
                    return Err(format!("touch: is a directory: {path}"));
                }
                self.ensure_parent(&path)?;
                self.fs.entry(path).or_insert(Node::File {
                    contents: Vec::new(),
                    mode: 0o644,
                });
                Ok(())
            }) {
                Ok(()) => {}
                Err(error) => return CommandResult::failure(error),
            }
        }
        CommandResult::success(Vec::new())
    }

    fn command_cat(&mut self, argv: &[String], input: &[u8]) -> CommandResult {
        if argv.is_empty() {
            return CommandResult::success(input.to_vec());
        }
        let mut output = Vec::new();
        for value in argv {
            let path = match self.resolve(value) {
                Ok(path) => path,
                Err(error) => return CommandResult::failure(format!("cat: {value}: {error}")),
            };
            match self.read_file_path(&path) {
                Ok(bytes) => {
                    self.read_files.insert(path);
                    output.extend(bytes);
                }
                Err(error) => return CommandResult::failure(format!("cat: {value}: {error}")),
            }
        }
        CommandResult::success(output)
    }

    fn command_mv(&mut self, argv: &[String]) -> CommandResult {
        if argv.len() != 2 {
            return CommandResult::failure("mv: expected SOURCE DEST".to_owned());
        }
        let source = match self.resolve(&argv[0]) {
            Ok(path) => path,
            Err(error) => return CommandResult::failure(error),
        };
        let mut destination = match self.resolve(&argv[1]) {
            Ok(path) => path,
            Err(error) => return CommandResult::failure(error),
        };
        if matches!(self.fs.get(&destination), Some(Node::Directory)) {
            let Some(name) = basename(&source) else {
                return CommandResult::failure("mv: invalid source".to_owned());
            };
            destination.push('/');
            destination.push_str(name);
        }
        if !self.fs.contains_key(&source) {
            return CommandResult::failure(format!("mv: cannot stat {source}"));
        }
        if let Err(error) = self.ensure_parent(&destination) {
            return CommandResult::failure(error);
        }
        let moved = self
            .fs
            .iter()
            .filter(|(path, _)| *path == &source || path.starts_with(&format!("{source}/")))
            .map(|(path, node)| (path.clone(), node.clone()))
            .collect::<Vec<_>>();
        for (path, _) in &moved {
            self.fs.remove(path);
        }
        for (path, node) in moved {
            let suffix = path.strip_prefix(&source).unwrap_or_default();
            self.fs.insert(format!("{destination}{suffix}"), node);
        }
        CommandResult::success(Vec::new())
    }

    fn command_cp(&mut self, argv: &[String]) -> CommandResult {
        if argv.len() != 2 {
            return CommandResult::failure("cp: expected SOURCE DEST".to_owned());
        }
        let destination = match self.resolve(&argv[1]) {
            Ok(path) => path,
            Err(error) => return CommandResult::failure(error),
        };
        let source_pattern = match self.resolve(&argv[0]) {
            Ok(path) => path,
            Err(error) => return CommandResult::failure(error),
        };
        let sources = if source_pattern.contains('*') || source_pattern.contains('?') {
            self.fs
                .iter()
                .filter(|(path, node)| {
                    matches!(node, Node::File { .. }) && glob_match(&source_pattern, path)
                })
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>()
        } else if self.fs.contains_key(&source_pattern) {
            vec![source_pattern]
        } else {
            Vec::new()
        };
        if sources.is_empty() {
            return CommandResult::failure(format!("cp: cannot stat {}", argv[0]));
        }
        let destination_is_directory = matches!(self.fs.get(&destination), Some(Node::Directory));
        if sources.len() > 1 && !destination_is_directory {
            return CommandResult::failure("cp: target is not a directory".to_owned());
        }
        for source in sources {
            let mut target = destination.clone();
            if destination_is_directory {
                let Some(name) = basename(&source) else {
                    return CommandResult::failure("cp: invalid source".to_owned());
                };
                target.push('/');
                target.push_str(name);
            }
            let Some(node) = self.fs.get(&source).cloned() else {
                return CommandResult::failure(format!("cp: cannot stat {source}"));
            };
            if matches!(node, Node::Directory) {
                return CommandResult::failure("cp: recursive copies are not enabled".to_owned());
            }
            if let Err(error) = self.ensure_parent(&target) {
                return CommandResult::failure(error);
            }
            self.fs.insert(target, node);
        }
        CommandResult::success(Vec::new())
    }

    fn command_rm(&mut self, argv: &[String]) -> CommandResult {
        let recursive = argv.iter().any(|item| item == "-r");
        for value in argv.iter().filter(|item| item.as_str() != "-r") {
            let path = match self.resolve(value) {
                Ok(path) => path,
                Err(error) => return CommandResult::failure(error),
            };
            if !self.fs.contains_key(&path) {
                return CommandResult::failure(format!("rm: cannot remove {path}"));
            }
            if matches!(self.fs.get(&path), Some(Node::Directory)) && !recursive {
                return CommandResult::failure(format!("rm: is a directory: {path}"));
            }
            let descendants = self
                .fs
                .keys()
                .filter(|candidate| {
                    *candidate == &path || candidate.starts_with(&format!("{path}/"))
                })
                .cloned()
                .collect::<Vec<_>>();
            for descendant in descendants {
                self.fs.remove(&descendant);
            }
        }
        CommandResult::success(Vec::new())
    }

    fn command_grep(&self, argv: &[String], input: &[u8]) -> CommandResult {
        let Some(pattern) = argv.first() else {
            return CommandResult::failure("grep: missing pattern".to_owned());
        };
        let contents = if let Some(path) = argv.get(1) {
            match self
                .resolve(path)
                .and_then(|path| self.read_file_path(&path))
            {
                Ok(contents) => contents,
                Err(error) => return CommandResult::failure(format!("grep: {path}: {error}")),
            }
        } else {
            input.to_vec()
        };
        let output = contents
            .split_inclusive(|byte| *byte == b'\n')
            .filter(|line| String::from_utf8_lossy(line).contains(pattern))
            .flat_map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        let status = i32::from(output.is_empty());
        CommandResult { output, status }
    }

    fn command_find(&self, argv: &[String]) -> CommandResult {
        let start = argv.first().map_or(".", String::as_str);
        let name = argv
            .windows(2)
            .find(|pair| pair[0] == "-name")
            .map(|pair| pair[1].as_str());
        let start = match self.resolve(start) {
            Ok(path) => path,
            Err(error) => return CommandResult::failure(error),
        };
        if !matches!(self.fs.get(&start), Some(Node::Directory)) {
            return CommandResult::failure(format!("find: {start}: not a directory"));
        }
        let mut matches = self
            .fs
            .keys()
            .filter(|path| *path == &start || path.starts_with(&format!("{start}/")))
            .filter(|path| {
                name.is_none_or(|pattern| basename(path).is_some_and(|base| base == pattern))
            })
            .cloned()
            .collect::<Vec<_>>();
        matches.sort();
        CommandResult::success(format!("{}\n", matches.join("\n")).into_bytes())
    }

    fn command_ls(&self, argv: &[String], piped_output: bool) -> CommandResult {
        let path = argv.first().map_or(self.cwd.as_str(), String::as_str);
        let path = match self.resolve(path) {
            Ok(path) => path,
            Err(error) => return CommandResult::failure(error),
        };
        let Some(Node::Directory) = self.fs.get(&path) else {
            return CommandResult::failure(format!("ls: not a directory: {path}"));
        };
        let prefix = format!("{path}/");
        let mut entries = self
            .fs
            .keys()
            .filter_map(|candidate| candidate.strip_prefix(&prefix))
            .filter_map(|relative| relative.split('/').next())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        entries.sort();
        entries.dedup();
        let values = entries
            .into_iter()
            .map(|entry| {
                let entry_path = format!("{prefix}{entry}");
                if matches!(self.fs.get(&entry_path), Some(Node::Directory)) {
                    format!("{entry}/")
                } else {
                    entry
                }
            })
            .collect::<Vec<_>>();
        if piped_output {
            let output = if values.is_empty() {
                String::new()
            } else {
                format!("{}\n", values.join("\n"))
            };
            CommandResult::success(output.into_bytes())
        } else {
            CommandResult::success(pretty_columns(&values, 3).into_bytes())
        }
    }

    fn command_head(&self, argv: &[String], input: &[u8]) -> CommandResult {
        let contents = if let Some(path) = argv.first() {
            match self
                .resolve(path)
                .and_then(|path| self.read_file_path(&path))
            {
                Ok(contents) => contents,
                Err(error) => return CommandResult::failure(error),
            }
        } else {
            input.to_vec()
        };
        let mut lines = contents.split_inclusive(|byte| *byte == b'\n');
        let mut output = Vec::new();
        for _ in 0..10 {
            let Some(line) = lines.next() else { break };
            output.extend(line);
        }
        CommandResult::success(output)
    }

    fn command_tail(&self, argv: &[String], input: &[u8]) -> CommandResult {
        let contents = if let Some(path) = argv.first() {
            match self
                .resolve(path)
                .and_then(|path| self.read_file_path(&path))
            {
                Ok(contents) => contents,
                Err(error) => return CommandResult::failure(error),
            }
        } else {
            input.to_vec()
        };
        let lines = contents
            .split_inclusive(|byte| *byte == b'\n')
            .collect::<Vec<_>>();
        CommandResult::success(lines[lines.len().saturating_sub(10)..].concat())
    }

    fn command_sort(&self, argv: &[String], input: &[u8]) -> CommandResult {
        let contents = if let Some(path) = argv.first() {
            match self
                .resolve(path)
                .and_then(|path| self.read_file_path(&path))
            {
                Ok(contents) => contents,
                Err(error) => return CommandResult::failure(error),
            }
        } else {
            input.to_vec()
        };
        let mut lines = String::from_utf8_lossy(&contents)
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        lines.sort();
        CommandResult::success(if lines.is_empty() {
            Vec::new()
        } else {
            format!("{}\n", lines.join("\n")).into_bytes()
        })
    }

    fn command_uniq(&self, argv: &[String], input: &[u8]) -> CommandResult {
        let contents = if let Some(path) = argv.first() {
            match self
                .resolve(path)
                .and_then(|path| self.read_file_path(&path))
            {
                Ok(contents) => contents,
                Err(error) => return CommandResult::failure(error),
            }
        } else {
            input.to_vec()
        };
        let mut unique = Vec::new();
        let text = String::from_utf8_lossy(&contents);
        for line in text.lines() {
            if unique.last().is_none_or(|last| *last != line) {
                unique.push(line);
            }
        }
        CommandResult::success(if unique.is_empty() {
            Vec::new()
        } else {
            format!("{}\n", unique.join("\n")).into_bytes()
        })
    }

    fn command_chmod(&mut self, argv: &[String]) -> CommandResult {
        if argv.len() < 2 {
            return CommandResult::failure("chmod: expected MODE FILE...");
        }
        for raw in &argv[1..] {
            let path = match self.resolve(raw) {
                Ok(path) => path,
                Err(error) => return CommandResult::failure(error),
            };
            let Some(Node::File { mode, .. }) = self.fs.get_mut(&path) else {
                return CommandResult::failure(format!("chmod: not a file: {raw}"));
            };
            match parse_permission_mode(*mode, &argv[0]) {
                Ok(updated) => *mode = updated,
                Err(error) => return CommandResult::failure(error),
            }
        }
        CommandResult::success(Vec::new())
    }

    fn command_stat(&self, argv: &[String]) -> CommandResult {
        let Some(raw) = argv.first() else {
            return CommandResult::failure("stat: expected FILE");
        };
        let path = match self.resolve(raw) {
            Ok(path) => path,
            Err(error) => return CommandResult::failure(error),
        };
        match self.fs.get(&path) {
            Some(Node::File { mode, .. }) => {
                CommandResult::success(format!("{mode:03o} {path}\n").into_bytes())
            }
            _ => CommandResult::failure(format!("stat: not a file: {raw}")),
        }
    }

    fn mkdir(&mut self, raw: &str, parents: bool) -> Result<(), RuntimeError> {
        let path = self.resolve(raw).map_err(RuntimeError::Shell)?;
        if matches!(self.fs.get(&path), Some(Node::Directory)) {
            return if parents {
                Ok(())
            } else {
                Err(RuntimeError::Shell(format!(
                    "mkdir: already exists: {path}"
                )))
            };
        }
        if !parents {
            self.ensure_parent(&path).map_err(RuntimeError::Shell)?;
            self.fs.insert(path, Node::Directory);
            return Ok(());
        }
        let mut current = String::from("/home/learner");
        for component in path
            .strip_prefix("/home/learner/")
            .unwrap_or_default()
            .split('/')
        {
            if component.is_empty() {
                continue;
            }
            current.push('/');
            current.push_str(component);
            if matches!(self.fs.get(&current), Some(Node::File { .. })) {
                return Err(RuntimeError::Shell(format!(
                    "mkdir: file exists: {current}"
                )));
            }
            self.fs.insert(current.clone(), Node::Directory);
        }
        Ok(())
    }

    fn ensure_parent(&self, path: &str) -> Result<(), String> {
        let Some(parent) = path
            .rsplit_once('/')
            .map(|(parent, _)| if parent.is_empty() { "/" } else { parent })
        else {
            return Err(format!("invalid path: {path}"));
        };
        if !matches!(self.fs.get(parent), Some(Node::Directory)) {
            return Err(format!("parent directory does not exist: {parent}"));
        }
        Ok(())
    }

    fn resolve(&self, raw: &str) -> Result<String, String> {
        let candidate = if raw == "~" {
            "/home/learner".to_owned()
        } else if let Some(relative) = raw.strip_prefix("~/") {
            format!("/home/learner/{relative}")
        } else if raw.starts_with('/') {
            raw.to_owned()
        } else {
            format!("{}/{raw}", self.cwd)
        };
        SandboxPath::parse(candidate)
            .map(|path| path.to_string())
            .map_err(|error| error.to_string())
    }

    fn read_file_path(&self, path: &str) -> Result<Vec<u8>, String> {
        match self.fs.get(path) {
            Some(Node::File { contents, mode }) if mode & 0o400 != 0 => Ok(contents.clone()),
            Some(Node::File { .. }) => Err(format!("permission denied: {path}")),
            Some(Node::Directory) => Err(format!("is a directory: {path}")),
            None => Err(format!("no such file: {path}")),
        }
    }

    fn write_file_path(&mut self, raw: &str, contents: &[u8], append: bool) -> Result<(), String> {
        let path = self.resolve(raw)?;
        self.ensure_parent(&path)?;
        if matches!(self.fs.get(&path), Some(Node::Directory)) {
            return Err(format!("is a directory: {path}"));
        }
        if matches!(self.fs.get(&path), Some(Node::File { mode, .. }) if mode & 0o200 == 0) {
            return Err(format!("permission denied: {path}"));
        }
        let previous = match self.fs.remove(&path) {
            Some(Node::File {
                contents: previous,
                mode,
            }) if append => {
                let mut combined = previous;
                combined.extend_from_slice(contents);
                (combined, mode)
            }
            Some(Node::File { mode, .. }) => (contents.to_vec(), mode),
            None => (contents.to_vec(), 0o644),
            Some(Node::Directory) => return Err(format!("is a directory: {path}")),
        };
        self.fs.insert(
            path,
            Node::File {
                contents: previous.0,
                mode: previous.1,
            },
        );
        Ok(())
    }
}

#[derive(Debug)]
struct CommandResult {
    output: Vec<u8>,
    status: i32,
}

impl CommandResult {
    fn success(output: Vec<u8>) -> Self {
        Self { output, status: 0 }
    }

    fn failure(error: impl Into<String>) -> Self {
        Self {
            output: format!("{}\n", error.into()).into_bytes(),
            status: 1,
        }
    }
}

fn parse_permission_mode(current: u32, specification: &str) -> Result<u32, String> {
    if specification
        .chars()
        .all(|character| character.is_ascii_digit())
    {
        return u32::from_str_radix(specification, 8)
            .ok()
            .filter(|mode| *mode <= 0o777)
            .ok_or_else(|| format!("chmod: invalid octal mode: {specification}"));
    }
    let mut mode = current;
    for clause in specification.split(',') {
        let Some(position) = clause.find(['+', '-', '=']) else {
            return Err(format!("chmod: invalid symbolic mode: {clause}"));
        };
        let (who, operation_and_rights) = clause.split_at(position);
        let operation = operation_and_rights.as_bytes()[0];
        let rights = &operation_and_rights[1..];
        if !who
            .chars()
            .all(|character| matches!(character, 'u' | 'g' | 'o' | 'a'))
            || who.is_empty()
            || !rights
                .chars()
                .all(|character| matches!(character, 'r' | 'w' | 'x'))
        {
            return Err(format!("chmod: invalid symbolic mode: {clause}"));
        }
        let mut target = 0;
        let mut granted = 0;
        for (class, shift) in [('u', 6), ('g', 3), ('o', 0)] {
            if who.contains(class) || who.contains('a') {
                target |= 0o7 << shift;
                for right in rights.chars() {
                    granted |= match right {
                        'r' => 0o4 << shift,
                        'w' => 0o2 << shift,
                        'x' => 0o1 << shift,
                        _ => 0,
                    };
                }
            }
        }
        mode = match operation {
            b'+' => mode | granted,
            b'-' => mode & !granted,
            b'=' => (mode & !target) | granted,
            _ => return Err(format!("chmod: invalid symbolic mode: {clause}")),
        };
    }
    Ok(mode)
}

fn is_virtual_builtin(command: &str) -> bool {
    matches!(
        command,
        "cd" | "pwd" | "printf" | "echo" | "true" | "clear" | "exit" | "help" | "man"
    )
}

fn is_known_command(command: &str) -> bool {
    matches!(
        command,
        "cat"
            | "cd"
            | "chmod"
            | "clear"
            | "cp"
            | "echo"
            | "exit"
            | "find"
            | "grep"
            | "head"
            | "help"
            | "less"
            | "ls"
            | "man"
            | "mkdir"
            | "mv"
            | "printf"
            | "pwd"
            | "rm"
            | "sort"
            | "stat"
            | "tail"
            | "touch"
            | "true"
            | "uniq"
            | "wc"
    )
}

fn is_basic_command(command: &str) -> bool {
    matches!(
        command,
        "cat" | "cd" | "echo" | "ls" | "printf" | "pwd" | "help"
    )
}

fn previous_boundary(value: &[u8], cursor: usize) -> usize {
    value[..cursor.min(value.len())]
        .iter()
        .rposition(|byte| *byte < 0x80 || *byte & 0xc0 == 0xc0)
        .unwrap_or(0)
}

fn next_boundary(value: &[u8], cursor: usize) -> usize {
    let cursor = cursor.min(value.len());
    if cursor >= value.len() {
        return value.len();
    }
    cursor
        + if value[cursor] & 0x80 == 0 {
            1
        } else if value[cursor] & 0xe0 == 0xc0 {
            2
        } else if value[cursor] & 0xf0 == 0xe0 {
            3
        } else {
            4
        }
}

fn completion_path(path: &str, token: &str, cwd: &str) -> Option<String> {
    if path == "/home/learner" {
        return None;
    }
    if token.starts_with("~/") {
        return Some(format!(
            "~{}",
            path.strip_prefix("/home/learner").unwrap_or(path)
        ));
    }
    if token.starts_with('/') {
        return Some(path.to_owned());
    }
    path.strip_prefix(&format!("{cwd}/")).map(str::to_owned)
}

fn pretty_columns(values: &[String], columns: usize) -> String {
    if values.is_empty() {
        return String::new();
    }
    let columns = columns.max(1);
    let rows = values.len().div_ceil(columns);
    let widths = (0..columns)
        .map(|column| {
            (0..rows)
                .filter_map(|row| values.get(row + column * rows))
                .map(String::len)
                .max()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let mut output = String::new();
    for row in 0..rows {
        for (column, width) in widths.iter().enumerate().take(columns) {
            let Some(value) = values.get(row + column * rows) else {
                continue;
            };
            output.push_str(value);
            if column + 1 < columns && row + column * rows + rows < values.len() {
                output.push_str(&" ".repeat(width.saturating_sub(value.len()) + 2));
            }
        }
        output.push('\n');
    }
    output
}

fn basename(path: &str) -> Option<&str> {
    path.rsplit('/').next().filter(|value| !value.is_empty())
}

fn glob_match(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let mut pattern_index = 0;
    let mut value_index = 0;
    let mut star = None;
    let mut star_value = 0;
    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == value[value_index] || pattern[pattern_index] == b'?')
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star = Some(pattern_index);
            pattern_index += 1;
            star_value = value_index;
        } else if let Some(star_index) = star {
            pattern_index = star_index + 1;
            star_value += 1;
            value_index = star_value;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

fn printf_bytes(arguments: &[String]) -> Vec<u8> {
    let Some(format) = arguments.first() else {
        return Vec::new();
    };
    let mut result = String::new();
    let mut values = arguments.iter().skip(1);
    let mut characters = format.chars();
    while let Some(character) = characters.next() {
        if character == '%' {
            if characters.next() == Some('s') {
                result.push_str(values.next().map_or("", String::as_str));
            } else {
                result.push('%');
            }
        } else if character == '\\' {
            match characters.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some(other) => result.push(other),
                None => result.push('\\'),
            }
        } else {
            result.push(character);
        }
    }
    result.into_bytes()
}

fn split_unquoted(value: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            current.push(character);
            escaped = true;
            continue;
        }
        match (quote, character) {
            (None, '\'' | '"') => {
                quote = Some(character);
                current.push(character);
            }
            (Some(active), character) if active == character => {
                quote = None;
                current.push(character);
            }
            (None, character) if character == delimiter => {
                parts.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    parts.push(current.trim().to_owned());
    parts
}

fn tokenize(segment: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut brace_expansion_allowed = true;
    let mut characters = segment.chars().peekable();
    while let Some(character) = characters.next() {
        if escaped {
            current.push(character);
            escaped = false;
            brace_expansion_allowed = false;
        } else if character == '\\' && quote != Some('\'') {
            escaped = true;
            brace_expansion_allowed = false;
        } else if let Some(active) = quote {
            if character == active {
                quote = None;
            } else {
                current.push(character);
            }
        } else {
            match character {
                '\'' | '"' => {
                    quote = Some(character);
                    brace_expansion_allowed = false;
                }
                ' ' | '\t' | '\r' | '\n' => {
                    if !current.is_empty() {
                        push_expanded_token(&mut tokens, &mut current, brace_expansion_allowed);
                        brace_expansion_allowed = true;
                    }
                }
                '>' | '<' => {
                    if !current.is_empty() {
                        push_expanded_token(&mut tokens, &mut current, brace_expansion_allowed);
                        brace_expansion_allowed = true;
                    }
                    if character == '>' && characters.peek() == Some(&'>') {
                        characters.next();
                        tokens.push(">>".to_owned());
                    } else {
                        tokens.push(character.to_string());
                    }
                }
                _ => current.push(character),
            }
        }
    }
    if escaped || quote.is_some() {
        return Err("learn: unterminated quote or escape".to_owned());
    }
    if !current.is_empty() {
        push_expanded_token(&mut tokens, &mut current, brace_expansion_allowed);
    }
    Ok(tokens)
}

fn push_expanded_token(tokens: &mut Vec<String>, token: &mut String, allowed: bool) {
    if allowed {
        let value = std::mem::take(token);
        tokens.extend(expand_braces(&value));
    } else {
        tokens.push(std::mem::take(token));
    }
}

fn expand_braces(value: &str) -> Vec<String> {
    let Some((start, end, body)) = first_brace_expression(value) else {
        return vec![value.to_owned()];
    };
    let alternatives = if body.contains("..") {
        brace_range(body)
    } else {
        body.split(',')
            .filter(|part| !part.is_empty())
            .map(str::to_owned)
            .collect()
    };
    if alternatives.is_empty() {
        return vec![value.to_owned()];
    }
    alternatives
        .into_iter()
        .flat_map(|alternative| {
            let prefix = &value[..start];
            let suffix = &value[end + 1..];
            expand_braces(&format!("{prefix}{alternative}{suffix}"))
        })
        .collect()
}

fn first_brace_expression(value: &str) -> Option<(usize, usize, &str)> {
    let start = value.find('{')?;
    let end = value[start + 1..].find('}')? + start + 1;
    Some((start, end, &value[start + 1..end]))
}

fn brace_range(body: &str) -> Vec<String> {
    let parts = body.split("..").collect::<Vec<_>>();
    if !(2..=3).contains(&parts.len()) {
        return Vec::new();
    }
    let Ok(start) = parts[0].parse::<i64>() else {
        return Vec::new();
    };
    let Ok(end) = parts[1].parse::<i64>() else {
        return Vec::new();
    };
    let default_step = if start <= end { 1 } else { -1 };
    let step = parts
        .get(2)
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(default_step);
    if step == 0 || (start < end && step < 0) || (start > end && step > 0) {
        return Vec::new();
    }
    let width = parts[0]
        .trim_start_matches('-')
        .len()
        .max(parts[1].trim_start_matches('-').len());
    let mut values = Vec::new();
    let mut value = start;
    while (step > 0 && value <= end) || (step < 0 && value >= end) {
        let rendered = if width > 1 {
            let sign = if value < 0 { "-" } else { "" };
            format!("{sign}{:0width$}", value.unsigned_abs(), width = width)
        } else {
            value.to_string()
        };
        values.push(rendered);
        if values.len() >= 4096 {
            break;
        }
        value = value.saturating_add(step);
        if value == i64::MAX || value == i64::MIN {
            break;
        }
    }
    values
}

type ParsedSegment = (Vec<String>, Option<String>, Option<(String, bool)>);

fn parse_segment(segment: &str) -> Result<ParsedSegment, String> {
    let tokens = tokenize(segment)?;
    let mut arguments = Vec::new();
    let mut stdin = None;
    let mut stdout = None;
    let mut index = 0;
    while index < tokens.len() {
        match tokens[index].as_str() {
            ">" | ">>" | "<" => {
                let operator = tokens[index].clone();
                let Some(path) = tokens.get(index + 1) else {
                    return Err(format!("learn: redirection {operator} needs a path"));
                };
                if operator == "<" {
                    stdin = Some(path.clone());
                } else {
                    stdout = Some((path.clone(), operator == ">>"));
                }
                index += 2;
            }
            value => {
                arguments.push(value.to_owned());
                index += 1;
            }
        }
    }
    Ok((arguments, stdin, stdout))
}

fn verify_virtual_expression(
    shell: &VirtualShell,
    expression: &VerifierExpr,
) -> Result<bool, RuntimeError> {
    match expression {
        VerifierExpr::All(children) => children
            .iter()
            .map(|child| verify_virtual_expression(shell, child))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| values.into_iter().all(std::convert::identity)),
        VerifierExpr::Any(children) => children
            .iter()
            .map(|child| verify_virtual_expression(shell, child))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| values.into_iter().any(std::convert::identity)),
        VerifierExpr::Not(child) => verify_virtual_expression(shell, child).map(|value| !value),
        VerifierExpr::Predicate(predicate) => match predicate {
            VerifierPredicate::FileMode { path, mode } => Ok(matches!(
                shell.fs.get(path.as_str()),
                Some(Node::File { mode: actual, .. }) if actual == mode
            )),
            VerifierPredicate::PathExists(path) => Ok(shell.fs.contains_key(path.as_str())),
            VerifierPredicate::PathMissing(path) => Ok(!shell.fs.contains_key(path.as_str())),
            VerifierPredicate::FileContains { path, needle } => {
                let Ok(contents) = shell.read_file_path(path.as_str()) else {
                    return Ok(false);
                };
                Ok(String::from_utf8_lossy(&contents).contains(needle))
            }
            VerifierPredicate::FileRead { path, expected } => {
                if !shell.read_files.contains(path.as_str()) {
                    return Ok(false);
                }
                let Ok(contents) = shell.read_file_path(path.as_str()) else {
                    return Ok(false);
                };
                Ok(String::from_utf8_lossy(&contents).contains(expected))
            }
            VerifierPredicate::FileEquals { path, expected } => {
                let Ok(contents) = shell.read_file_path(path.as_str()) else {
                    return Ok(false);
                };
                Ok(contents == *expected)
            }
            VerifierPredicate::FileTextEquals { path, expected } => {
                let Ok(contents) = shell.read_file_path(path.as_str()) else {
                    return Ok(false);
                };
                Ok(learn_core::text_contents_equal(&contents, expected))
            }
            VerifierPredicate::WorkingDirectory(expected) => Ok(shell.cwd == expected.as_str()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(setup: &[SetupAction]) -> WasmSession {
        start_wasm_session("test", setup, &EnvironmentSpec::default(), 80, 24)
            .expect("WASM session starts")
    }

    fn command(session: &WasmSession, value: &str) {
        session
            .write(format!("{value}\r").as_bytes())
            .expect("command is accepted");
    }

    #[test]
    fn embedded_guest_bundle_is_valid() {
        assert_eq!(wasm_bundle_version().expect("bundle validates"), 1);
    }

    #[test]
    fn shell_basics_and_completion_are_available() {
        let shell = VirtualShell::new(EnvironmentSpec::default());
        assert!(shell.command_allowed("ls"));
        assert!(shell.command_allowed("cat"));
        assert!(shell.command_allowed("exit"));
        assert!(is_known_command("grep"));
        assert!(!is_known_command("hi"));
        assert_eq!(
            shell.completion_candidates("l", true),
            vec![("less".to_owned(), false), ("ls".to_owned(), false),]
        );
        assert_eq!(
            completion_path("/home/learner/notes.txt", "~/", "/home/learner"),
            Some("~/notes.txt".to_owned())
        );
    }

    #[test]
    fn unknown_commands_are_not_reported_as_disabled() {
        let mut shell = VirtualShell::new(EnvironmentSpec::default());
        let result = shell.execute_line("hi");
        assert_eq!(result.status, 1);
        assert_eq!(
            String::from_utf8(result.output).expect("shell output is UTF-8"),
            "learn: command 'hi' not found\n"
        );
    }

    #[test]
    fn one_line_text_verification_accepts_echo_and_printf() {
        let verifier = VerifierExpr::Predicate(VerifierPredicate::FileTextEquals {
            path: SandboxPath::parse("~/status.txt").expect("path"),
            expected: b"ready".to_vec(),
        });
        let echo_session = session(&[]);
        command(&echo_session, "echo \"ready\" > status.txt");
        assert!(echo_session.verify(&verifier).expect("verification"));

        let printf_session = session(&[]);
        command(&printf_session, "printf ready > ~/status.txt");
        assert!(printf_session.verify(&verifier).expect("verification"));
    }

    #[test]
    fn command_help_newlines_always_return_to_column_zero() {
        let mut session = session(&[]);
        let mut reader = session.take_reader().expect("reader is available");
        session
            .write(b"printf --help\r\x04")
            .expect("help command is accepted");
        let mut output = String::new();
        reader
            .read_to_string(&mut output)
            .expect("session output is readable");
        let newline_count = output.bytes().filter(|byte| *byte == b'\n').count();
        let carriage_return_newline_count = output
            .as_bytes()
            .windows(2)
            .filter(|pair| *pair == b"\r\n")
            .count();
        assert_eq!(newline_count, carriage_return_newline_count);
        assert!(output.contains("USAGE: printf TEXT"));
    }

    #[test]
    fn file_read_verification_requires_a_real_read() {
        let path = SandboxPath::parse("~/welcome.txt").expect("path");
        let setup = [SetupAction::WriteFile {
            path: path.clone(),
            contents: b"Welcome to a real shell.".to_vec(),
        }];
        let session = session(&setup);
        let verifier = VerifierExpr::Predicate(VerifierPredicate::FileRead {
            path,
            expected: "Welcome to a real shell.".to_owned(),
        });

        assert!(!session.verify(&verifier).expect("verification is readable"));
        command(&session, "echo not-the-solution");
        assert!(!session.verify(&verifier).expect("verification is readable"));
        command(&session, "cat ~/welcome.txt");
        assert!(session.verify(&verifier).expect("verification is readable"));
    }

    #[test]
    fn exit_is_available_and_closes_every_session() {
        let mut session = session(&[]);
        let mut reader = session.take_reader().expect("reader is available");
        session.write(b"exit\r").expect("input is accepted");

        let mut output = String::new();
        reader
            .read_to_string(&mut output)
            .expect("session output is readable");
        assert!(output.contains("[session closed]"));
        assert_eq!(
            session.try_wait().expect("session status is readable"),
            Some(0)
        );

        let restricted = EnvironmentSpec {
            restriction: ToolRestrictionStrength::ExternalExecutableVisibility,
            ..EnvironmentSpec::default()
        };
        let mut shell = VirtualShell::new(restricted);
        assert_eq!(shell.execute_line("exit").status, 0);
    }

    #[test]
    fn tab_completion_inserts_a_command_into_the_session() {
        let mut session = session(&[]);
        let mut reader = session.take_reader().expect("reader is available");
        session.write(b"ec\thi\r\x04").expect("input is accepted");
        let mut output = String::new();
        reader
            .read_to_string(&mut output)
            .expect("session output is readable");
        assert!(output.contains("hi\r\n") || output.contains("hi\n"));
        assert!(output.contains("learner@learn"));
    }

    #[test]
    fn later_lesson_starts_without_repeating_the_fetch() {
        let mut session =
            start_wasm_session_quiet("later", &[], &EnvironmentSpec::default(), 80, 24)
                .expect("session starts");
        let mut reader = session.take_reader().expect("reader is available");
        session.write(b"\x04").expect("session closes");
        let mut output = String::new();
        reader.read_to_string(&mut output).expect("read output");
        assert!(!output.contains("LINUX SHELL DOJO"));
        assert!(output.contains("learner@learn"));
    }

    #[test]
    fn file_output_appears_before_the_following_prompt() {
        let setup = [SetupAction::WriteFile {
            path: SandboxPath::parse("~/welcome.txt").expect("path"),
            contents: b"Welcome to a real shell.".to_vec(),
        }];
        let mut session = session(&setup);
        let mut reader = session.take_reader().expect("reader is available");
        session
            .write(b"cat welcome.txt\r\x04")
            .expect("command accepted");
        let mut output = String::new();
        reader.read_to_string(&mut output).expect("read output");
        assert!(output.contains("Welcome to a real shell.\r\n\r\x1b]777"));
    }

    #[test]
    fn startup_fetch_and_starship_prompt_are_emitted_once() {
        let mut session = session(&[]);
        let mut reader = session.take_reader().expect("reader is available");
        session.write(b"\x04").expect("session closes");
        let mut output = String::new();
        reader
            .read_to_string(&mut output)
            .expect("session output is readable");
        assert_eq!(output.matches("LINUX SHELL DOJO").count(), 1);
        assert!(output.contains("embedded-posix"));
        assert!(output.contains("learner@learn  ~  ◈"));
        assert!(output.contains("\r\n❯ "));
    }

    #[test]
    fn prompt_editing_redraws_in_place_instead_of_appending_a_prompt() {
        let mut session = session(&[]);
        let mut reader = session.take_reader().expect("reader is available");
        session.write(b"abc\x7f\x04").expect("editing is accepted");
        let mut output = String::new();
        reader
            .read_to_string(&mut output)
            .expect("session output is readable");
        assert!(output.contains("\r\x1b[2K\x1b[1A\r\x1b[2K"));
    }

    #[test]
    fn clear_emits_screen_clear_and_cursor_home_controls() {
        let mut shell = VirtualShell::new(EnvironmentSpec::default());
        let result = shell.execute_line("clear");
        assert_eq!(result.status, 0);
        assert_eq!(result.output, b"\x1b[2J\x1b[H");
    }

    #[test]
    fn brace_ranges_expand_before_mkdir() {
        let mut shell = VirtualShell::new(EnvironmentSpec::default());
        assert_eq!(shell.execute_line("mkdir folder_{0..3}").status, 0);
        assert_eq!(shell.execute_line("mkdir {1..3}").status, 0);

        for name in [
            "folder_0", "folder_1", "folder_2", "folder_3", "1", "2", "3",
        ] {
            assert!(shell.fs.contains_key(&format!("/home/learner/{name}")));
        }
        assert!(!shell.fs.contains_key("/home/learner/folder_{0..3}"));
        assert!(!shell.fs.contains_key("/home/learner/{1..3}"));
    }

    #[test]
    fn quoted_or_escaped_braces_stay_literal() {
        let mut shell = VirtualShell::new(EnvironmentSpec::default());
        assert_eq!(shell.execute_line("mkdir '{1..3}'").status, 0);
        assert_eq!(shell.execute_line(r"mkdir \{4..6\}").status, 0);

        assert!(shell.fs.contains_key("/home/learner/{1..3}"));
        assert!(shell.fs.contains_key("/home/learner/{4..6}"));
        assert!(!shell.fs.contains_key("/home/learner/1"));
    }

    #[test]
    fn multi_match_completion_starts_a_fresh_prompt_line() {
        let mut session = session(&[]);
        let mut reader = session.take_reader().expect("reader is available");
        session.write(b"c\t\x04").expect("completion is accepted");
        let mut output = String::new();
        reader
            .read_to_string(&mut output)
            .expect("session output is readable");
        assert_eq!(output.matches("learner@learn").count(), 2);
        assert!(output.contains("cat"));
    }

    #[test]
    fn canonical_filesystem_and_navigation_commands_verify() {
        let setup = vec![SetupAction::CreateDir {
            path: SandboxPath::parse("~/practice").expect("path"),
        }];
        let session = session(&setup);
        command(&session, "cd ~/practice");
        assert!(
            session
                .verify(&VerifierExpr::Predicate(
                    VerifierPredicate::WorkingDirectory(
                        SandboxPath::parse("~/practice").expect("path")
                    )
                ))
                .expect("verification")
        );
        command(&session, "mkdir ~/project");
        command(&session, "touch ~/project/notes.txt");
        assert!(
            session
                .verify(&VerifierExpr::All(vec![
                    VerifierExpr::Predicate(VerifierPredicate::PathExists(
                        SandboxPath::parse("~/project").expect("path"),
                    )),
                    VerifierExpr::Predicate(VerifierPredicate::PathExists(
                        SandboxPath::parse("~/project/notes.txt").expect("path"),
                    )),
                ]))
                .expect("verification")
        );
    }

    #[test]
    fn command_history_survives_a_new_lesson_session() {
        let previous = session(&[]);
        previous.write(b"pwd\r").expect("command accepted");
        let next = session(&[]);
        next.set_command_history(previous.command_history())
            .expect("history restored");
        next.write(b"\x1b[A").expect("up arrow accepted");
        let state = next.state.lock().expect("session state");
        assert_eq!(state.input, b"pwd");
    }

    #[test]
    fn lesson_snapshot_restores_files_working_directory_and_history() {
        let previous = session(&[]);
        command(&previous, "mkdir ~/project");
        command(&previous, "cd ~/project");
        let snapshot = previous.snapshot().expect("session serializes");

        let restored = session(&[]);
        restored
            .restore_snapshot("test", &snapshot)
            .expect("matching lesson state restores");
        assert_eq!(restored.current_directory(), "/home/learner/project");
        assert!(
            restored
                .verify(&VerifierExpr::Predicate(VerifierPredicate::PathExists(
                    SandboxPath::parse("~/project").expect("path"),
                )))
                .expect("filesystem verifies")
        );
        restored.write(b"\x1b[A").expect("up arrow accepted");
        let state = restored.state.lock().expect("session state");
        assert_eq!(state.input, b"cd ~/project");
    }

    #[test]
    fn completion_only_lists_immediate_children() {
        let mut shell = VirtualShell::new(EnvironmentSpec::default());
        assert_eq!(shell.execute_line("mkdir reports").status, 0);
        assert_eq!(shell.execute_line("touch reports/one.log").status, 0);
        assert_eq!(
            shell.completion_candidates("repor", false),
            vec![("reports".to_owned(), true)]
        );
        assert_eq!(
            shell.completion_candidates("reports/o", false),
            vec![("reports/one.log".to_owned(), false)]
        );
    }

    #[test]
    fn pipes_and_redirection_use_the_virtual_filesystem() {
        let session = session(&[SetupAction::WriteFile {
            path: SandboxPath::parse("~/lines.txt").expect("path"),
            contents: b"apples\nLinux is a system\noranges".to_vec(),
        }]);
        command(&session, "cat ~/lines.txt | grep Linux > ~/match.txt");
        command(&session, "printf saved > ~/result.txt");
        assert!(
            session
                .verify(&VerifierExpr::All(vec![
                    VerifierExpr::Predicate(VerifierPredicate::FileContains {
                        path: SandboxPath::parse("~/match.txt").expect("path"),
                        needle: "Linux is a system".to_owned(),
                    }),
                    VerifierExpr::Predicate(VerifierPredicate::FileEquals {
                        path: SandboxPath::parse("~/result.txt").expect("path"),
                        expected: b"saved".to_vec(),
                    }),
                ]))
                .expect("verification")
        );
    }

    #[test]
    fn move_find_and_path_confinement_work() {
        let session = session(&[
            SetupAction::CreateDir {
                path: SandboxPath::parse("~/archive/2025").expect("path"),
            },
            SetupAction::WriteFile {
                path: SandboxPath::parse("~/loose-note.txt").expect("path"),
                contents: b"Keep me organized.".to_vec(),
            },
        ]);
        command(&session, "mv ~/loose-note.txt ~/archive/");
        command(&session, "find ~/archive -name loose-note.txt");
        assert!(
            session
                .verify(&VerifierExpr::All(vec![
                    VerifierExpr::Predicate(VerifierPredicate::PathExists(
                        SandboxPath::parse("~/archive/loose-note.txt").expect("path"),
                    )),
                    VerifierExpr::Predicate(VerifierPredicate::PathMissing(
                        SandboxPath::parse("~/loose-note.txt").expect("path"),
                    )),
                ]))
                .expect("verification")
        );
        command(&session, "touch ../../escape.txt");
        assert!(
            !session
                .verify(&VerifierExpr::Predicate(VerifierPredicate::PathExists(
                    SandboxPath::parse("~/escape.txt").expect("path"),
                )))
                .expect("verification")
        );
    }

    #[test]
    fn filename_globs_expand_before_copy() {
        let session = session(&[
            SetupAction::CreateDir {
                path: SandboxPath::parse("~/reports").expect("path"),
            },
            SetupAction::CreateDir {
                path: SandboxPath::parse("~/backup").expect("path"),
            },
            SetupAction::WriteFile {
                path: SandboxPath::parse("~/reports/one.log").expect("path"),
                contents: b"one".to_vec(),
            },
            SetupAction::WriteFile {
                path: SandboxPath::parse("~/reports/two.log").expect("path"),
                contents: b"two".to_vec(),
            },
            SetupAction::WriteFile {
                path: SandboxPath::parse("~/reports/readme.txt").expect("path"),
                contents: b"not a log".to_vec(),
            },
        ]);
        command(&session, "cp ~/reports/*.log ~/backup/");
        assert!(
            session
                .verify(&VerifierExpr::All(vec![
                    VerifierExpr::Predicate(VerifierPredicate::PathExists(
                        SandboxPath::parse("~/backup/one.log").expect("path"),
                    )),
                    VerifierExpr::Predicate(VerifierPredicate::PathExists(
                        SandboxPath::parse("~/backup/two.log").expect("path"),
                    )),
                    VerifierExpr::Predicate(VerifierPredicate::PathMissing(
                        SandboxPath::parse("~/backup/readme.txt").expect("path"),
                    )),
                ]))
                .expect("verification")
        );
    }
}
