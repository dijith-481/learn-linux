#![forbid(unsafe_code)]

//! Command-line entry point for the Learn Linux application.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use clap::{Parser, Subcommand};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use learn_challenge::compile_directory;
use learn_daemon::Daemon;
use learn_runtime::{
    WasmSession, default_rootfs_path, doctor, install_rootfs, start_wasm_session, verify_rootfs,
};

#[derive(Debug, Parser)]
#[command(
    name = "learn",
    version,
    about = "Learn Linux through real, verifiable shell challenges"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Launch the full-screen client.
    Tui,
    /// Run the daemon on the local Unix socket.
    Daemon,
    /// Attach a plain terminal to one real sandboxed challenge.
    Shell { challenge_id: String },
    /// Inspect host/runtime capabilities.
    Doctor {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Install or verify a prepared Linux rootfs.
    Runtime {
        #[command(subcommand)]
        command: RuntimeCommand,
    },
    /// Validate all built-in KDL content.
    Content {
        #[command(subcommand)]
        command: ContentCommand,
    },
    /// List the compiled skill/challenge path.
    Graph,
    /// Search the offline command reference.
    Docs {
        /// Natural-language or command-oriented query.
        query: String,
        /// Maximum number of results.
        #[arg(short, long, default_value_t = 5)]
        limit: usize,
    },
    /// Show the local progress file.
    Status,
    /// Reset one challenge's local progress.
    Reset { challenge_id: String },
    /// Print the version.
    Version,
}

#[derive(Debug, Subcommand)]
enum ContentCommand {
    /// Compile every built-in challenge.
    Check,
    /// Compile and print canonical solutions.
    Simulate {
        /// Simulate all built-in challenges.
        #[arg(long)]
        all: bool,
    },
}

#[derive(Debug, Subcommand)]
enum RuntimeCommand {
    /// Install a prepared rootfs directory as the active runtime.
    Install { source: PathBuf },
    /// Verify the active runtime rootfs.
    Verify,
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_ansi(false)
        .try_init()
        .ok();
    let cli = Cli::parse();
    let root = repository_root();
    match cli.command.unwrap_or(Command::Tui) {
        Command::Tui => {
            let daemon = Daemon::load(root.join("content/challenges"))?;
            learn_tui::run(&daemon, None).await?;
        }
        Command::Daemon => {
            let daemon = Daemon::load(root.join("content/challenges"))?;
            println!(
                "learn daemon listening on {}",
                learn_daemon::socket_path().display()
            );
            daemon.serve(learn_daemon::socket_path()).await?;
        }
        Command::Shell { challenge_id } => {
            let daemon = Daemon::load(root.join("content/challenges"))?;
            run_shell(&daemon, &challenge_id)?;
        }
        Command::Doctor { json } => {
            let report = doctor();
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                print!("{}", report.render());
            }
        }
        Command::Runtime { command } => match command {
            RuntimeCommand::Install { source } => {
                let destination = default_rootfs_path();
                install_rootfs(source, &destination)?;
                println!("installed runtime rootfs at {}", destination.display());
            }
            RuntimeCommand::Verify => {
                let rootfs = default_rootfs_path();
                verify_rootfs(&rootfs)?;
                println!("runtime rootfs OK: {}", rootfs.display());
            }
        },
        Command::Content { command } => match command {
            ContentCommand::Check => {
                let pack = compile_directory(root.join("content/challenges"))?;
                println!(
                    "content: {} challenges compiled; 0 errors",
                    pack.challenges.len()
                );
                for challenge in pack.challenges {
                    println!("  ✓ {}", challenge.spec.id);
                }
            }
            ContentCommand::Simulate { all } => {
                let pack = compile_directory(root.join("content/challenges"))?;
                if !all && pack.challenges.len() != 1 {
                    return Err("pass --all when simulating a content pack".into());
                }
                for challenge in pack.challenges {
                    let session = start_wasm_session(
                        challenge.spec.id.as_str(),
                        &challenge.spec.setup,
                        &challenge.spec.environment,
                        100,
                        30,
                    )?;
                    let mut solved = false;
                    println!("{}: simulating canonical solution", challenge.spec.id);
                    for command in challenge.spec.solution.commands {
                        println!("  $ {command}");
                        session.write(format!("{command}\r").as_bytes())?;
                        solved = session.verify(&challenge.spec.verifier)?;
                    }
                    if !solved {
                        return Err(format!(
                            "canonical solution did not verify for {}",
                            challenge.spec.id
                        )
                        .into());
                    }
                    println!("  ✓ verified in embedded WASM runtime");
                }
            }
        },
        Command::Graph => {
            let daemon = Daemon::load(root.join("content/challenges"))?;
            println!("Challenges:");
            for challenge in &daemon.content.challenges {
                println!("  {} — {}", challenge.spec.id, challenge.spec.meta.title);
            }
            println!("\nSkills:");
            for skill in daemon.graph.skills() {
                println!("  {} — {}", skill.id, skill.title);
            }
        }
        Command::Docs { query, limit } => {
            let daemon = Daemon::load(root.join("content/challenges"))?;
            for result in daemon.search_docs(&query, limit) {
                println!("{} — {}", result.document.id, result.document.title);
                println!("  {}", result.snippet);
            }
        }
        Command::Status => {
            let daemon = Daemon::load(root.join("content/challenges"))?;
            let progress = daemon.progress()?;
            println!(
                "completed: {}",
                progress
                    .iter()
                    .filter(|item| item.status == learn_core::CompletionStatus::Completed)
                    .count()
            );
            for item in &progress {
                println!(
                    "  {:?} {} (attempts: {})",
                    item.status, item.challenge_id, item.attempts
                );
            }
        }
        Command::Reset { challenge_id } => {
            let daemon = Daemon::load(root.join("content/challenges"))?;
            let id = learn_core::ChallengeId::new(challenge_id)?;
            daemon.reset_challenge(&id)?;
            println!("reset progress for {id}");
        }
        Command::Version => println!("learn {}", env!("CARGO_PKG_VERSION")),
    }
    Ok(())
}

fn run_shell(daemon: &Daemon, challenge_id: &str) -> Result<(), Box<dyn std::error::Error>> {
    let challenge = daemon
        .challenge(challenge_id)
        .ok_or_else(|| format!("unknown challenge {challenge_id}"))?
        .clone();
    let mut session = start_wasm_session(
        challenge_id,
        &challenge.spec.setup,
        &challenge.spec.environment,
        100,
        30,
    )?;
    daemon.start_challenge(challenge.spec.id.clone())?;
    let mut reader = session
        .take_reader()
        .ok_or("sandbox PTY reader was unavailable")?;
    let (output_sender, output_receiver) = mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if output_sender.send(buffer[..count].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    enable_raw_mode()?;
    let interactive_result = interactive_loop(&mut session, &output_receiver);
    disable_raw_mode()?;
    interactive_result?;
    let solved = session.verify(&challenge.spec.verifier)?;
    println!();
    if solved {
        daemon.complete_challenge(&challenge.spec.id)?;
        println!("✓ Solved: {}", challenge.spec.meta.title);
        println!("{}", challenge.spec.solution.concept);
    } else {
        println!("Session ended without reaching the verified challenge state.");
    }
    Ok(())
}

fn interactive_loop(
    session: &mut WasmSession,
    output_receiver: &mpsc::Receiver<Vec<u8>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stdout = io::stdout();
    loop {
        while let Ok(data) = output_receiver.try_recv() {
            stdout.write_all(&data)?;
            stdout.flush()?;
        }
        if session.try_wait()?.is_some() {
            while let Ok(data) = output_receiver.try_recv() {
                stdout.write_all(&data)?;
            }
            stdout.flush()?;
            break;
        }
        if event::poll(Duration::from_millis(20))? {
            match event::read()? {
                Event::Key(key) => {
                    if let Some(bytes) = key_bytes(key.code, key.modifiers) {
                        session.write(&bytes)?;
                    }
                }
                Event::Resize(columns, rows) => session.resize(columns, rows)?,
                _ => {}
            }
        }
    }
    Ok(())
}

fn key_bytes(code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    if modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(character) = code {
            let character = character.to_ascii_lowercase();
            if character.is_ascii_lowercase() {
                return Some(vec![character as u8 - b'a' + 1]);
            }
        }
    }
    Some(match code {
        KeyCode::Char(character) => character.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        _ => return None,
    })
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}
