#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, missing_docs)]

//! Portable WASM lesson runtime plus an optional Bubblewrap-backed backend.

mod wasm;

pub use wasm::{
    WASM_GUEST_ABI_VERSION, WasmSession, start_wasm_session, start_wasm_session_quiet,
    wasm_bundle_version,
};

use std::collections::BTreeSet;
use std::fmt::{self, Write as FmtWrite};
use std::fs;
use std::io;
use std::io::{Read, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use learn_core::{
    EnvironmentSpec, NetworkPolicy, SandboxPath, SetupAction, StateMode, ToolId,
    ToolRestrictionStrength, VerifierExpr, VerifierPredicate,
};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::Serialize;
use thiserror::Error;

/// Bubblewrap versions below this are rejected by policy.
pub const MIN_BWRAP_MAJOR: u32 = 0;
/// Minimum Bubblewrap minor version for the required security fix.
pub const MIN_BWRAP_MINOR: u32 = 12;

/// Return the default installed rootfs location.
pub fn default_rootfs_path() -> PathBuf {
    std::env::var_os("LEARN_ROOTFS")
        .map_or_else(
            || {
                std::env::var_os("XDG_CACHE_HOME").map_or_else(
                    || {
                        std::env::var_os("HOME")
                            .map_or_else(|| PathBuf::from("."), PathBuf::from)
                            .join(".cache")
                    },
                    PathBuf::from,
                )
            },
            PathBuf::from,
        )
        .join("learn/runtime/rootfs/current")
}

/// Diagnostic status for one runtime capability.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Check {
    /// Capability name.
    pub name: String,
    /// Whether the check passed.
    pub ok: bool,
    /// Human-readable detail.
    pub detail: String,
}

/// Runtime doctor report.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DoctorReport {
    /// Operating system.
    pub os: String,
    /// Kernel release.
    pub kernel: String,
    /// Bubblewrap version if discovered.
    pub bubblewrap_version: Option<String>,
    /// Individual checks.
    pub checks: Vec<Check>,
}

impl DoctorReport {
    /// True when the default embedded WASM runtime is available.
    pub fn usable(&self) -> bool {
        self.checks
            .iter()
            .find(|check| check.name == "wasm-bundle")
            .is_some_and(|check| check.ok)
    }

    /// True when the optional native Bubblewrap backend is available.
    pub fn native_usable(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.name.starts_with("native-"))
            && self
                .checks
                .iter()
                .filter(|check| check.name.starts_with("native-"))
                .all(|check| check.ok)
    }

    /// Render a concise human-readable report.
    pub fn render(&self) -> String {
        let mut output = String::from("Learn runtime doctor\n\n");
        if let Some(check) = self.checks.iter().find(|check| check.name == "wasm-bundle") {
            let marker = if check.ok { "[OK]" } else { "[FAIL]" };
            let _ = writeln!(output, "{marker} WASM lesson runtime: {}", check.detail);
        }
        if self.os == "linux" {
            let _ = writeln!(output, "[OK] Linux kernel: {}", self.kernel);
        } else {
            let _ = writeln!(output, "[OK] Host OS: {}", self.os);
        }
        if let Some(version) = &self.bubblewrap_version {
            let _ = writeln!(
                output,
                "{} Bubblewrap: {version}",
                if self.bubblewrap_ok() {
                    "[OK]"
                } else {
                    "[INFO]"
                }
            );
        } else {
            output.push_str("[INFO] Bubblewrap: not installed (optional native backend)\n");
        }
        for check in &self.checks {
            if check.name == "native-bubblewrap-version" || check.name == "wasm-bundle" {
                continue;
            }
            let _ = writeln!(
                output,
                "{} {}: {}\n",
                if check.ok {
                    "[OK]"
                } else if check.name.starts_with("native-") {
                    "[INFO]"
                } else {
                    "[FAIL]"
                },
                check.name,
                check.detail
            );
        }
        if self.usable() {
            output.push_str("\nPortable simulated Linux sessions are ready.\n");
        } else {
            output.push_str("\nThe embedded WASM lesson runtime is unavailable.\n");
        }
        output
    }

    fn bubblewrap_ok(&self) -> bool {
        self.checks
            .iter()
            .find(|check| check.name == "native-bubblewrap-version")
            .is_some_and(|check| check.ok)
    }
}

/// Discover Linux runtime capabilities without mutating the host.
pub fn doctor() -> DoctorReport {
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .map_or_else(|_| "unknown".to_owned(), |value| value.trim().to_owned());
    let version_output = Command::new("bwrap").arg("--version").output();
    let (bubblewrap_version, bubblewrap_check) = match version_output {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let ok = parse_bwrap_version(&version).is_some_and(|(major, minor)| {
                major > MIN_BWRAP_MAJOR || (major == MIN_BWRAP_MAJOR && minor >= MIN_BWRAP_MINOR)
            });
            (
                Some(version.clone()),
                Check {
                    name: "bubblewrap-version".to_owned(),
                    ok,
                    detail: if ok {
                        version
                    } else {
                        format!("{version}; requires Bubblewrap 0.12.0 or newer")
                    },
                },
            )
        }
        _ => (
            None,
            Check {
                name: "bubblewrap-version".to_owned(),
                ok: false,
                detail: "bwrap --version failed".to_owned(),
            },
        ),
    };
    let rootfs_path = default_rootfs_path();
    let rootfs_result = verify_rootfs(&rootfs_path);
    let wasm_check = wasm_check();
    let checks = vec![
        wasm_check,
        Check {
            name: "native-bubblewrap-version".to_owned(),
            ..bubblewrap_check
        },
        Check {
            name: "native-user-namespace".to_owned(),
            ok: namespace_probe("--unshare-user"),
            detail: "unprivileged user namespace probe".to_owned(),
        },
        Check {
            name: "native-mount-namespace".to_owned(),
            ok: namespace_probe("--unshare-all"),
            detail: "Bubblewrap namespace construction probe".to_owned(),
        },
        Check {
            name: "native-pid-namespace".to_owned(),
            ok: namespace_probe("--unshare-pid"),
            detail: "isolated process namespace probe".to_owned(),
        },
        Check {
            name: "native-network-namespace".to_owned(),
            ok: namespace_probe("--unshare-net"),
            detail: "isolated network namespace probe".to_owned(),
        },
        Check {
            name: "native-new-session".to_owned(),
            ok: bwrap_help_contains("--new-session"),
            detail: "required session isolation flag".to_owned(),
        },
        Check {
            name: "native-pty".to_owned(),
            ok: true,
            detail: "portable-pty is available".to_owned(),
        },
        Check {
            name: "native-rootfs".to_owned(),
            ok: rootfs_result.is_ok(),
            detail: rootfs_result.map_or_else(
                |error| error.to_string(),
                |()| format!("{} is valid", rootfs_path.display()),
            ),
        },
    ];
    DoctorReport {
        os: std::env::consts::OS.to_owned(),
        kernel,
        bubblewrap_version,
        checks,
    }
}

/// Return the fast startup report for the default portable backend.
pub fn portable_doctor() -> DoctorReport {
    DoctorReport {
        os: std::env::consts::OS.to_owned(),
        kernel: fs::read_to_string("/proc/sys/kernel/osrelease").map_or_else(
            |_| "not applicable".to_owned(),
            |value| value.trim().to_owned(),
        ),
        bubblewrap_version: None,
        checks: vec![wasm_check()],
    }
}

fn wasm_check() -> Check {
    match wasm_bundle_version() {
        Ok(version) => Check {
            name: "wasm-bundle".to_owned(),
            ok: version == WASM_GUEST_ABI_VERSION,
            detail: format!("embedded guest ABI v{version}; deterministic virtual Linux"),
        },
        Err(error) => Check {
            name: "wasm-bundle".to_owned(),
            ok: false,
            detail: error.to_string(),
        },
    }
}

/// Verify that a rootfs has the minimum files needed to start Bash.
pub fn verify_rootfs(rootfs: impl AsRef<Path>) -> Result<(), RuntimeError> {
    let rootfs = rootfs.as_ref();
    if !rootfs.is_dir() {
        return Err(RuntimeError::MissingRootfs(rootfs.to_path_buf()));
    }
    for required in ["bin/bash", "usr", "tmp", "home"] {
        if !rootfs.join(required).exists() {
            return Err(RuntimeError::RootfsInvalid {
                rootfs: rootfs.to_path_buf(),
                missing: required.to_owned(),
            });
        }
    }
    Ok(())
}

/// Install a prepared rootfs without replacing an existing installation.
pub fn install_rootfs(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
) -> Result<(), RuntimeError> {
    let source = source.as_ref();
    let destination = destination.as_ref();
    verify_rootfs(source)?;
    if destination.exists() {
        return Err(RuntimeError::RootfsDestinationExists(
            destination.to_path_buf(),
        ));
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(RuntimeError::Io)?;
    }
    copy_tree(source, destination)?;
    verify_rootfs(destination)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), RuntimeError> {
    fs::create_dir_all(destination).map_err(RuntimeError::Io)?;
    for entry in fs::read_dir(source).map_err(RuntimeError::Io)? {
        let entry = entry.map_err(RuntimeError::Io)?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).map_err(RuntimeError::Io)?;
        if metadata.file_type().is_symlink() {
            #[cfg(unix)]
            std::os::unix::fs::symlink(
                fs::read_link(&source_path).map_err(RuntimeError::Io)?,
                destination_path,
            )
            .map_err(RuntimeError::Io)?;
        } else if metadata.is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path).map_err(RuntimeError::Io)?;
            fs::set_permissions(&destination_path, metadata.permissions())
                .map_err(RuntimeError::Io)?;
        }
    }
    Ok(())
}

fn parse_bwrap_version(value: &str) -> Option<(u32, u32)> {
    let version = value.split_whitespace().last()?;
    let mut pieces = version.split('.');
    Some((pieces.next()?.parse().ok()?, pieces.next()?.parse().ok()?))
}

fn bwrap_help_contains(flag: &str) -> bool {
    Command::new("bwrap")
        .arg("--help")
        .output()
        .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).contains(flag))
}

fn namespace_probe(flag: &str) -> bool {
    let command = [
        "/usr/bin/true",
        "/bin/true",
        "/run/current-system/sw/bin/true",
    ]
    .into_iter()
    .find(|path| Path::new(path).exists())
    .unwrap_or("/bin/true");
    Command::new("bwrap")
        .args([
            flag,
            "--ro-bind",
            "/",
            "/",
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--",
            command,
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A compiled runtime session.
pub struct SandboxSession {
    /// Session ID.
    pub id: String,
    writer: Arc<Mutex<Box<dyn IoWrite + Send>>>,
    reader: Option<Box<dyn Read + Send>>,
    child: Box<dyn Child + Send + Sync>,
    master: Box<dyn MasterPty + Send>,
    home: PathBuf,
    working_directory: Arc<Mutex<Option<SandboxPath>>>,
    command_buffer: Arc<Mutex<Vec<u8>>>,
    read_files: Arc<Mutex<BTreeSet<SandboxPath>>>,
}

impl fmt::Debug for SandboxSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SandboxSession")
            .field("id", &self.id)
            .field("home", &self.home)
            .finish_non_exhaustive()
    }
}

impl SandboxSession {
    /// Read bytes currently available from the PTY.
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<usize, RuntimeError> {
        self.reader
            .as_mut()
            .ok_or(RuntimeError::ReaderTaken)?
            .read(buffer)
            .map_err(RuntimeError::Io)
    }

    /// Take ownership of the PTY reader for a forwarding thread.
    pub fn take_reader(&mut self) -> Option<Box<dyn Read + Send>> {
        self.reader.take()
    }

    /// Write raw terminal input bytes.
    pub fn write(&self, data: &[u8]) -> Result<(), RuntimeError> {
        self.writer
            .lock()
            .map_err(|_| RuntimeError::Poisoned)?
            .write_all(data)
            .map_err(RuntimeError::Io)?;
        record_shell_input(
            data,
            &self.command_buffer,
            &self.read_files,
            &self.working_directory,
        )
    }

    /// Resize the learner terminal.
    pub fn resize(&self, columns: u16, rows: u16) -> Result<(), RuntimeError> {
        self.master
            .resize(PtySize {
                rows,
                cols: columns,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| RuntimeError::Pty(error.to_string()))
    }

    /// Return whether the sandbox process has exited.
    pub fn try_wait(&mut self) -> Result<Option<i32>, RuntimeError> {
        self.child
            .try_wait()
            .map_err(|error| RuntimeError::Pty(error.to_string()))
            .map(|status| {
                status.map(|status| i32::try_from(status.exit_code()).unwrap_or(i32::MAX))
            })
    }

    /// Verify filesystem predicates against the private challenge home.
    pub fn verify(&self, expression: &VerifierExpr) -> Result<bool, RuntimeError> {
        let working_directory = self
            .working_directory
            .lock()
            .map_err(|_| RuntimeError::Poisoned)?
            .clone();
        let read_files = self.read_files.lock().map_err(|_| RuntimeError::Poisoned)?;
        verify_expression(
            &self.home,
            working_directory.as_ref(),
            &read_files,
            expression,
        )
    }
}

impl Drop for SandboxSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = fs::remove_dir_all(&self.home);
    }
}

/// Start a real Bash session inside a challenge rootfs.
#[allow(clippy::too_many_lines)]
pub fn start_session(
    rootfs: impl AsRef<Path>,
    challenge_id: &str,
    setup: &[SetupAction],
    environment: &EnvironmentSpec,
    columns: u16,
    rows: u16,
) -> Result<SandboxSession, RuntimeError> {
    let report = doctor();
    if !report.native_usable() {
        return Err(RuntimeError::Unavailable(report.render()));
    }
    validate_environment(environment)?;
    let rootfs = rootfs.as_ref();
    verify_rootfs(rootfs)?;
    let home = session_home(challenge_id)?;
    apply_setup(&home, setup)?;
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols: columns,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| RuntimeError::Pty(error.to_string()))?;
    let mut command = CommandBuilder::new("bwrap");
    command.args([
        "--clearenv",
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-net",
        "--ro-bind",
    ]);
    command.arg(rootfs);
    command.arg("/");
    command.args(["--proc", "/proc", "--dev", "/dev", "--bind"]);
    command.arg(&home);
    command.arg("/home/learner");
    append_tool_visibility(&mut command, rootfs, environment)?;
    let default_working_directory =
        SandboxPath::parse("~").map_err(|error| RuntimeError::InvalidPath(error.to_string()))?;
    let working_directory = Arc::new(Mutex::new(Some(default_working_directory)));
    let command_buffer = Arc::new(Mutex::new(Vec::new()));
    let read_files = Arc::new(Mutex::new(BTreeSet::new()));
    command.args([
        "--dir",
        "/tmp",
        "--chdir",
        "/home/learner",
        "--setenv",
        "HOME",
        "/home/learner",
        "--setenv",
        "USER",
        "learner",
        "--setenv",
        "LOGNAME",
        "learner",
        "--setenv",
        "SHELL",
        "/bin/bash",
        "--setenv",
        "TERM",
        "xterm-256color",
        "--setenv",
        "LANG",
        "C.UTF-8",
        "--setenv",
        "PS1",
        r"\[\e]777;LEARN_PWD;\w\a\]\w\$ ",
        "--setenv",
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
        "--setenv",
        "PWD",
        "/home/learner",
        "--setenv",
        "LEARN_CHALLENGE",
        challenge_id,
        "--",
        "/bin/bash",
        "--noprofile",
        "--norc",
        "-i",
    ]);
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| RuntimeError::Pty(error.to_string()))?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| RuntimeError::Pty(error.to_string()))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| RuntimeError::Pty(error.to_string()))?;
    Ok(SandboxSession {
        id: format!("{}-{}", challenge_id, unique_suffix()),
        writer: Arc::new(Mutex::new(writer)),
        reader: Some(Box::new(TrackingReader::new(reader, &working_directory))),
        child,
        master: pair.master,
        home,
        working_directory,
        command_buffer,
        read_files,
    })
}

fn validate_environment(environment: &EnvironmentSpec) -> Result<(), RuntimeError> {
    if environment.state != StateMode::Ephemeral {
        return Err(RuntimeError::UnsupportedStateMode(environment.state));
    }
    if environment.network != NetworkPolicy::None {
        return Err(RuntimeError::UnsupportedNetworkPolicy(environment.network));
    }
    Ok(())
}

fn record_shell_input(
    data: &[u8],
    command_buffer: &Arc<Mutex<Vec<u8>>>,
    read_files: &Arc<Mutex<BTreeSet<SandboxPath>>>,
    working_directory: &Arc<Mutex<Option<SandboxPath>>>,
) -> Result<(), RuntimeError> {
    let mut command_buffer = command_buffer.lock().map_err(|_| RuntimeError::Poisoned)?;
    let current_directory = working_directory
        .lock()
        .map_err(|_| RuntimeError::Poisoned)?
        .clone()
        .unwrap_or(
            SandboxPath::parse("~")
                .map_err(|error| RuntimeError::InvalidPath(error.to_string()))?,
        );
    for byte in data {
        match byte {
            b'\r' | b'\n' => {
                record_read_command(&command_buffer, &current_directory, read_files)?;
                command_buffer.clear();
            }
            0x03 => command_buffer.clear(),
            0x08 | 0x7f => {
                command_buffer.pop();
            }
            byte if !byte.is_ascii_control() => command_buffer.push(*byte),
            _ => {}
        }
    }
    Ok(())
}

fn record_read_command(
    command: &[u8],
    working_directory: &SandboxPath,
    read_files: &Arc<Mutex<BTreeSet<SandboxPath>>>,
) -> Result<(), RuntimeError> {
    let command = String::from_utf8_lossy(command);
    let mut tokens = command.split_ascii_whitespace();
    let Some(program) = tokens.next() else {
        return Ok(());
    };
    if program != "cat" && program != "less" {
        return Ok(());
    }
    let mut paths = Vec::new();
    for token in tokens {
        if token == "--" {
            continue;
        }
        if token.starts_with('-') || matches!(token, ">" | ">>" | "<") {
            continue;
        }
        let normalized = if token.starts_with('/') || token.starts_with('~') {
            token.to_owned()
        } else {
            format!("{}/{}", working_directory.as_str(), token)
        };
        if let Ok(path) = SandboxPath::parse(normalized) {
            paths.push(path);
        }
    }
    read_files
        .lock()
        .map_err(|_| RuntimeError::Poisoned)?
        .extend(paths);
    Ok(())
}

fn append_tool_visibility(
    command: &mut CommandBuilder,
    rootfs: &Path,
    environment: &EnvironmentSpec,
) -> Result<(), RuntimeError> {
    if environment.restriction == ToolRestrictionStrength::Guidance {
        return Ok(());
    }

    for directory in [
        "/bin",
        "/usr/bin",
        "/sbin",
        "/usr/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
    ] {
        command.args(["--tmpfs", directory]);
    }

    bind_executable(
        command,
        rootfs,
        &ToolId::new("bash").expect("static tool ID"),
    )?;
    for tool in &environment.allowed_tools {
        bind_executable(command, rootfs, tool)?;
    }
    Ok(())
}

fn bind_executable(
    command: &mut CommandBuilder,
    rootfs: &Path,
    tool: &ToolId,
) -> Result<(), RuntimeError> {
    if is_bash_builtin(tool.as_str()) {
        return Ok(());
    }
    let candidates = [
        (
            rootfs.join("bin").join(tool.as_str()),
            format!("/bin/{}", tool.as_str()),
        ),
        (
            rootfs.join("usr/bin").join(tool.as_str()),
            format!("/usr/bin/{}", tool.as_str()),
        ),
        (
            rootfs.join("sbin").join(tool.as_str()),
            format!("/sbin/{}", tool.as_str()),
        ),
        (
            rootfs.join("usr/sbin").join(tool.as_str()),
            format!("/usr/sbin/{}", tool.as_str()),
        ),
    ];
    let Some((source, target)) = candidates.into_iter().find(|(path, _)| path.is_file()) else {
        return Err(RuntimeError::MissingTool(tool.to_string()));
    };
    command.args(["--ro-bind"]);
    command.arg(source);
    command.arg(target);
    Ok(())
}

fn is_bash_builtin(tool: &str) -> bool {
    matches!(
        tool,
        ":" | "."
            | "alias"
            | "bg"
            | "bind"
            | "break"
            | "builtin"
            | "caller"
            | "cd"
            | "command"
            | "compgen"
            | "complete"
            | "continue"
            | "declare"
            | "dirs"
            | "disown"
            | "echo"
            | "enable"
            | "eval"
            | "exec"
            | "exit"
            | "export"
            | "fc"
            | "fg"
            | "getopts"
            | "hash"
            | "help"
            | "history"
            | "jobs"
            | "kill"
            | "let"
            | "local"
            | "logout"
            | "mapfile"
            | "popd"
            | "printf"
            | "pushd"
            | "pwd"
            | "read"
            | "readonly"
            | "return"
            | "set"
            | "shift"
            | "shopt"
            | "source"
            | "suspend"
            | "test"
            | "times"
            | "trap"
            | "type"
            | "typeset"
            | "ulimit"
            | "umask"
            | "unalias"
            | "unset"
            | "wait"
    )
}

fn session_home(challenge_id: &str) -> Result<PathBuf, RuntimeError> {
    let safe_id = challenge_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let home = std::env::temp_dir().join(format!("learn-session-{safe_id}-{}", unique_suffix()));
    fs::create_dir_all(&home).map_err(RuntimeError::Io)?;
    Ok(home)
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_micros())
}

fn apply_setup(home: &Path, setup: &[SetupAction]) -> Result<(), RuntimeError> {
    for action in setup {
        match action {
            SetupAction::CreateDir { path } => {
                fs::create_dir_all(host_path(home, path)?).map_err(RuntimeError::Io)?;
            }
            SetupAction::WriteFile { path, contents } => {
                let destination = host_path(home, path)?;
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(RuntimeError::Io)?;
                }
                fs::write(destination, contents).map_err(RuntimeError::Io)?;
            }
            SetupAction::SetMode { path, mode } => {
                let destination = host_path(home, path)?;
                let permissions = fs::metadata(&destination)
                    .map_err(RuntimeError::Io)?
                    .permissions();
                #[cfg(unix)]
                let permissions = {
                    let mut permissions = permissions;
                    use std::os::unix::fs::PermissionsExt;
                    permissions.set_mode(*mode);
                    permissions
                };
                #[cfg(not(unix))]
                let permissions = {
                    let mut permissions = permissions;
                    permissions.set_readonly(*mode & 0o222 == 0);
                    permissions
                };
                fs::set_permissions(destination, permissions).map_err(RuntimeError::Io)?;
            }
        }
    }
    Ok(())
}

fn host_path(home: &Path, path: &SandboxPath) -> Result<PathBuf, RuntimeError> {
    let relative = path
        .as_str()
        .strip_prefix("/home/learner")
        .ok_or_else(|| RuntimeError::InvalidPath(path.to_string()))?;
    Ok(home.join(relative.trim_start_matches('/')))
}

fn verify_expression(
    home: &Path,
    working_directory: Option<&SandboxPath>,
    read_files: &BTreeSet<SandboxPath>,
    expression: &VerifierExpr,
) -> Result<bool, RuntimeError> {
    match expression {
        VerifierExpr::All(children) => children
            .iter()
            .map(|child| verify_expression(home, working_directory, read_files, child))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| values.into_iter().all(std::convert::identity)),
        VerifierExpr::Any(children) => children
            .iter()
            .map(|child| verify_expression(home, working_directory, read_files, child))
            .collect::<Result<Vec<_>, _>>()
            .map(|values| values.into_iter().any(std::convert::identity)),
        VerifierExpr::Not(child) => {
            verify_expression(home, working_directory, read_files, child).map(|value| !value)
        }
        VerifierExpr::Predicate(predicate) => match predicate {
            VerifierPredicate::FileMode { path, mode } => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let metadata = match fs::metadata(host_path(home, path)?) {
                        Ok(metadata) => metadata,
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                            ) =>
                        {
                            return Ok(false);
                        }
                        Err(error) => return Err(RuntimeError::Io(error)),
                    };
                    let permissions = metadata.permissions();
                    Ok(permissions.mode() & 0o777 == *mode)
                }
                #[cfg(not(unix))]
                {
                    let _ = (path, mode);
                    Err(RuntimeError::Unavailable(
                        "native file-mode verification requires Unix".to_owned(),
                    ))
                }
            }
            VerifierPredicate::PathExists(path) => Ok(host_path(home, path)?.exists()),
            VerifierPredicate::PathMissing(path) => Ok(!host_path(home, path)?.exists()),
            VerifierPredicate::FileContains { path, needle } => {
                let Some(contents) = read_verifier_file(home, path)? else {
                    return Ok(false);
                };
                Ok(String::from_utf8_lossy(&contents).contains(needle))
            }
            VerifierPredicate::FileRead { path, expected } => {
                if !read_files.contains(path) {
                    return Ok(false);
                }
                let Some(contents) = read_verifier_file(home, path)? else {
                    return Ok(false);
                };
                Ok(String::from_utf8_lossy(&contents).contains(expected))
            }
            VerifierPredicate::FileEquals { path, expected } => {
                let Some(contents) = read_verifier_file(home, path)? else {
                    return Ok(false);
                };
                Ok(contents == *expected)
            }
            VerifierPredicate::FileTextEquals { path, expected } => {
                let Some(contents) = read_verifier_file(home, path)? else {
                    return Ok(false);
                };
                Ok(learn_core::text_contents_equal(&contents, expected))
            }
            VerifierPredicate::WorkingDirectory(expected) => {
                Ok(working_directory == Some(expected))
            }
        },
    }
}

fn read_verifier_file(home: &Path, path: &SandboxPath) -> Result<Option<Vec<u8>>, RuntimeError> {
    match fs::read(host_path(home, path)?) {
        Ok(contents) => Ok(Some(contents)),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound
                    | io::ErrorKind::PermissionDenied
                    | io::ErrorKind::IsADirectory
            ) =>
        {
            Ok(None)
        }
        Err(error) => Err(RuntimeError::Io(error)),
    }
}

struct TrackingReader {
    inner: Box<dyn Read + Send>,
    working_directory: Arc<Mutex<Option<SandboxPath>>>,
    scan_buffer: Vec<u8>,
}

impl TrackingReader {
    fn new(
        inner: Box<dyn Read + Send>,
        working_directory: &Arc<Mutex<Option<SandboxPath>>>,
    ) -> Self {
        Self {
            inner,
            working_directory: Arc::clone(working_directory),
            scan_buffer: Vec::new(),
        }
    }

    fn scan(&mut self, bytes: &[u8]) {
        const PREFIX: &[u8] = b"\x1b]777;LEARN_PWD;";
        self.scan_buffer.extend_from_slice(bytes);
        loop {
            let Some(start) = self
                .scan_buffer
                .windows(PREFIX.len())
                .position(|window| window == PREFIX)
            else {
                let keep = PREFIX.len().saturating_sub(1);
                if self.scan_buffer.len() > keep {
                    let drop_count = self.scan_buffer.len() - keep;
                    self.scan_buffer.drain(..drop_count);
                }
                return;
            };
            let value_start = start + PREFIX.len();
            let Some(end_offset) = self.scan_buffer[value_start..]
                .iter()
                .position(|byte| *byte == b'\x07')
            else {
                if start > 0 {
                    self.scan_buffer.drain(..start);
                }
                return;
            };
            let end = value_start + end_offset;
            let value = String::from_utf8_lossy(&self.scan_buffer[value_start..end]);
            if let Ok(path) = SandboxPath::parse(value.as_ref()) {
                if let Ok(mut current) = self.working_directory.lock() {
                    *current = Some(path);
                }
            }
            self.scan_buffer.drain(..=end);
        }
    }
}

impl Read for TrackingReader {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        if count > 0 {
            self.scan(&buffer[..count]);
        }
        Ok(count)
    }
}

/// Runtime error.
#[derive(Debug, Error)]
pub enum RuntimeError {
    /// Embedded WASM guest validation or execution failed.
    #[error("WASM runtime failed: {0}")]
    Wasm(String),
    /// A simulated shell command or filesystem operation failed.
    #[error("virtual shell failed: {0}")]
    Shell(String),
    /// Required sandbox capability is not available.
    #[error("sandbox runtime unavailable: {0}")]
    Unavailable(String),
    /// Rootfs directory is absent.
    #[error("runtime rootfs does not exist: {0}")]
    MissingRootfs(PathBuf),
    /// Rootfs is missing a required file or directory.
    #[error("runtime rootfs {rootfs} is missing {missing}")]
    RootfsInvalid {
        /// Rootfs being checked.
        rootfs: PathBuf,
        /// Required path that was absent.
        missing: String,
    },
    /// An installation already exists.
    #[error("runtime rootfs destination already exists: {0}")]
    RootfsDestinationExists(PathBuf),
    /// Path could not be mapped.
    #[error("invalid sandbox path: {0}")]
    InvalidPath(String),
    /// I/O failed.
    #[error("runtime I/O failed: {0}")]
    Io(#[source] std::io::Error),
    /// PTY operation failed.
    #[error("PTY operation failed: {0}")]
    Pty(String),
    /// A mutex was poisoned.
    #[error("runtime session lock was poisoned")]
    Poisoned,
    /// The PTY reader has already been handed to another owner.
    #[error("PTY reader has already been taken")]
    ReaderTaken,
    /// A challenge declared a tool that is absent from the runtime rootfs.
    #[error("runtime rootfs does not provide declared tool `{0}`")]
    MissingTool(String),
    /// The current runtime only implements fresh per-session homes.
    #[error("runtime does not yet support challenge state mode {0:?}")]
    UnsupportedStateMode(StateMode),
    /// Non-empty network policies are not yet available to challenge sessions.
    #[error("network policy {0:?} is not available to challenge sessions")]
    UnsupportedNetworkPolicy(NetworkPolicy),
}

#[cfg(test)]
mod tests {
    use super::*;
    use learn_core::{SandboxPath, SetupAction, VerifierExpr, VerifierPredicate};

    #[test]
    fn parses_bwrap_versions() {
        assert_eq!(parse_bwrap_version("bubblewrap 0.12.0"), Some((0, 12)));
        assert_eq!(parse_bwrap_version("bubblewrap 0.11.2"), Some((0, 11)));
    }

    #[test]
    fn bash_builtins_do_not_require_rootfs_executables() {
        assert!(is_bash_builtin("cd"));
        assert!(is_bash_builtin("printf"));
        assert!(!is_bash_builtin("grep"));
    }

    #[test]
    fn unsupported_environment_capabilities_fail_explicitly() {
        let persistent = EnvironmentSpec {
            state: StateMode::PersistentHome,
            ..EnvironmentSpec::default()
        };
        assert!(matches!(
            validate_environment(&persistent),
            Err(RuntimeError::UnsupportedStateMode(
                StateMode::PersistentHome
            ))
        ));
        let host_network = EnvironmentSpec {
            network: NetworkPolicy::HostNetwork,
            ..EnvironmentSpec::default()
        };
        assert!(matches!(
            validate_environment(&host_network),
            Err(RuntimeError::UnsupportedNetworkPolicy(
                NetworkPolicy::HostNetwork
            ))
        ));
    }

    #[test]
    fn restricted_visibility_binds_external_tools_but_not_builtins() {
        let rootfs = std::env::temp_dir().join(format!("learn-tool-rootfs-{}", unique_suffix()));
        fs::create_dir_all(rootfs.join("bin")).expect("create bin");
        fs::write(rootfs.join("bin/bash"), b"bash").expect("write bash");
        fs::write(rootfs.join("bin/cat"), b"cat").expect("write cat");
        let mut environment = EnvironmentSpec {
            restriction: ToolRestrictionStrength::ExternalExecutableVisibility,
            ..EnvironmentSpec::default()
        };
        environment
            .allowed_tools
            .insert(ToolId::new("cat").expect("tool ID"));
        environment
            .allowed_tools
            .insert(ToolId::new("cd").expect("tool ID"));
        let mut command = CommandBuilder::new("bwrap");
        append_tool_visibility(&mut command, &rootfs, &environment).expect("visibility plan");
        let arguments = command
            .get_argv()
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let expected = [
            "--ro-bind".to_owned(),
            rootfs.join("bin/cat").display().to_string(),
            "/bin/cat".to_owned(),
        ];
        assert!(arguments.windows(3).any(|window| window == expected));
        assert!(!arguments.iter().any(|argument| argument == "cd"));
        fs::remove_dir_all(rootfs).expect("remove tool rootfs");
    }

    #[test]
    fn current_doctor_never_claims_old_bwrap_is_safe() {
        let report = doctor();
        if report.bubblewrap_version.as_deref() == Some("bubblewrap 0.11.2") {
            assert!(!report.native_usable());
        }
    }

    #[test]
    fn portable_doctor_does_not_require_native_prerequisites() {
        let report = portable_doctor();
        assert!(report.usable());
        assert!(
            report
                .checks
                .iter()
                .all(|check| check.name == "wasm-bundle")
        );
    }

    #[test]
    fn setup_and_state_verification_use_only_the_private_home() {
        let home = std::env::temp_dir().join(format!("learn-runtime-test-{}", unique_suffix()));
        fs::create_dir_all(&home).expect("create test home");
        let notes = SandboxPath::parse("~/notes").expect("valid sandbox path");
        let note = SandboxPath::parse("~/notes/message.txt").expect("valid sandbox path");
        apply_setup(
            &home,
            &[
                SetupAction::CreateDir { path: notes },
                SetupAction::WriteFile {
                    path: note.clone(),
                    contents: b"hello from the challenge".to_vec(),
                },
            ],
        )
        .expect("setup succeeds");

        let expression = VerifierExpr::All(vec![
            VerifierExpr::Predicate(VerifierPredicate::PathExists(note.clone())),
            VerifierExpr::Predicate(VerifierPredicate::FileContains {
                path: note,
                needle: "challenge".to_owned(),
            }),
        ]);
        assert!(
            verify_expression(&home, None, &BTreeSet::new(), &expression)
                .expect("verification succeeds")
        );
        let absent = SandboxPath::parse("~/later.txt").expect("valid sandbox path");
        let unfinished = VerifierExpr::All(vec![
            VerifierExpr::Predicate(VerifierPredicate::FileEquals {
                path: absent.clone(),
                expected: b"ready\n".to_vec(),
            }),
            VerifierExpr::Predicate(VerifierPredicate::FileTextEquals {
                path: absent.clone(),
                expected: b"ready".to_vec(),
            }),
            VerifierExpr::Predicate(VerifierPredicate::FileContains {
                path: absent,
                needle: "ready".to_owned(),
            }),
        ]);
        assert!(
            !verify_expression(&home, None, &BTreeSet::new(), &unfinished)
                .expect("an incomplete lesson is a normal unsolved state")
        );
        assert!(!home.join("notes/message.txt").is_symlink());
        fs::remove_dir_all(home).expect("remove test home");
    }

    #[test]
    fn prompt_metadata_tracks_a_fragmented_working_directory() {
        let working_directory = Arc::new(Mutex::new(None));
        let mut reader = TrackingReader::new(
            Box::new(std::io::Cursor::new(Vec::<u8>::new())),
            &working_directory,
        );
        reader.scan(b"\x1b]777;LEARN_PWD;/home/lear");
        reader.scan(b"ner/practice\x07/home/learner$ ");
        let current = working_directory
            .lock()
            .expect("working directory lock")
            .clone();
        assert_eq!(
            current,
            Some(SandboxPath::parse("~/practice").expect("valid path"))
        );

        let expected = SandboxPath::parse("~/practice").expect("valid path");
        let expression = VerifierExpr::Predicate(VerifierPredicate::WorkingDirectory(expected));
        assert!(
            verify_expression(
                Path::new("/tmp/learn-runtime-test-home"),
                current.as_ref(),
                &BTreeSet::new(),
                &expression
            )
            .expect("verification succeeds")
        );
    }

    #[test]
    fn rootfs_validation_reports_the_first_missing_required_entry() {
        let rootfs = std::env::temp_dir().join(format!("learn-rootfs-test-{}", unique_suffix()));
        fs::create_dir_all(rootfs.join("bin")).expect("create rootfs");
        fs::create_dir_all(rootfs.join("tmp")).expect("create tmp");
        fs::create_dir_all(rootfs.join("home")).expect("create home");
        let error = verify_rootfs(&rootfs).expect_err("rootfs is incomplete");
        assert!(
            matches!(error, RuntimeError::RootfsInvalid { missing, .. } if missing == "bin/bash")
        );
        fs::remove_dir_all(rootfs).expect("remove test rootfs");
    }

    #[test]
    fn rootfs_installation_is_copy_only_and_verifiable() {
        let source = std::env::temp_dir().join(format!("learn-rootfs-source-{}", unique_suffix()));
        let destination =
            std::env::temp_dir().join(format!("learn-rootfs-destination-{}", unique_suffix()));
        fs::create_dir_all(source.join("bin")).expect("create bin");
        fs::create_dir_all(source.join("usr")).expect("create usr");
        fs::create_dir_all(source.join("tmp")).expect("create tmp");
        fs::create_dir_all(source.join("home")).expect("create home");
        fs::write(source.join("bin/bash"), b"prepared bash").expect("write bash");
        install_rootfs(&source, &destination).expect("install rootfs");
        verify_rootfs(&destination).expect("installed rootfs verifies");
        assert_eq!(
            fs::read(destination.join("bin/bash")).expect("read installed bash"),
            b"prepared bash"
        );
        assert!(matches!(
            install_rootfs(&source, &destination),
            Err(RuntimeError::RootfsDestinationExists(path)) if path == destination
        ));
        fs::remove_dir_all(source).expect("remove source");
        fs::remove_dir_all(destination).expect("remove destination");
    }
}
