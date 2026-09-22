# Linux Learning TUI — Implementation-Grade Architecture & Build Plan

> **Status:** Architecture baseline for implementation
> **Primary implementation language:** Rust
> **Primary platform for v1:** portable WASM simulation; optional native Linux backend
> **Primary shell:** versioned virtual POSIX shell over embedded WASM runtime
> **Primary UI:** Full-screen TUI
> **Architecture:** Persistent local daemon + replaceable clients
> **Content format:** KDL authored, strongly typed internally
> **Product character:** “Duolingo for Linux” pacing with OverTheWire-like authenticity
> **Last architecture verification:** 2026-09-22

> **Architecture amendment (2026-09-22): portable WASM-first runtime**
>
> The original plan below describes a Bubblewrap + Bash runtime as the
> canonical first implementation. That design is retained as an optional
> native backend, but it is no longer the default product path. The default
> must be a single self-contained executable with an embedded WASM guest
> contract and deterministic virtual Linux environment. This amendment
> supersedes any later instruction that makes Bubblewrap, a rootfs, Bash, or a
> host PTY a prerequisite for launching the app.
>
> The portable runtime owns a private virtual filesystem rooted at
> /home/learner, a shell line editor, cwd/prompt metadata, and the command
> semantics currently needed by the compiled curriculum: cd, pwd, printf,
> echo, mkdir, touch, cat, less, mv, cp with filename globs, rm, grep, find,
> ls, head, wc, pipes, and output/input redirection. Setup actions and state
> verifiers execute against that same virtual state, so a challenge cannot pass
> by printing a fake success message.
>
> The embedded guest bundle is validated at startup and identifies the
> versioned WASM ABI. The host implementation is deliberately deterministic
> and dependency-free; it is a Linux-semantic simulation, not a Linux kernel,
> and must not claim support for process namespaces, arbitrary ELF binaries,
> device files, signals, ownership, or distribution-specific Bash behavior.
> Content that needs those capabilities belongs behind the optional native
> backend and must declare that requirement.
>
> Required acceptance checks for this amendment are:
>
> - cargo run --offline -- content check
> - cargo run --offline -- content simulate --all
> - cargo test --workspace --offline
> - cargo clippy --workspace --all-targets --all-features --offline -- -D warnings
> - cargo build --workspace --release --offline
> - daemon protocol test opens a simulated session, executes a canonical
>   command, observes a completion event, and closes the session

---

## 0. Purpose of this document

This is not a brainstorm. It is the implementation contract for the first serious version of the project.

An implementation agent should be able to take this file, create the repository, implement the architecture in phases, run the acceptance commands, and prove that each phase works before moving to the next one.

The project is a local-first Linux learning environment for beginners. It gives the learner a real shell inside a lightweight isolated Linux environment, a modern full-screen TUI, short progressive challenges, local documentation with very fast fuzzy search, an underlying skill graph, and later an optional AI challenge generator.

The application must feel intentionally engineered rather than assembled from loosely connected generated code.

The learner must leave the application with skills that transfer to an ordinary Linux terminal. The application is allowed to provide guidance around Bash; it must not quietly replace Linux semantics with a toy shell and then call that “learning Linux”.

---

# 1. Product definition

## 1.1 One-sentence product definition

A modern local-first interactive Linux learning environment that puts a real Bash shell inside a disposable sandbox and teaches Linux through short state-verified challenges, contextual documentation, visual explanations, and a skill graph.

## 1.2 Product identity

The desired interaction model is:

- **Duolingo-like pacing**: short lessons, visible progression, approachable language, optional placement, checkpoints.
- **OverTheWire-like authenticity**: the learner actually uses a shell and can solve challenges creatively.
- **Modern Linux UI philosophy**: clean, minimal, keyboard-first, visually polished, colorful where useful, not retro-terminal cosplay and not cyberpunk decoration.
- **Real-world transfer**: commands and concepts should behave as they do on a normal Linux machine.

The application is not primarily a “CTF with beginner flags”. It is a Linux learning environment that can use puzzle-like challenges where they improve learning.

## 1.3 Primary audience

The system supports multiple entry points, but v1 content should be biased toward people with little or no command-line experience.

Expected learners:

1. Complete beginner who has never used a terminal.
2. User who knows `cd`, `ls`, `mkdir`, maybe `cat` but does not understand Linux as a system.
3. Developer who wants a structured Linux fundamentals path.

Onboarding should include a very short terminal introduction and an optional placement path.

## 1.4 Core learning goals

The first curriculum should repeatedly reinforce five ideas:

1. Basic Linux commands are tools that compose.
2. “Everything is a file” is a useful mental model, with appropriate nuance later.
3. Linux is conceptually simple in many places: processes, files, descriptors, permissions, pipes, namespaces.
4. The user has real freedom to solve problems in multiple ways.
5. Linux trusts the user; destructive power is real, which is why understanding matters.

## 1.5 Non-goals for v1

Do **not** make these v1 requirements:

- Native Windows Linux-semantic sandbox.
- Native macOS Linux-semantic sandbox.
- Browser-only Linux environment.
- AI-generated content.
- Multiplayer.
- Cloud accounts.
- Social leaderboard.
- Full distro management.
- A shell implementation written from scratch.
- A replacement for Bash.
- Heavy gamification, mascots, loot, currencies, daily-streak pressure.
- Arbitrary downloaded community challenge execution.
- Perfect security against a malicious local user intentionally attacking their own machine.

Architect for later portability, but make Linux v1 genuinely good.

---

# 2. Locked architecture decisions

These should be treated as defaults unless implementation evidence proves one is technically wrong.

## 2.1 Real Linux, not a fake shell

The canonical path uses:

```text
real PTY
  -> Bubblewrap sandbox
     -> isolated Linux root filesystem
        -> real Bash
```

Do not implement a parser that pretends to be a shell.

Do not emulate `ls`, `find`, `grep`, pipes, processes, permissions, signals, or redirections in Rust simply to gain portability.

Portability comes from multiple runtime backends later, not from weakening the canonical Linux backend.

## 2.2 Bash is canonical

Bash is the teaching reference shell for the initial curriculum.

A future portable/assist backend may use **Brush**, whose current Rust core is embeddable and focuses on Bash/POSIX compatibility. That backend must be explicitly labeled as a compatibility mode, not silently substituted for canonical Linux lessons.

## 2.3 Daemon/client split

The backend is a persistent local daemon. Clients connect using a small typed protocol.

Initial clients:

- full-screen TUI — default
- CLI control surface — first-class

Later clients:

- local web UI
- desktop GUI
- remote display client if intentionally added

The frontend must not know Bubblewrap arguments, rootfs paths, or challenge verification details.

## 2.4 Skill graph, not a hard linear course

The internal curriculum is a directed graph.

The default learner experience may still look like a simple path. A beginner should not be forced to understand a giant graph before learning `pwd`.

The graph enables:

- prerequisites
- branches
- alternate challenge choices
- placement
- reinforcement
- checkpoints
- later adaptive selection

## 2.5 Creative solutions are accepted

Verification should prefer final state over command history.

If the learner solves a challenge with a clever command that was not the intended teaching route, the challenge should pass.

After completion, the UI can show:

- the intended concept
- the canonical solution
- why it works
- useful alternative approaches

Command restrictions are pedagogical constraints, not a security proof.

## 2.6 Challenge-defined capabilities

Every challenge explicitly declares what it needs:

- tools
- network
- writable paths
- process features
- user/permission features
- root-like teaching mode
- persistent state mode
- verifier capabilities

The challenge compiler must reject content that requests a capability unsupported by the active runtime backend.

## 2.7 Local-first

Everything required for the normal course should work without the internet after installation.

Internet-backed docs or AI may exist later as optional additions.

## 2.8 Visuals are explanatory, not decorative noise

Visualizations should react to actual learning concepts:

- data moving through a pipe
- process tree changes
- permission bits
- filesystem tree
- signals
- file descriptors
- network sockets

They must be globally disableable.

---

# 3. Coding philosophy — mandatory for implementation agents

This section is part of the architecture, not optional style advice.

## 3.1 Rust style

Prefer:

- explicit enums
- newtypes for important IDs
- `Result<T, E>` for fallible behavior
- `Option<T>` for legitimate absence
- small pure functions
- iterator transformations where they improve clarity
- immutable values by default
- exhaustive `match`
- typed state transitions
- concrete types until polymorphism is actually needed
- errors that describe the domain
- compiler-enforced invariants

Avoid:

- inheritance-shaped trait hierarchies
- “manager”, “service”, “repository”, “factory” layers with no real semantic boundary
- `Box<dyn Trait>` simply to look extensible
- global mutable state
- stringly typed domain states
- booleans that represent multi-state domain concepts
- needless `Arc<Mutex<_>>`
- clone-heavy ownership avoidance
- `unwrap()` in non-test runtime paths
- `expect()` except for genuinely compile-time/static invariants with a reason
- generic frameworks inside the application
- premature micro-optimizations
- abstractions created because a future implementation *might* exist

## 3.2 Functional core / imperative shell

Keep transformations pure where possible.

Examples that should be pure:

```text
KDL source
 -> parsed AST
 -> ChallengeSpec
 -> validation
 -> CompiledChallenge
 -> SandboxPlan
```

```text
AppState + UiEvent
 -> AppState + Effects
```

```text
SkillGraph + Progress
 -> EligibleChallenges
```

Side effects belong near boundaries:

- filesystem
- Bubblewrap process creation
- PTY I/O
- socket I/O
- persistence
- terminal rendering

## 3.3 Unsafe policy

All first-party crates begin with:

```rust
#![forbid(unsafe_code)]
```

Dependencies may internally use unsafe where their implementation requires it; that does not authorize unsafe in project code.

If first-party unsafe ever becomes necessary:

1. first provide a benchmark showing the real bottleneck;
2. show why safe alternatives are insufficient;
3. isolate unsafe into the smallest possible crate/module;
4. document the invariant at every unsafe boundary;
5. add dedicated tests/fuzzing;
6. require an explicit architecture decision record.

Until all six occur, first-party unsafe is rejected.

## 3.4 Error style

Library crates should expose typed errors.

Preferred:

```rust
#[derive(Debug, thiserror::Error)]
pub enum ChallengeCompileError {
    #[error("unknown skill `{0}`")]
    UnknownSkill(SkillId),
    #[error("tool `{tool}` is unavailable in runtime pack `{pack}`")]
    MissingTool { tool: ToolId, pack: PackId },
    #[error("verifier `{kind}` requires unsupported capability `{capability}`")]
    UnsupportedVerifier {
        kind: VerifierKind,
        capability: ProbeCapability,
    },
}
```

Do not erase all errors into a generic dynamic error inside core crates.

At the binary entrypoint it is acceptable to format multiple typed errors for the user.

## 3.5 Async policy

Do not make everything async.

Use async for:

- daemon socket I/O
- client socket I/O
- event fan-out
- timers
- process/session orchestration where it genuinely helps

Keep parsing, validation, graph operations, challenge compilation, ranking, and most state transitions synchronous and deterministic.

## 3.6 Dependency policy

Every runtime dependency must answer at least one of these:

- does it implement a hard platform primitive we should not write ourselves?
- does it provide a mature algorithm we would otherwise maintain?
- does it materially improve correctness?
- does it eliminate a substantial amount of high-risk code?

Dependencies are not rejected merely because they exist; unnecessary dependencies are rejected.

No custom implementation of PTYs, terminal escape parsing, Protobuf, cryptographic hashing, or Linux namespace setup should be written simply to save a dependency.

---

# 4. Top-level system architecture

```text
                                ┌──────────────────────┐
                                │      learn CLI       │
                                │  doctor / content /  │
                                │  reset / tui / web   │
                                └──────────┬───────────┘
                                           │
                                   Protobuf frames
                                           │
                         ┌─────────────────▼─────────────────┐
                         │          local daemon             │
                         │                                   │
                         │  session orchestration            │
                         │  challenge engine                 │
                         │  progress engine                  │
                         │  docs search                      │
                         │  persistence                      │
                         └───────┬───────────┬───────────────┘
                                 │           │
                       compiled  │           │ progress/docs
                       challenge │           │
                                 │           │
                    ┌────────────▼───┐   ┌───▼────────────┐
                    │ Linux runtime  │   │ local storage  │
                    │                │   │                │
                    │ Bubblewrap     │   │ progress DB    │
                    │ PTY            │   │ caches         │
                    │ rootfs store   │   │ profiles       │
                    └───────┬────────┘   └────────────────┘
                            │
                       real Bash
                            │
                    learner commands
```

The TUI is a client, not the application core.

```text
TUI client
├── terminal renderer
├── challenge pane
├── docs/search pane
├── skill/progress views
└── visualizer

NO:
├── Bubblewrap policy
├── rootfs assembly
├── verification authority
└── persistent progress authority
```

---

# 5. Repository layout

Do not explode the repository into dozens of crates.

Start with this:

```text
/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── flake.nix
├── README.md
├── ARCHITECTURE.md                  # later extracted summary if useful
├── proto/
│   └── learn.proto
├── crates/
│   ├── learn-core/                  # pure domain model + skill graph
│   ├── learn-challenge/             # KDL parsing/validation/compilation
│   ├── learn-protocol/              # protobuf + framing
│   ├── learn-runtime/               # PTY + Linux sandbox execution
│   ├── learn-docs/                  # docs ingestion + fuzzy search
│   ├── learn-daemon/                # orchestration + persistence + IPC
│   └── learn-tui/                   # ratatui client
├── app/
│   └── learn/                       # distributable binary
├── content/
│   ├── curriculum.kdl
│   ├── challenges/
│   ├── fixtures/
│   └── docs/
├── runtime/
│   ├── rootfs.kdl
│   ├── package-lock.kdl
│   └── shell/
│       ├── bashrc
│       └── profile
├── tests/
│   ├── scenarios/
│   ├── golden/
│   └── fixtures/
├── benches/
└── xtask/
```

### Why these crate boundaries exist

`learn-core`
: Pure domain types. Must not depend on Tokio, Ratatui, Bubblewrap, redb, PTY libraries, or KDL syntax.

`learn-challenge`
: Converts authoring documents into validated domain structures and compiled plans.

`learn-protocol`
: Owns wire compatibility. Keeps generated Protobuf types out of domain logic.

`learn-runtime`
: Owns operating-system execution. It can depend on PTY/process/platform crates.

`learn-docs`
: Owns indexing and ranking. It should be independently benchmarkable.

`learn-daemon`
: Coordinates the pure parts and side effects. Owns session lifetime and persistence.

`learn-tui`
: UI only.

`app/learn`
: CLI parsing, process startup, daemon launch, command routing.

Do not create `learn-storage`, `learn-utils`, `learn-common`, `learn-services`, etc. until a real independent boundary appears.

---

# 6. Distribution model

Use one primary executable initially:

```text
learn
```

Subcommands:

```text
learn                       # launch default TUI
learn tui
learn daemon                # normally auto-started
learn doctor
learn status
learn reset <challenge>
learn graph
learn content check
learn content compile
learn content simulate
learn runtime install
learn runtime verify
learn version
```

A single executable can spawn another copy of itself as the daemon process. This preserves process separation without duplicating most Rust code across distributable binaries.

If packaging later benefits from a distinct `learn-daemon` binary, the crate architecture already permits that split.

---

# 7. IPC protocol

## 7.1 Why Protobuf without gRPC

Use Protobuf for stable typed messages, but do not introduce gRPC for a local Unix socket when simple framed messages are sufficient.

Transport for Linux v1:

```text
Unix domain socket
 + length-delimited frames
 + Protobuf payload
```

Suggested implementation:

- `prost`
- `tokio-util::codec::LengthDelimitedCodec`
- `bytes`

The Protobuf schema is the wire contract; domain structs remain hand-written Rust types.

## 7.2 Socket location

Linux:

```text
$XDG_RUNTIME_DIR/learn/daemon.sock
```

Fallback only if runtime dir is unavailable:

```text
/tmp/learn-$UID/daemon.sock
```

Requirements:

- daemon-owned directory mode `0700`
- socket accessible only to current user
- stale socket detection
- daemon pid metadata
- version handshake before normal requests

## 7.3 Message model

Use a small envelope:

```proto
syntax = "proto3";
package learn.v1;

message Envelope {
  uint32 protocol_version = 1;
  uint64 message_id = 2;

  oneof body {
    Request request = 10;
    Response response = 11;
    Event event = 12;
    Cancel cancel = 13;
  }
}
```

Requests should be domain-shaped rather than path-shaped RPC strings.

Example:

```proto
message Request {
  oneof kind {
    HelloRequest hello = 1;
    OpenSessionRequest open_session = 2;
    TerminalInput terminal_input = 3;
    ResizeTerminal resize_terminal = 4;
    SubmitAnswer submit_answer = 5;
    RequestHint request_hint = 6;
    SearchDocsRequest search_docs = 7;
    GetChallengeRequest get_challenge = 8;
    GetGraphRequest get_graph = 9;
    ResetChallengeRequest reset_challenge = 10;
  }
}
```

Events:

```proto
message Event {
  oneof kind {
    TerminalOutput terminal_output = 1;
    ChallengeStateChanged challenge_state_changed = 2;
    ProgressChanged progress_changed = 3;
    SessionEnded session_ended = 4;
    HintUnlocked hint_unlocked = 5;
  }
}
```

## 7.4 Protocol rules

1. Unknown fields are tolerated according to Protobuf semantics.
2. Unknown required conceptual features are rejected during handshake.
3. Protocol version changes only for incompatible wire/semantic changes.
4. Requests receive exactly one terminal response unless explicitly documented as streaming.
5. Terminal output is delivered as event chunks.
6. Terminal bytes stay bytes. Do not force PTY data into UTF-8 strings.
7. Frame size has a strict maximum.
8. Malformed frames close the connection after a structured log entry.
9. Protocol parsing must be fuzzed.
10. Client disconnection must never leave a sandbox process orphaned indefinitely.

## 7.5 Protocol framing limits

Initial frame maximum:

```text
8 MiB
```

Normal terminal frames should be far smaller. The large ceiling leaves room for docs responses and future structured payloads while preventing unbounded allocation from a corrupt client.

## 7.6 Handshake

Client sends:

```text
client protocol version
client kind: tui | cli | web
client version
supported optional features
```

Daemon returns:

```text
server protocol version
server version
runtime status
content pack version
supported features
```

If incompatible, fail with an actionable message rather than hanging.

---

# 8. Daemon lifecycle

## 8.1 Startup

`learn tui` performs:

```text
try connect
  ├─ success -> handshake -> continue
  └─ failure
      -> acquire daemon startup lock
      -> spawn `learn daemon`
      -> wait for socket readiness
      -> handshake
```

No shell script wrapper is required.

## 8.2 Daemon state

Conceptually:

```rust
struct DaemonState {
    runtime: RuntimeState,
    curriculum: Curriculum,
    docs: DocsIndex,
    progress: ProgressStore,
    sessions: SessionTable,
}
```

Do not place all of this under one giant async mutex.

Each subsystem owns its mutable state. Immutable curriculum/docs indices should be shared read-only.

## 8.3 Session lifecycle

```text
Created
  -> Preparing
  -> Running
  -> Completed
  -> Closing
  -> Closed

Any pre-closed state
  -> Failed(reason)
```

Represent this with an enum, not booleans such as `is_running`, `is_complete`, `failed`.

Each session owns:

- `SessionId`
- `ChallengeId`
- compiled challenge
- sandbox handle
- PTY handle
- controlling client
- terminal dimensions
- telemetry summary
- verifier state

## 8.4 Disconnect behavior

Initial behavior:

- terminal session has one controlling client;
- when the controlling client exits normally, sandbox closes;
- unexpected disconnect gets a short grace period;
- if not reattached, kill the session cleanly;
- daemon must reap all descendants.

Full detachable persistent sessions can be added later when there is a concrete use case.

---

# 9. Linux sandbox architecture

## 9.1 Runtime primitive

Use **Bubblewrap >= 0.12.0**.

Do not support older Bubblewrap versions because 0.12.0 fixed a sandbox escape involving absolute symlink resolution during sandbox setup.

The runtime doctor must verify the discovered binary version before allowing sandbox startup.

## 9.2 Why Bubblewrap

It provides the Linux primitives needed without introducing Docker/Podman as a user-facing dependency:

- mount namespace
- user namespace
- PID namespace
- IPC namespace
- network namespace
- UTS namespace
- controlled bind mounts
- private `/proc`
- minimal `/dev`
- environment construction

Bubblewrap is a low-level sandbox construction primitive, not a complete policy. **Our compiled SandboxPlan is the policy.**

## 9.3 Baseline namespace policy

Baseline challenge should compile toward behavior equivalent to:

```text
new user namespace
new mount namespace
new pid namespace
new ipc namespace
new uts namespace
new cgroup namespace when possible
new network namespace by default
new session
private procfs
minimal dev
clear environment
readonly system layers
writable challenge workspace
writable learner home only if challenge state policy allows it
```

Important Bubblewrap safety rule:

- use `--new-session` unless a stronger relevant TTY/seccomp arrangement replaces the need;
- never bind host D-Bus sockets into the learner sandbox;
- never bind the user's real home directory;
- never expose Docker/Podman sockets;
- never expose SSH agent sockets by default;
- never expose Wayland/X11 sockets by default.

## 9.4 Environment sanitization

Start with a cleared environment and explicitly set only known values:

```text
HOME=/home/learner
USER=learner
LOGNAME=learner
SHELL=/bin/bash
PATH=<compiled tool path>
TERM=<client terminal type>
LANG=C.UTF-8
LC_ALL=C.UTF-8
LEARN_CHALLENGE=<id>
```

Do not leak:

- host tokens
- host API keys
- SSH variables
- cloud credentials
- host PATH
- host HOME
- editor/plugin environment

## 9.5 Root filesystem model

### Bootstrap

Use a version-pinned Alpine minirootfs as a small bootstrapping base.

As of 2026-09-22, Alpine 3.24.2 x86-64 minirootfs is approximately 4 MiB compressed.

### Critical rule

Do **not** confuse “small Alpine base” with “complete canonical Linux teaching environment under 10 MiB”.

Alpine defaults to musl + BusyBox. Many learners will later use GNU-heavy desktop/server distributions. Therefore the teaching runtime should deliberately add GNU-compatible tools for lessons where semantics/options matter.

### Rootfs packages

Initial tool families:

```text
foundation
├── bash
├── coreutils
├── findutils
├── grep
├── sed
├── gawk
├── diffutils
├── file
└── less

process
├── procps-ng
├── util-linux
└── psmisc

permissions-users
├── shadow
└── acl (later if needed)

network
├── iproute2
├── iputils
├── curl (only selected challenges)
└── busybox-extras/netcat variant only if deliberately taught

modern-tools (later)
├── ripgrep
├── fd
├── bat
└── optional others
```

Do not install every utility into every visible challenge.

## 9.6 Reproducible rootfs manifest

Maintain:

```text
runtime/rootfs.kdl
runtime/package-lock.kdl
```

The lock includes:

- Alpine release
- architecture
- repository URLs
- package name
- exact package version
- package checksum
- generated rootfs content hash

Rootfs build must be deterministic enough that CI can verify package/hash drift.

A content hash should use BLAKE3.

## 9.7 Rootfs cache

Suggested Linux layout:

```text
$XDG_CACHE_HOME/learn/runtime/
├── blobs/
├── rootfs/<content-hash>/
└── manifests/
```

Immutable rootfs content is shared across sessions.

Writable session data lives elsewhere.

## 9.8 Tool restrictions

Challenge authors may declare visible external commands.

Example:

```kdl
tools {
    allow "ls"
    allow "cat"
    allow "find"
    deny "grep"
    deny "rg"
}
```

### Required semantics

- Restricting a tool should make the external executable unavailable through normal and absolute paths in the challenge view.
- Do not implement restrictions only as `PATH` filtering if the hidden executable remains reachable as `/usr/bin/grep`.
- Construct the visible `/bin` and `/usr/bin` view from approved executable entries and their required runtime closure.
- Runtime libraries may remain available read-only.

### Honest limitation

Bash itself has builtins and a programming language.

A challenge that hides `grep` must **not** assume the learner is incapable of implementing equivalent filtering with Bash loops, `case`, parameter expansion, or another exposed tool.

Creative shell-only solutions are valid unless the curriculum explicitly uses a special restricted-shell exercise later.

Tool restrictions guide learning; they are not DRM.

## 9.9 Network policy

Challenge schema chooses one:

```rust
enum NetworkPolicy {
    None,
    Loopback,
    HostNetwork,
    // Later: isolated virtual topology
}
```

Default is `None`.

Do not allow arbitrary internet access merely because a command such as `curl` exists.

Network-enabled challenges must explain why network is required.

Later, networking lessons should prefer deterministic local services in isolated namespaces over public internet dependencies.

## 9.10 State modes

```rust
enum StateMode {
    Ephemeral,
    PersistentHome,
    PersistentWorld,
}
```

`Ephemeral`
: Challenge workspace is recreated from its seed on restart.

`PersistentHome`
: Learner home survives across compatible challenges; challenge-specific world resets.

`PersistentWorld`
: Explicitly selected long-lived environment for advanced tracks.

Default beginner challenges should use either `Ephemeral` or `PersistentHome`.

## 9.11 Reset behavior

Every challenge has deterministic reset.

A reset must not depend on undoing user changes one by one.

Correct model:

```text
discard mutable challenge world
 -> recreate from immutable fixture + typed setup actions
```

Persistent user home is only reset when the challenge or user explicitly asks for it.

## 9.12 Resource exhaustion

Namespace isolation alone does not stop:

- fork bombs
- CPU loops
- memory exhaustion
- excessive file creation

Therefore model resource protection separately:

```rust
enum ResourceGuard {
    CgroupV2(CgroupLimits),
    SystemdUserScope(CgroupLimits),
    Unavailable,
}
```

Initial limits should support:

- max processes/tasks
- max memory
- CPU quota where available
- optional temporary-storage quota later

`learn doctor` reports an isolation grade.

Example:

```text
filesystem isolation   OK
pid isolation          OK
network isolation      OK
user namespace         OK
resource guard         OK (systemd user scope)
community-safe mode    ELIGIBLE
```

If resource guarding is unavailable:

- official trusted beginner challenges may still run;
- arbitrary community packs must not run;
- arbitrary AI-generated executable setup must not run;
- UI must not claim complete hostile-code isolation.

## 9.13 Users and sudo lessons

Do not fake full Unix multi-user semantics in the earliest phase.

Bubblewrap user namespaces can present a custom UID/GID, but multiple realistic UID mappings need additional mapping support and vary by host configuration.

Plan progression:

### Stage A
Teach:

- current user
- numeric UID/GID concepts
- file mode bits
- ownership vocabulary
- root concept

using controlled fixtures that do not require a true multi-user login system.

### Stage B
Add a runtime capability:

```text
multi_uid
```

only after the implementation can prove correct UID/GID mappings on supported hosts.

Challenges that require it are rejected on runtimes that cannot provide it.

Never silently simulate real ownership while claiming it is kernel-enforced ownership.

---

# 10. PTY and shell integration

## 10.1 PTY library

Use `portable-pty` unless a measured Linux-specific limitation appears.

Even with Linux-only v1, it gives us a clean path for future clients/platform experiments and avoids custom PTY code.

## 10.2 Process chain

Conceptually:

```text
daemon
  -> open PTY pair
  -> spawn Bubblewrap attached to slave
      -> Bash attached to same terminal
  -> daemon reads/writes master
```

PTY output is forwarded to the controlling client as raw bytes.

## 10.3 Bash configuration

Ship a minimal controlled `.bashrc` for the learner environment.

It may provide:

- readable colored prompt
- challenge identifier
- sensible history settings
- optional completion
- terminal metadata emission

It must not redefine core command semantics.

## 10.4 Shell telemetry

Use lightweight shell integration only for UX/adaptation.

Good uses:

- current working directory
- previous command exit code
- command history event
- prompt-ready event

Potential mechanism:

- Bash `PROMPT_COMMAND`
- application-specific OSC escape sequence intercepted by client/daemon

Telemetry is **not verification authority**. The learner can modify shell state; that is fine.

## 10.5 Assisted mode

Do not attempt a fake Fish shell on top of Bash in v1.

Assisted mode may provide around the terminal:

- recently used command explanation
- “what happened?” panel after an error
- related documentation
- hints
- completion education

Do not implement fragile inline ghost-text editing until the project has a proper readline integration or switches to an embeddable shell mode.

## 10.6 Future Brush mode

Keep a future shell enum in domain design only when needed:

```rust
enum ShellKind {
    Bash,
    // later
    Brush,
}
```

Do not create a `ShellBackend` trait until Brush is actually implemented.

---

# 11. Challenge authoring model

## 11.1 Authoring format

Use **KDL v2** for human-authored challenge and curriculum files.

Use the Rust `kdl` crate.

KDL is the authoring syntax, not the application domain model.

## 11.2 Compilation pipeline

```text
source.kdl
  -> KDL parse
  -> SourceChallenge
  -> semantic decoding
  -> ChallengeSpec
  -> schema validation
  -> curriculum validation
  -> runtime-capability validation
  -> fixture validation
  -> CompiledChallenge
```

Only `CompiledChallenge` reaches the runtime.

## 11.3 Strong IDs

Use newtypes:

```rust
pub struct ChallengeId(String);
pub struct SkillId(String);
pub struct ToolId(String);
pub struct TrackId(String);
pub struct DocId(String);
pub struct PackId(String);
pub struct SessionId(String);
```

Validation occurs at construction/parsing.

IDs should use predictable lowercase dotted/kebab namespace conventions, for example:

```text
fs.navigation.pwd
fs.search.find
shell.pipes.basic
process.signals.kill
```

## 11.4 Domain model

Illustrative shape:

```rust
pub struct ChallengeSpec {
    pub id: ChallengeId,
    pub meta: ChallengeMeta,
    pub skills: ChallengeSkills,
    pub objective: Objective,
    pub environment: EnvironmentSpec,
    pub setup: Vec<SetupAction>,
    pub verifier: VerifierExpr,
    pub hints: Vec<Hint>,
    pub solution: CanonicalSolution,
    pub docs: Vec<DocId>,
    pub visual: Option<VisualSpec>,
}
```

## 11.5 Example KDL

```kdl
challenge "fs.find.lost-note" version=1 {
    title "The Lost Note"
    summary "Find a file without manually opening every directory."
    difficulty "beginner"
    estimated-minutes 4

    skills {
        requires "fs.navigation.basic"
        teaches "fs.search.find"
        reinforces "fs.paths.relative"
    }

    state "ephemeral"
    shell "bash"

    capabilities {
        network "none"
        process-observation false
        multi-user false

        tools {
            allow "ls"
            allow "cd"
            allow "pwd"
            allow "cat"
            allow "find"
        }
    }

    objective {
        text "Locate `treasure.txt`, move into its directory, and print its contents."
    }

    setup {
        mkdir "~/archive/2024/notes"
        mkdir "~/archive/2025/drafts"
        write "~/archive/2024/notes/treasure.txt" {
            text "You found it."
        }
        write "~/archive/2025/drafts/todo.txt" {
            text "Nothing useful here."
        }
    }

    verify {
        all {
            file-read "~/archive/2024/notes/treasure.txt"
            cwd "~/archive/2024/notes"
        }
    }

    hint level=1 {
        text "Opening folders one by one works, but Linux has a tool made for locating files."
    }

    hint level=2 {
        doc "command.find"
    }

    solution {
        concept "Use `find` to search a directory tree by name."
        command "find ~/archive -name treasure.txt"
        command "cd ~/archive/2024/notes"
        command "cat treasure.txt"
    }

    visual "filesystem-tree"
}
```

## 11.6 Typed setup actions

AI-safe and community-safe setup must be declarative.

Initial setup enum:

```rust
enum SetupAction {
    CreateDir { path: SandboxPath, mode: Option<FileMode> },
    WriteFile { path: SandboxPath, contents: Vec<u8>, mode: Option<FileMode> },
    CopyFixture { fixture: FixtureId, destination: SandboxPath },
    CreateSymlink { target: SandboxPath, link: SandboxPath },
    SetMode { path: SandboxPath, mode: FileMode },
    SetEnv { key: EnvKey, value: String },
}
```

Later capabilities can add:

```text
spawn_process
create_user
start_local_service
create_socket
mount_fixture
```

Each later action must advertise runtime capabilities required.

### Trusted executable setup

Built-in hand-maintained challenge packs may eventually support:

```text
trusted_exec
```

but it should be clearly marked and unavailable to AI-generated or untrusted packs.

Do not make shell scripts the default setup language.

## 11.7 Verifier expression tree

Verification is compositional:

```rust
enum VerifierExpr {
    All(Vec<VerifierExpr>),
    Any(Vec<VerifierExpr>),
    Not(Box<VerifierExpr>),
    Predicate(VerifierPredicate),
}
```

Initial predicates:

```rust
enum VerifierPredicate {
    PathExists(SandboxPath),
    PathMissing(SandboxPath),
    FileContains { path: SandboxPath, needle: String },
    FileEquals { path: SandboxPath, expected: Vec<u8> },
    FileMode { path: SandboxPath, mode: FileMode },
    SymlinkTarget { path: SandboxPath, target: SandboxPath },
    WorkingDirectory(SandboxPath),
    SubmittedAnswer(AnswerRule),
}
```

Later predicates gated by runtime probe capability:

```text
ProcessExists
ProcessMissing
SocketListening
EnvironmentValue
FileOwner
GroupMembership
MountPresent
```

## 11.8 Verification authority

Prefer backend-observed state.

Examples:

- filesystem result -> inspect writable sandbox state from host-side runtime mapping;
- submitted answer -> backend evaluates answer rule;
- current directory -> telemetry is acceptable for beginner UX, but document that it is not a hostile-user security check;
- process state -> only expose once a reliable runtime probe exists.

Do not verify a `find` lesson using “did command history contain the word `find`?”.

## 11.9 Answer challenges

Some puzzle-style lessons can use an answer/flag.

Do not leave a universal plaintext `flag{...}` file in every challenge.

Use:

```text
state-based verification first
answer submission where conceptually appropriate
literal discoverable flags only when the lesson is specifically about finding them
```

An offline learner can always reverse-engineer local content if determined. The goal is to prevent accidental trivial solutions, not build anti-cheat DRM.

---

# 12. Challenge compiler

The compiler is one of the highest-value correctness components.

## 12.1 Compiler outputs

```rust
pub struct CompiledChallenge {
    pub id: ChallengeId,
    pub required_runtime: RuntimeRequirements,
    pub sandbox_plan: SandboxPlan,
    pub setup_plan: SetupPlan,
    pub verifier: CompiledVerifier,
    pub learning: LearningPlan,
    pub presentation: PresentationPlan,
}
```

## 12.2 Validation passes

Run distinct passes and report all useful errors where possible.

### Parse validation

- valid KDL
- allowed node names
- expected types

### Identity validation

- unique challenge IDs
- unique skill IDs
- valid references

### Graph validation

- prerequisite cycles rejected unless explicitly modeling reinforcement edges separately
- challenge prerequisites must exist
- checkpoint rules valid

### Environment validation

- tool IDs exist
- denied and allowed tools do not conflict
- network policy supported
- state mode supported
- shell supported

### Setup validation

- paths remain inside sandbox namespace
- fixture references exist
- setup does not write readonly system paths unless capability permits it
- symlink definitions cannot escape allowed fixture model

### Verifier validation

- referenced paths are valid
- required runtime probe capabilities exist
- answer rules are well-formed

### Pedagogy linting

Warnings rather than hard errors for:

- challenge teaches skill but exposes no related docs
- challenge has no hint
- canonical solution uses a tool not exposed
- estimated difficulty conflicts with prerequisite depth
- objective mentions a path that setup never creates
- hidden answer appears literally in an exposed source file

## 12.3 Content compiler command

```text
learn content check
```

must compile **every built-in challenge**, validate graph integrity, fixture hashes, docs links, solution tool availability, and verifier support.

No release is permitted with this command failing.

---

# 13. Skill graph

## 13.1 Data model

```rust
pub struct Skill {
    pub id: SkillId,
    pub title: String,
    pub description: String,
    pub prerequisites: Vec<SkillId>,
    pub track: TrackId,
    pub tags: Vec<SkillTag>,
}
```

Challenge relation:

```rust
pub struct ChallengeSkills {
    pub requires: Vec<SkillId>,
    pub teaches: Vec<SkillId>,
    pub reinforces: Vec<SkillId>,
}
```

## 13.2 Initial tracks

Suggested graph families:

```text
Foundation
├── terminal orientation
├── paths/navigation
├── files/directories
├── reading files
└── help/documentation

Shell Composition
├── globbing
├── quoting
├── redirection
├── pipes
├── exit status
└── environment variables

Finding & Text
├── find
├── grep
├── sort/uniq
├── wc
├── cut/paste
├── sed
└── awk

Permissions & Identity
├── mode bits
├── ownership
├── groups
└── root/sudo concepts

Processes
├── ps
├── jobs
├── background/foreground
├── signals
├── kill
└── process tree

Filesystem Model
├── links
├── file types
├── descriptors
├── /dev
├── /proc
└── mounts

Networking
├── addresses
├── ports
├── DNS
├── sockets
├── curl
└── basic troubleshooting

Shell Scripting
├── variables
├── conditionals
├── loops
├── functions
└── robust scripts

Modern CLI Tools
├── rg
├── fd
├── bat
└── other deliberately selected tools

Linux Internals
├── namespaces
├── procfs deeper dive
├── syscalls/strace
├── cgroups
└── service/process supervision

Vim Track (optional)
├── movement
├── insert/normal mode
├── editing
└── terminal workflow
```

## 13.3 Progress model

Do not reduce progress to XP.

Persist at least:

```rust
pub struct ChallengeProgress {
    pub challenge_id: ChallengeId,
    pub status: CompletionStatus,
    pub attempts: u32,
    pub hints_used: Vec<HintId>,
    pub first_started_at: Option<Timestamp>,
    pub completed_at: Option<Timestamp>,
    pub active_seconds: u64,
}
```

Skill mastery can initially be a transparent derived state:

```rust
enum Mastery {
    Locked,
    Introduced,
    Practiced,
    Demonstrated,
}
```

Avoid fake precision such as “grep mastery = 73.42%” until there is evidence such scoring improves learning.

## 13.4 Checkpoints

Every roughly 8–12 normal challenges along a beginner route, offer a “prove it” checkpoint.

Checkpoint characteristics:

- hints hidden initially
- docs may be hidden or optionally available
- combines multiple previous skills
- no new concept introduced
- reset remains safe

Failure does not punish the learner; it identifies which branch to reinforce.

## 13.5 Learner-facing graph

Default view shows only the local neighborhood around the learner.

Offer:

```text
Current Path
Nearby Skills
Full Graph
```

Do not open with a giant tech tree.

---

# 14. Initial curriculum shape

This is a starting content plan, not a hard linear dependency chain.

## Foundation sequence

### 0. “This is a shell”
Learn:

- prompt
- command + arguments
- Enter executes
- `Ctrl-C`
- output vs input

### 1. “Where am I?”

- `pwd`
- absolute path concept

### 2. “What is here?”

- `ls`
- hidden files introduced later, not all flags at once

### 3. “Move around”

- `cd`
- `.`
- `..`
- `~`

### 4. “Make something”

- `mkdir`
- `touch`

### 5. “Read it”

- `cat`
- `less`

### 6. “Move, copy, remove”

- `cp`
- `mv`
- `rm`
- safe sandbox makes destructive behavior tangible

### 7. “Patterns”

- `*`
- `?`
- quoting distinction

### 8. “Output goes somewhere”

- `>`
- `>>`
- stdin/stdout mental model

### 9. “Tools compose”

- `|`
- two-command pipe

### 10. Checkpoint 1

No hints initially. Combine navigation, files, globbing, redirection, pipe.

### 11. “Find the file”

- `find`

### 12. “Find the text”

- `grep`

### 13. “Who may access this?”

- `ls -l`
- mode bits
- `chmod`

### 14. “Processes are running programs”

- `ps`
- PID

### 15. “Ask a process to stop”

- signals
- `kill`
- do not immediately teach `kill -9` as normal behavior

### 16. “Foreground and background”

- `&`
- `jobs`
- `fg`
- `bg`

### 17. “Environment”

- env vars
- `$PATH`

### 18. “Links”

- symlink
- hard link later or same branch depending pace

### 19. “Linux exposes itself as files”

- `/proc`
- process information

### 20. Checkpoint 2

Then branch naturally into text processing, permissions/users, scripting, networking, modern tools, and internals.

---

# 15. Documentation system

## 15.1 User goal

The learner should be able to type imperfect natural-ish fragments such as:

```text
find file by name
change owner file
show process using port
search text inside files
```

and receive useful local results without an LLM.

## 15.2 Sources

The local docs index may combine:

- curated beginner docs written for the application
- command reference cards
- examples
- challenge-specific notes
- licensed man-page content bundled where redistribution is allowed
- locally discovered system man pages where appropriate

Do not blindly redistribute every man page without checking its license.

## 15.3 Search model

Use `frizbee` as the initial fuzzy matcher.

As of 2026-09-22, current `frizbee` is a SIMD typo-resistant fuzzy matcher and is already used by projects including fff, Atuin, Television, Skim, and blink.cmp.

Do not add embeddings to v1.

## 15.4 Document chunk model

```rust
pub struct DocChunk {
    pub id: DocChunkId,
    pub doc_id: DocId,
    pub title: String,
    pub section: Option<String>,
    pub aliases: Vec<String>,
    pub keywords: Vec<String>,
    pub body: String,
    pub commands: Vec<ToolId>,
    pub skills: Vec<SkillId>,
}
```

Precompute normalized search fields at index build time.

## 15.5 Ranking

Start with a transparent ranker:

```text
exact alias/title match
  > prefix title match
  > fuzzy title/alias score
  > fuzzy keyword score
  > body match
  + current challenge skill relevance
```

Do not create an opaque ranking ML model.

Benchmark before adding a more complex index.

## 15.6 Search UX

Right pane supports:

```text
Search
Docs
Man
Examples
Related
```

Example result:

```text
find
  Locate files and directories by conditions such as name or type.

rg
  Recursively search text inside files.

grep
  Search lines matching a pattern.
```

## 15.7 Performance target

For a local index of at least 20,000 chunks on a normal development laptop:

```text
p95 search latency <= 20 ms
```

This is a benchmark target, not a reason to write unsafe SIMD code ourselves.

---

# 16. TUI architecture

## 16.1 Libraries

Baseline:

- `ratatui`
- `crossterm`
- `tachyonfx` for small, state-driven transitions only; keep motion optional and never animate the learner's terminal output
- `tui-term` for terminal screen rendering

The daemon owns the PTY process lifecycle. Do not use unstable `tui-term` controller APIs as the authority over sessions.

## 16.2 Main layout

Large terminal:

```text
┌──────────────────────────────────────────────────────┬──────────────────────┐
│ challenge visual / concept model                     │ Objective            │
│ state-reactive, usually 15–25% of left column       │ progress / hints     │
├──────────────────────────────────────────────────────┤──────────────────────┤
│                                                      │ Docs / search        │
│                                                      │                      │
│                  REAL TERMINAL                       │ fuzzy results        │
│                                                      │ man/reference        │
│                                                      │                      │
│                                                      │                      │
├──────────────────────────────────────────────────────┴──────────────────────┤
│ challenge • skill • runtime status • optional key hints                    │
└─────────────────────────────────────────────────────────────────────────────┘
```

Approximate width split:

```text
left  75–82%
right 18–25%
```

The terminal remains dominant.

## 16.3 Responsive layouts

At narrower widths:

```text
terminal full width
right pane becomes overlay/drawer
visual panel collapses first
```

At very small terminals:

```text
terminal
+ status line
+ command palette to open objective/docs/progress/Vim actions
```

Never render an unusable three-column layout because the design was tuned for a screenshot.

## 16.4 Input philosophy

Keyboard-first, mouse optional.

Mouse support:

- tabs
- docs result selection
- scrolling
- buttons where useful

Mouse can be disabled.

Normal shell keybindings must continue to work.

Do not steal common Bash/readline bindings such as `Ctrl-P`, `Ctrl-N`, `Ctrl-R`, `Ctrl-A`, `Ctrl-E`, or `Ctrl-W` as global app shortcuts.

The primary navigation is modal rather than a collection of hidden
function-key commands. Function keys are intentionally not part of the
product workflow. `Ctrl-Space` opens a searchable command menu; typing filters
actions, arrow keys select, Enter executes, and Esc returns to the lesson.
`Ctrl-H` focuses the terminal, `Ctrl-L` focuses the docs search pane, and the
palette exposes docs, challenges, progress, Vim dojo, motion, themes, reset,
and quit.

A command palette can be accessible through the menu or a carefully selected chord after testing terminal compatibility.

## 16.5 Optional Vim navigation

Vim controls are opt-in.

The application can contain a Vim learning track and a UI navigation setting, but a beginner must not need `hjkl` to operate the basic app.

The first implementation slice is the command-palette Vim dojo. It is a
self-contained practice surface rather than a hidden replacement for the Linux shell:

- Snake uses `h`, `j`, `k`, and `l` for movement.
- Repeat & repair teaches `i`, `Esc`, `a`, `A`, `0`, `$`, `x`, and `dd` against a short target sentence.
- Ex substitute teaches `:%s/old/new/g`-style editing with the concrete exercise `%s/hate/love/g`.
- Exercise text may be playful and sarcastic, but must remain readable, non-abusive, and useful.
- Every activity has a reset and a clear route back to the dojo menu or Linux lesson.

## 16.6 TUI state model

Prefer reducer-like state transitions. The first client uses an explicit `AppMode` state (`Lesson`, `Palette`, `Docs`, `Challenges`, `Progress`, and `Vim`) so focus ownership is unambiguous. In `Lesson`, unhandled keys are forwarded to the real shell; in every other mode, the modal owns input until Esc closes it or an action completes.

```rust
enum UiMode {
    Terminal,
    DocsSearch,
    DocsView,
    Challenge,
    Progress,
    Help,
}

struct AppState {
    mode: UiMode,
    session: SessionView,
    terminal: TerminalView,
    docs: DocsView,
    challenge: ChallengeView,
    progress: ProgressView,
    settings: UiSettings,
}
```

Input processing should produce domain UI events and side effects rather than directly mutating unrelated widgets everywhere.

## 16.7 Rendering

Rendering should be a function of state as much as practical.

No UI component should directly call daemon persistence or Bubblewrap.

## 16.8 Animation architecture

No general game engine.

Use a small set of state-driven visual primitives:

```rust
enum VisualSpec {
    FileTree(FileTreeVisual),
    Pipeline(PipelineVisual),
    ProcessTree(ProcessTreeVisual),
    Permissions(PermissionsVisual),
    Network(NetworkVisual),
}
```

Animations update only while something is changing.

Idle UI should not repaint at 60 FPS.

Suggested animation cadence when active:

```text
10–20 FPS
```

Use event-driven redraw otherwise.

Reduced-motion mode turns transitions into immediate state changes.

## 16.9 Visual examples

Pipe:

```text
cat notes.txt  ── data ──▶  grep rust  ── matches ──▶  stdout
```

Permissions:

```text
owner  [r][w][-]
group  [r][-][-]
other  [-][-][-]
```

Processes:

```text
bash(1)
├─ sleep(24)
└─ grep(31)
```

The visual should update because actual lesson state changes, not because a timer plays a random animation.

---

# 17. Persistence

## 17.1 Database

Use `redb` initially.

Reasons:

- pure Rust
- embedded
- ACID
- crash-safe
- simple B-tree model
- one daemon is the single writer

Do not introduce SQL unless future product requirements actually benefit from relational querying.

## 17.2 Storage location

Linux:

```text
$XDG_STATE_HOME/learn/
├── state.redb
├── logs/
└── homes/
```

Cache:

```text
$XDG_CACHE_HOME/learn/
├── runtime/
├── docs/
└── compiled-content/
```

Config:

```text
$XDG_CONFIG_HOME/learn/config.kdl
```

## 17.3 Stored tables

Conceptual tables:

```text
profiles
challenge_progress
skill_progress
settings
content_versions
```

Do not persist full terminal output indefinitely by default.

Command history used for learning adaptation remains local and should have retention limits/settings.

## 17.4 Migrations

Persist an explicit schema version.

Migration functions are sequential and tested using fixture databases from previous versions.

Never make startup “best effort parse whatever is there”.

---

# 18. AI mode — later architecture only

AI is explicitly not Phase 1.

## 18.1 Role

The first AI feature is **challenge generation**, not an all-knowing chat assistant controlling the runtime.

Later AI may receive:

- current skill graph neighborhood
- completed challenges
- attempts
- hints used
- recent command patterns

and generate the next challenge within a requested skill/difficulty envelope.

## 18.2 Provider model

Architect for OpenAI-compatible HTTP APIs.

Ollama can be supported by configuring an OpenAI-compatible local endpoint where available.

Do not build a provider abstraction until the AI phase begins.

## 18.3 AI output

Do not ask the model to output KDL directly as the canonical machine contract.

Preferred flow:

```text
LLM
 -> strict structured JSON
 -> GeneratedChallengeSpec
 -> type validation
 -> semantic validation
 -> capability validation
 -> deterministic compiler
 -> runnable challenge
```

Human-authored KDL and AI JSON both compile into the same internal `ChallengeSpec`.

## 18.4 AI safety boundary

AI-generated challenge content cannot:

- execute arbitrary shell setup scripts
- mount host paths
- enable network unless policy permits
- invent unknown binaries
- select unrestricted host capabilities
- run host commands

It may only compose typed setup/verifier primitives that the runtime understands.

If generation requests an unsupported capability, reject it and optionally ask the model to regenerate.

## 18.5 AI verification before user sees challenge

Generated challenge must pass:

1. schema validation;
2. semantic compiler;
3. runtime capability compiler;
4. deterministic setup dry-run;
5. verifier satisfiability smoke test where possible;
6. canonical-solution simulation if supplied;
7. timeout/resource limits.

AI generation is not trusted because the model says “this should work”.

---

# 19. Cross-platform strategy

## 19.1 v1

Linux only for full runtime.

The code should compile cleanly with OS boundaries isolated, but do not delay the Linux product trying to implement fake Linux everywhere.

## 19.2 Future Windows

Preferred full-fidelity route:

```text
Windows client
 -> local daemon adapter
 -> WSL runtime
```

A later Brush-based portable mode may support a subset of introductory shell lessons without WSL.

## 19.3 Future macOS

Possible routes:

- lightweight Linux VM backend for full fidelity
- Brush/native compatibility backend for restricted introductory lessons

## 19.4 Future web/WASM

WASM can support a subset:

```text
navigation model
files
simple shell composition
basic scripting
```

Do not claim full Linux semantics for:

- `/proc`
- PID namespaces
- real Unix ownership
- mounts
- cgroups
- Linux signals/process behavior
- system services

## 19.5 Avoid speculative trait architecture

Do not create five runtime backend implementations that do not exist.

Keep Linux runtime concrete.

When a second backend is actually implemented, extract the minimum shared interface from the real call sites.

---

# 20. Dependency baseline

Reference versions verified around 2026-09-22. The implementation agent must re-check compatibility before editing the lockfile, but should not churn versions without a reason.

| Purpose | Crate/tool | Verified reference |
|---|---|---:|
| async runtime | `tokio` | 1.53.1 |
| protobuf | `prost` | 0.14.4 |
| framed transport | `tokio-util` | 0.7.19 |
| byte buffers | `bytes` | 1.12.1 |
| TUI | `ratatui` | 0.30.2 |
| terminal backend | `crossterm` | 0.29.0 |
| optional TUI effects | `tachyonfx` | 0.25.2 |
| terminal widget | `tui-term` | 0.3.4 |
| PTY | `portable-pty` | 0.9.x |
| challenge/config parser | `kdl` | 6.7.1 |
| fuzzy matching | `frizbee` | 0.13.0 |
| embedded DB | `redb` | 4.3.0 |
| CLI | `clap` | 4.6.7 |
| tracing | `tracing` | 0.1.44 |
| hash | `blake3` | 1.8.7 |
| property tests | `proptest` | 1.11.0 |
| Linux sandbox | Bubblewrap | >= 0.12.0 |
| bootstrap rootfs | Alpine | 3.24.2 baseline |
| optional future shell | `brush-core` | 0.5.0 |

Additional likely small dependencies:

- `thiserror`
- `serde`
- `serde_json` for persistence/AI boundaries where appropriate
- `tracing-subscriber`

Do not add a dependency purely because an implementation agent prefers its API.

---

# 21. Configuration

Use KDL for user configuration to keep one human-editable structured language.

Example:

```kdl
ui {
    mouse true
    animations true
    reduced-motion false
    vim-navigation false
}

terminal {
    shell "bash"
}

docs {
    online-search false
}

telemetry {
    command-history true
    retention-days 30
}
```

Config precedence:

```text
compiled defaults
 < user config
 < command-line override
```

Environment variables only for deployment/runtime needs, not every setting.

---

# 22. Observability

Use `tracing`.

Logs must never corrupt the TUI screen.

Default log destination:

```text
$XDG_STATE_HOME/learn/logs/daemon.log
```

Useful spans:

```text
session.create
challenge.compile
sandbox.prepare
sandbox.spawn
docs.search
verifier.run
progress.commit
ipc.request
```

Do not log raw terminal input by default because it can contain secrets typed by the learner.

A debug option may record sanitized command events with explicit user consent.

---

# 23. Security model

## 23.1 Threat model

Primary goals:

- learner commands cannot casually access host home/secrets;
- learner commands cannot access network unless challenge permits it;
- challenge content has explicit capabilities;
- official typed setup cannot mutate host filesystem;
- runtime state is resettable;
- community/AI content is not automatically trusted.

Not a goal:

- prevent the machine owner from reverse-engineering local challenge files;
- protect secrets from root on the host;
- make local challenges safe against unknown kernel zero-days.

## 23.2 Host paths

Default sandbox sees none of the host filesystem except deliberately mounted immutable runtime content and challenge-owned state.

Never mount:

```text
$HOME
~/.ssh
~/.config
/run/user/$UID
/var/run/docker.sock
podman socket
Wayland socket
X11 socket
host /proc
```

## 23.3 Network

None by default.

## 23.4 Content trust levels

```rust
enum ContentTrust {
    BuiltIn,
    SignedOfficial,
    TrustedLocal,
    Community,
    Generated,
}
```

Trust affects allowed setup capabilities.

For initial releases, only `BuiltIn` needs execution support.

## 23.5 Community packs later

Before arbitrary community packs are enabled, implement:

- package manifest
- content hash
- signature/trust mechanism
- capability display before install
- resource guard requirement
- no arbitrary host-side executable hooks

---

# 24. Performance budgets

These are engineering targets to benchmark, not excuses for premature unsafe code.

## 24.1 Interactive targets

On a normal modern Linux laptop:

```text
TUI first render after daemon available       <= 150 ms target
PTY input forwarding p95                      <=   5 ms
PTY output to next render p95                 <=  16 ms
docs search over 20k chunks p95               <=  20 ms
simple challenge preparation warm-cache       <= 200 ms
simple challenge reset                        <= 500 ms
idle CPU daemon                               < 0.5% target
idle CPU TUI                                  < 0.5% target
```

## 24.2 Memory targets

Initial soft budget:

```text
daemon idle RSS       <= 35 MiB
TUI idle RSS          <= 40 MiB
```

Measure before optimizing.

## 24.3 Distribution size

Do not force the complete product under 10 MiB if that damages Linux fidelity.

Track separately:

```text
stripped executable size
compressed base runtime
compressed optional tool packs
documentation pack
```

Prefer tens of MiB total over hundreds of MiB, but correctness wins over an arbitrary screenshot-friendly size target.

## 24.4 Release profile

Evaluate:

```toml
[profile.release]
lto = "thin"
codegen-units = 1
strip = "symbols"
panic = "abort"
```

Benchmark startup and binary size before making these permanent.

---

# 25. Testing strategy

The application must be unusually testable because content and runtime behavior are core product logic.

## 25.1 Unit tests

Required for:

- ID parsing
- KDL semantic decoding
- challenge validators
- graph eligibility
- mastery derivation
- verifier expression evaluation
- docs ranking
- protocol conversions
- TUI reducer transitions

## 25.2 Property tests

Use `proptest` for areas where broad generated input matters:

- path normalization cannot escape sandbox path model
- challenge ID parser
- graph ordering invariants
- protocol envelope roundtrip
- verifier boolean expression semantics
- docs rank ordering stability under normalization

## 25.3 Golden tests

Golden files for:

- KDL diagnostics
- compiled challenge summary
- selected TUI small-layout snapshots where stable enough
- docs query ranking for curated queries

Do not snapshot huge ANSI screens blindly.

## 25.4 Content tests

Every built-in challenge must compile in tests.

Each challenge must have at least one machine-readable expected canonical completion path where practical.

Example scenario file:

```text
challenge: fs.find.lost-note
commands:
  - "find ~/archive -name treasure.txt"
  - "cd ~/archive/2024/notes"
  - "cat treasure.txt"
expect: completed
```

A runner executes the commands through the actual sandbox/PTY or through a noninteractive equivalent appropriate to the challenge.

## 25.5 Sandbox integration tests

Linux-only integration tests must prove:

- host home is not visible;
- readonly rootfs cannot be modified;
- writable challenge path can be modified;
- `/proc` shows isolated PID namespace behavior;
- network is unavailable in `NetworkPolicy::None`;
- allowed tool executes;
- denied external tool path is not mounted;
- reset restores fixture;
- session death cleans processes;
- daemon crash cleanup strategy works as designed.

## 25.6 Security regression tests

Include specific tests for:

- absolute symlink fixture escape attempts
- `../` path escape attempts
- symlink setup actions targeting outside sandbox model
- environment leakage
- direct `/usr/bin/<denied-tool>` access
- network access when disabled

## 25.7 Protocol tests

- version mismatch
- malformed frame
- oversized frame
- unknown event field
- interrupted response
- terminal byte payload with arbitrary non-UTF8 bytes

## 25.8 Fuzzing

Fuzz targets:

```text
challenge KDL parser/semantic decoder
protocol frame decoder
path parser
optional docs query parser
```

Fuzzing can run nightly/periodically rather than every local test.

## 25.9 TUI tests

Do not attempt to unit-test Ratatui by checking every character.

Test:

- reducer/state transition logic
- responsive layout breakpoints
- focus behavior
- key routing
- terminal gets unclaimed keys unchanged
- animations disabled means no timer-driven redraw loop

## 25.10 Performance benchmarks

Bench:

- challenge parse + compile for entire content set
- docs index build
- docs queries
- skill graph eligibility selection
- protocol encode/decode
- terminal output ingestion/render preparation

Store representative benchmark history in CI artifacts if practical.

---

# 26. AI-verifiable implementation contract

This section is specifically for implementation agents.

## 26.1 Definition of “done”

A task is not done because code exists.

A task is done only when:

1. implementation is present;
2. tests covering intended behavior are present;
3. static checks pass;
4. relevant integration test passes;
5. user-visible behavior can be demonstrated through a deterministic command;
6. no unrelated architecture shortcuts were introduced.

## 26.2 Mandatory commands before claiming a milestone complete

At minimum:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

When content exists:

```bash
cargo run -- content check
```

When runtime exists:

```bash
cargo run -- doctor --json
cargo run -- runtime verify
```

When challenge execution exists:

```bash
cargo run -- content simulate --all
```

When Nix packaging exists:

```bash
nix flake check
nix build
```

Security/dependency CI:

```bash
cargo audit
cargo deny check
```

These may require CI tools installed separately; they are development tools, not runtime dependencies.

## 26.3 Agent evidence format

At the end of each milestone, implementation agent should report:

```text
Implemented:
- ...

Tests added:
- ...

Verification run:
- cargo fmt ... PASS
- cargo clippy ... PASS
- cargo test ... 143 passed
- learn content check ... 28 challenges valid
- relevant e2e ... PASS

Known limitations:
- ...

Architecture deviations:
- none
```

No “should work” claims.

## 26.4 No placeholder completion

A milestone fails review if it contains:

- `todo!()` in production path
- `unimplemented!()` in required path
- placeholder verifier returning true
- fake sandbox implementation presented as complete
- ignored failing tests without documented environment reason
- UI buttons that do nothing
- `TODO` replacing a stated acceptance criterion

## 26.5 Architecture deviation rule

If implementation evidence suggests this plan is wrong, the agent may change it **only** by recording:

```text
Decision
Evidence
Alternatives considered
Chosen change
Compatibility impact
Tests proving new behavior
```

Do not silently refactor core architecture while implementing an unrelated feature.

---

# 27. CI plan

## 27.1 Fast PR lane

Every PR:

```text
fmt
clippy
unit tests
property tests
content compile
protocol golden tests
Linux build
```

## 27.2 Linux integration lane

Run:

- Bubblewrap smoke test
- sandbox integration tests
- sample curriculum simulation
- network isolation test
- rootfs manifest verification

If generic hosted CI prevents user namespaces, do not replace the test with a fake pass. Add a dedicated Linux/NixOS VM job or documented environment that actually runs it.

## 27.3 Nix

Because NixOS is a primary development environment, provide:

```text
nix develop
nix build
nix flake check
```

The dev shell should include:

- stable Rust toolchain
- rustfmt
- clippy
- protobuf compiler
- Bubblewrap >= required version
- rootfs build prerequisites

Do not make Nix the only supported developer environment.

## 27.4 Release lane

Before release:

- clean rootfs rebuild
- full content simulation
- dependency audit
- binary size report
- startup benchmark
- docs search benchmark
- x86_64 Linux package
- aarch64 Linux package when supported

---

# 28. Development phases

Do not build the beautiful TUI first. Prove the runtime and challenge model first.

---

## Phase 0 — Repository spine and engineering gates

### Deliverables

- Cargo workspace
- crate boundaries from this plan
- `#![forbid(unsafe_code)]` in first-party crates
- Rust toolchain pin
- base lint configuration
- Nix dev shell
- CI fast lane
- `learn version`
- typed error conventions

### Acceptance

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
nix flake check
```

### Explicit non-goals

No TUI design work beyond a compileable skeleton.

---

## Phase 1 — Challenge domain + KDL compiler

### Deliverables

- IDs/newtypes
- challenge domain model
- KDL v2 parser
- setup action model
- verifier expression model
- skill graph model
- validators
- diagnostics
- `learn content check`
- first 5 fixture challenges that compile but do not yet run

### Tests

- malformed KDL diagnostics
- duplicate IDs
- missing skill references
- cyclic prerequisites
- missing fixture
- unsupported verifier
- path escape attempts
- solution uses hidden tool warning

### Acceptance

```bash
cargo test -p learn-core
cargo test -p learn-challenge
cargo run -- content check
```

Expected output should report exact content counts and zero errors.

---

## Phase 2 — Linux runtime proof

### Deliverables

- `learn doctor`
- Bubblewrap discovery/version check
- rootfs installer/builder
- rootfs integrity verification
- PTY creation
- spawn real Bash in sandbox
- raw interactive CLI attach mode before TUI
- namespace baseline
- network-none baseline
- deterministic cleanup

### Required proof

From an actual session:

```text
$ echo $SHELL
/bin/bash

$ ps
# isolated view

$ ls ~
# challenge home, not host home
```

Host secret fixture must be unreachable.

### Acceptance

```bash
cargo run -- doctor --json
cargo run -- runtime verify
cargo test -p learn-runtime --test sandbox
```

The test must run actual Bubblewrap, not mock it.

---

## Phase 3 — Executable challenges + verifier

### Deliverables

- compile `ChallengeSpec` to `SandboxPlan`
- setup action execution
- tool visibility plan
- writable workspace
- filesystem verifier predicates
- answer verifier
- reset
- challenge completion event
- canonical solution display data

### Content

Implement first 10 foundation challenges.

### Acceptance

```bash
cargo run -- content check
cargo run -- content simulate --all
cargo test -p learn-runtime --test tool_visibility
cargo test -p learn-challenge --test canonical_solutions
```

Every built-in canonical solution must complete its challenge.

---

## Phase 4 — Daemon + Protobuf protocol

### Deliverables

- Unix socket daemon
- auto-start
- version handshake
- request/response routing
- event stream
- terminal input/output stream
- terminal resize
- one controlling client
- cleanup on disconnect

### CLI proof

```bash
learn status
learn tui --plain-terminal-test
```

A temporary simple terminal client is acceptable here; full TUI comes next.

### Acceptance

- malformed-client tests
- daemon restart test
- protocol roundtrip
- real session through RPC

---

## Phase 5 — Full-screen TUI baseline

### Deliverables

- Ratatui application
- dominant terminal area
- right objective/docs pane
- responsive layout
- mouse toggle
- animations setting even if visuals are not yet elaborate
- terminal rendering from daemon PTY events
- F-key navigation
- challenge completion screen
- canonical solution/review screen

### UX rule

The user should be able to complete the first ten lessons without touching configuration.

### Acceptance

Manual/scripted smoke:

```text
launch
start challenge
run commands
open docs
return to terminal
resize terminal
complete challenge
view solution
move to next challenge
```

All must work in one session.

---

## Phase 6 — Docs search

### Deliverables

- curated docs schema
- docs index build
- Frizbee search
- context ranking
- right-pane search UI
- command reference/man ingestion path
- benchmark

### Acceptance queries

These should return an appropriate command in top results despite imperfect wording:

```text
find file by name
serch text in fiel
change permision
stop proces
where am i
copy folder
```

Benchmark p95 target <= 20 ms for representative index.

---

## Phase 7 — Progress graph + checkpoints

### Deliverables

- redb persistence
- profile
- challenge progress
- mastery derivation
- eligible-next challenge selection
- graph UI
- local-neighborhood default view
- first checkpoint
- optional placement test

### Acceptance

- restart daemon/app; progress persists
- reset challenge does not erase unrelated progress
- graph unlocks deterministically
- checkpoint completion updates required skills

---

## Phase 8 — State-driven visuals

### Deliverables

Implement only the first useful visual primitives:

1. filesystem tree
2. pipeline
3. permissions
4. process tree

Visuals must subscribe to challenge/session state.

### Acceptance

- global animation off
- reduced motion
- small terminal collapses visual pane
- idle CPU remains within budget

---

## Phase 9 — Process lessons and stronger runtime probes

### Deliverables

- reliable process-state observation
- process fixtures
- signals challenges
- job-control challenges
- resource guard integration

Do not implement process verifiers by parsing `ps` output from learner-controlled shell.

Use runtime-owned observation.

### Acceptance

- runaway challenge process cleaned on reset
- intended signal challenge passes
- unrelated host process is never visible inside challenge PID view
- process count/resource limit verified where guard available

---

## Phase 10 — Permissions/users deeper path

### Deliverables

- ownership lessons supported by actual runtime semantics the host can provide
- explicit capability check for multi-UID behavior
- root/sudo teaching design

If the host cannot provide the required semantics, challenge is marked unavailable with a precise reason.

No fake silent ownership model.

---

## Phase 11 — Networking track

### Deliverables

- deterministic local networking fixtures
- loopback/local service challenges
- port/socket visuals
- DNS concept exercises where deterministic
- internet remains optional

Do not make core progression depend on a public website staying online.

---

## Phase 12 — Community pack format

Only after resource controls and content validation are mature.

### Deliverables

- pack manifest
- hash
- trust level
- capability summary
- signature verification
- installation cache
- pack uninstall

No arbitrary host hooks.

---

## Phase 13 — AI challenge generation

Only after typed challenge model has broad coverage.

### Deliverables

- OpenAI-compatible endpoint configuration
- strict structured output schema
- generated challenge validator
- generated setup limited to typed actions
- canonical solution simulation
- failure/regeneration loop
- opt-in data disclosure screen

AI should not be necessary to use the product.

---

# 29. Rootfs and content build pipeline

Use `cargo xtask` for repository build operations that are too complex for shell one-liners.

Suggested:

```text
cargo xtask rootfs fetch
cargo xtask rootfs build
cargo xtask rootfs verify
cargo xtask content compile
cargo xtask docs build
cargo xtask release verify
```

`xtask` may invoke external system tools, but must validate their versions and outputs.

Do not hide important release steps in undocumented scripts.

---

# 30. Runtime doctor

`learn doctor` is critical because Linux distributions differ.

Human output example:

```text
Learn runtime doctor

[OK] Linux kernel: 6.x
[OK] Bubblewrap: 0.12.0
[OK] unprivileged user namespace
[OK] mount namespace
[OK] PID namespace
[OK] network namespace
[OK] new-session support
[OK] runtime rootfs installed
[OK] rootfs checksum
[OK] PTY
[OK] docs index
[WARN] cgroup resource guard unavailable
       Official local lessons can run.
       Untrusted community challenges are disabled.
```

Machine-readable:

```bash
learn doctor --json
```

CI should consume this.

---

# 31. Content quality rules

Every challenge should answer:

1. What concept is being taught?
2. Why would a Linux user care?
3. What state must exist before the challenge?
4. What state proves success?
5. Can a learner solve it creatively?
6. Is the canonical solution actually available in the challenge tool set?
7. What is the first hint?
8. What is the second hint/document reference?
9. What misconception is likely?
10. Does reset always restore the intended starting state?

Avoid challenge writing like:

> Run `grep foo bar.txt`.

Prefer:

> A large log contains the line that explains why the service failed. Find the relevant lines without opening the entire file manually.

The objective creates a reason to use the concept without scripting every keystroke.

---

# 32. “Intended solution” behavior

Completion screen:

```text
✓ Solved

You reached the correct state.

Concept
Use `find` to search a directory tree using conditions.

Canonical approach
$ find ~/archive -name treasure.txt

Why it works
- `~/archive` is the starting tree
- `-name` filters entries by filename

Your approach
<locally summarized command history if enabled>

Also useful later
fd treasure ~/archive
```

Do not shame alternate solutions.

If learner uses an advanced tool before it is taught, acknowledge it and still explain the current concept.

---

# 33. Command restriction design detail

There are three levels of restriction. Name them explicitly so content authors understand guarantees.

```rust
enum ToolRestrictionStrength {
    Guidance,
    ExternalExecutableVisibility,
    // future only if a controlled shell backend can truly provide it
    RestrictedShell,
}
```

`Guidance`
: UI says which tools are expected, no runtime hiding.

`ExternalExecutableVisibility`
: only approved external executables are mounted into executable paths.

`RestrictedShell`
: not implemented in Bash v1.

Most beginner lessons use `ExternalExecutableVisibility` where a restriction matters.

Never label it “only possible solution”.

---

# 34. Architecture for tool visibility

Build a tool manifest at rootfs construction time.

Conceptual record:

```rust
pub struct ToolManifestEntry {
    pub id: ToolId,
    pub canonical_path: SandboxPath,
    pub aliases: Vec<SandboxPath>,
    pub runtime_files: Vec<RuntimeFile>,
}
```

`runtime_files` includes executable dependencies that must remain available, such as loader/library files or helper data discovered by the rootfs build pipeline.

At challenge compile time:

```text
allowed ToolIds
 -> resolve manifest entries
 -> compute executable visibility set
 -> produce mount plan
```

At runtime:

```text
readonly rootfs libs/data
empty executable directories
bind only approved executable entries
```

The exact mount strategy must be integration-tested on the selected rootfs; do not finalize it from assumptions alone.

---

# 35. TUI terminal correctness

The terminal renderer must correctly handle common full-screen/interactive terminal behavior eventually:

- ANSI colors
- cursor movement
- erase sequences
- alternate screen
- resize
- UTF-8 output
- arbitrary byte chunks split at any boundary

`tui-term`/`vt100` handles screen parsing. Do not implement ANSI parsing manually.

Important architectural point:

```text
daemon transports bytes
TUI interprets terminal screen
```

The daemon should not convert terminal output into lines because that breaks interactive programs.

---

# 36. Filesystem path types

Do not pass arbitrary host `PathBuf` values throughout challenge domain code.

Create distinct types:

```rust
SandboxPath
HostRuntimePath
FixturePath
```

`SandboxPath` parser guarantees:

- absolute or well-defined home-relative semantics
- normalized components
- no NUL
- no host escape concept

Compilation resolves challenge paths before runtime operations.

Property-test the parser aggressively.

---

# 37. Time and determinism

Challenges should be deterministic by default.

If randomization improves replayability, require an explicit seed:

```rust
pub struct ChallengeSeed(u64);
```

Persist seed with attempt state.

The same challenge + seed + content version must produce the same starting world.

Do not use wall-clock time as hidden challenge setup state unless the lesson explicitly teaches time.

---

# 38. Versioning

Version separately:

```text
application version
wire protocol version
content schema version
content pack version
runtime/rootfs version
persistence schema version
```

Do not overload application SemVer to mean all of these.

A bug fix to the TUI should not require a content schema migration.

---

# 39. Accessibility and beginner friendliness

Required:

- no color-only status meaning
- reduced-motion mode
- animations off switch
- mouse off switch
- readable default contrast
- normal arrow/tab/enter navigation
- error messages explaining what failed
- no required Vim knowledge
- docs searchable without knowing exact command names

When a command fails, the UI may explain the class of error after the shell returns it, but should not replace the real error text.

Example:

```text
bash: cd: logs: No such file or directory

Side explanation:
`cd` can only enter an existing directory. Try `ls` to inspect the current directory.
```

---

# 40. Dependency and architecture review checklist

Before adding a crate, implementation agent must answer in PR notes:

```text
What problem does it solve?
How much code/risk does it replace?
Is it runtime or dev-only?
Is the project actively maintained?
Does it pull a large dependency tree?
Could std/current dependencies solve this clearly?
```

Before adding a trait:

```text
What are the two real implementations today?
OR
What hard external boundary requires dynamic dispatch?
```

If neither exists, prefer a concrete type or enum.

---

# 41. Performance methodology

Do not optimize from intuition.

Workflow:

```text
measure
 -> identify dominant cost
 -> isolate benchmark
 -> optimize
 -> prove improvement
 -> check readability/regression
```

Examples:

- If docs search is already 4 ms, do not write a custom unsafe matcher.
- If rootfs startup dominates, optimize mount/session preparation rather than micro-optimizing Protobuf encoding.
- If TUI idle CPU is high, inspect redraw/tick strategy before changing data structures.

---

# 42. Failure handling

## Daemon crash

On next start:

- detect stale socket
- detect orphan session metadata
- clean safe known temporary paths
- do not delete unknown user files

## Runtime missing

Show:

```text
Linux sandbox runtime is not installed.
Run: learn runtime install
```

## Bubblewrap too old

Show exact detected and required version.

## User namespaces disabled

Explain what host feature is missing and point to `learn doctor` output.

Do not silently fall back to running Bash directly on the host.

That fallback would violate the product safety model.

---

# 43. Installation behavior

Recommended release behavior:

- application package remains relatively small;
- first launch or package post-install fetches/installs the pinned official runtime rootfs;
- distro packages may ship rootfs separately;
- a self-contained offline release bundle may include both.

Support both:

```text
network installer
offline bundle
```

All downloaded runtime artifacts require checksum verification before unpacking.

---

# 44. Modern-tools philosophy

Teach canonical Linux tools first when they establish transferable concepts.

Then explicitly show modern alternatives:

```text
grep   -> rg
find   -> fd
cat    -> bat (when presentation helps)
cd     -> zoxide as workflow enhancement, not replacement of path knowledge
```

The learner should understand the base system before shortcuts hide it.

---

# 45. Destructive learning

The sandbox should eventually allow genuine consequences:

```text
rm
chmod 000
kill
broken PATH
bad symlink
misconfigured shell script
```

Use these after basic fundamentals, not in the first minutes.

The lesson should teach recovery as much as destruction.

Example advanced challenge:

```text
“You broke your PATH. `ls` now says command not found. Recover without resetting.”
```

This is one of the product's strongest advantages over a static course.

---

# 46. Initial UI visual language

Use a modern minimal visual system:

- one accent color + semantic colors
- subtle borders
- generous whitespace where terminal geometry permits
- clear typography hierarchy using terminal attributes, not ASCII-art banners
- small motion with purpose
- no “hacker green” default
- no fake CRT scanlines
- no matrix rain
- no cyberpunk framing

The application should look at home next to modern Linux TUIs and compositors, not like a movie prop terminal.

---

# 47. Open technical questions to resolve by prototype, not debate

These are intentionally deferred to measured prototypes.

## 47.1 Exact executable visibility mount strategy

Prototype nested Bubblewrap mounts against the chosen Alpine+GNU rootfs and prove:

- approved binaries work;
- hidden absolute paths do not;
- dynamic libraries still resolve;
- startup remains fast.

Choose the simplest proven strategy.

## 47.2 Resource guard implementation

Prototype:

1. systemd user scope;
2. direct delegated cgroup v2 where available.

Pick primary + fallback based on actual distro behavior.

## 47.3 Process verifier control plane

Do not invent a complex supervisor until process lessons require it.

Prototype options:

- daemon-observed host descendant PIDs;
- minimal sandbox init/helper with private control channel;
- cgroup membership observation.

Select based on correctness and cleanup behavior.

## 47.4 Documentation index structure

Start with in-memory chunk vector + Frizbee ranking.

Only add a specialized search index if benchmark misses target.

---

# 48. Explicit anti-patterns

Reject these during implementation review:

### “Everything implements a trait”

No.

### “One giant AppState in `Arc<Mutex<_>>`”

No.

### “Use Docker because sandboxing is hard”

Not for the baseline local runtime.

### “Parse terminal output line-by-line”

No. PTY data is a terminal byte stream.

### “Verifier checks the command string”

Only for rare pedagogy-specific cases, never as default.

### “AI can write setup.sh and we execute it because Bubblewrap exists”

No.

### “Run without sandbox when Bubblewrap fails”

No.

### “Bundle every GNU tool and pretend challenge deny list in UI enforces it”

No.

### “Make the learner use Vim keys because TUI users know Vim”

No.

### “Add embeddings/vector DB before measuring fuzzy search”

No.

### “Optimize with unsafe before a benchmark exists”

No.

### “Create cross-platform abstraction layers with only one backend”

No.

---

# 49. Minimum viable release definition

The first public-quality release should include:

- Linux x86_64 support
- NixOS tested
- Bash sandbox through Bubblewrap
- reproducible runtime rootfs
- full-screen Ratatui client
- daemon/client protocol
- 25–40 polished beginner challenges
- skill graph
- at least 2 checkpoints
- local docs/search
- challenge reset
- persistent progress
- animations toggle
- mouse toggle
- `learn doctor`
- deterministic content validator
- full canonical-solution simulation in CI

It should **not** ship AI just to have an AI checkbox.

If the non-AI learning loop is not good, AI will not fix the product.

---

# 50. Suggested first release challenge count

Quality target:

```text
Foundation              10
Finding/Text              6
Permissions               5
Processes                 6
Filesystem model          5
Checkpoints                3
----------------------------
Total                     35
```

This is enough to feel like a real learning product without creating a content factory before the engine is proven.

---

# 51. Final implementation order summary

```text
0. repo + quality gates
1. typed challenge domain
2. KDL compiler
3. real Bubblewrap+Bash runtime
4. deterministic setup/reset/verifier
5. daemon + Protobuf IPC
6. TUI terminal experience
7. docs fuzzy search
8. persistence + skill graph
9. educational visuals
10. process/resource-control depth
11. permissions/users depth
12. networking
13. community packs
14. AI-generated challenges
15. extra runtime/frontends
```

Never reverse this into:

```text
pretty TUI
 -> AI
 -> lots of content
 -> figure out runtime semantics later
```

The runtime/challenge model is the product foundation.

---

# 52. Final architecture invariants

These invariants should still be true a year into development:

1. **A challenge is data, not application code.**
2. **A challenge compiles before it runs.**
3. **Unsupported capabilities fail before session start.**
4. **The canonical Linux path uses real Linux behavior.**
5. **Frontend does not own sandbox policy.**
6. **Daemon does not depend on TUI details.**
7. **Verification prefers resulting state over intended keystrokes.**
8. **Creative valid solutions pass.**
9. **AI never bypasses typed challenge validation.**
10. **Offline usage remains first-class.**
11. **First-party unsafe remains forbidden unless evidence forces a narrowly audited exception.**
12. **Performance work begins with measurement.**
13. **No hidden unsafe fallback runs learner commands directly on the host.**
14. **Every built-in challenge has machine-verifiable integrity.**
15. **Every milestone can prove itself by running commands, not by explanation.**

---

# 53. Reference ecosystem verification

The following external choices were checked while preparing this architecture on 2026-09-22. Re-check before implementation if substantial time has passed.

- Ratatui: <https://docs.rs/ratatui/latest/ratatui/>
- tui-term: <https://docs.rs/tui-term/latest/tui_term/>
- portable-pty: <https://docs.rs/portable-pty/latest/portable_pty/>
- Frizbee: <https://docs.rs/frizbee/latest/frizbee/>
- KDL Rust parser: <https://docs.rs/kdl/latest/kdl/>
- Prost: <https://docs.rs/prost/latest/prost/>
- Tokio: <https://docs.rs/tokio/latest/tokio/>
- redb: <https://docs.rs/redb/latest/redb/>
- Bubblewrap releases: <https://github.com/containers/bubblewrap/releases>
- Bubblewrap security notes: <https://github.com/containers/bubblewrap>
- Alpine minirootfs: <https://dl-cdn.alpinelinux.org/alpine/latest-stable/releases/x86_64/>
- Brush shell/core: <https://docs.rs/brush-core/latest/brush_core/>

---

# 54. Instructions to the implementation agent

When this plan is handed to an implementation agent:

1. Read the whole plan before editing code.
2. Implement **one phase at a time**.
3. Do not skip runtime verification to make UI progress look faster.
4. Keep code small and typed.
5. Prefer concrete structs/enums over speculative traits.
6. Keep first-party crates `#![forbid(unsafe_code)]`.
7. Add tests with the feature, not after the architecture has drifted.
8. Run the required verification commands before saying a phase is complete.
9. If a runtime assumption is false, demonstrate it with a failing prototype/test and record the architecture change.
10. Never replace a hard behavior with a mock and call the phase complete.
11. Never introduce AI-generated arbitrary code execution.
12. Never silently expose the host when sandbox setup fails.
13. Maintain the learner experience: simple, modern, approachable, real Linux underneath.

The implementation objective is not maximum code volume. It is a small, explainable, verifiable system whose behavior remains obvious to the person maintaining it.
