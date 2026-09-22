# Learn Linux

Learn Linux is a local-first Linux learning environment built around typed
challenge content, a portable POSIX-like shell simulation, and verifiable
filesystem state. Real Bash remains an optional native backend.

The default runtime is a self-contained WASM-packaged Linux simulation. It
does not require Bubblewrap, a rootfs, Bash, or any host command. Each lesson
gets a fresh virtual home directory with deterministic POSIX-like command,
pipe, redirection, wildcard, and verifier semantics. The native Bubblewrap
backend remains an optional advanced backend for future lessons that genuinely
need a real kernel.

## Development

```bash
cargo test --workspace
cargo run -- content check
cargo run -- content simulate --all
cargo run -- doctor --json
cargo run
```

content simulate --all executes every canonical solution through the embedded
runtime and verifies the resulting virtual filesystem. cargo run opens the
full-screen Ratatui client immediately; no runtime installation is needed.

learn doctor reports the embedded WASM lesson runtime as the required
capability. Bubblewrap, namespaces, PTY support, and a rootfs are shown only
as optional native-backend information. The simulator is intentionally not
presented as a full Linux kernel: it is the portable execution target for
content compiled against its supported command and verifier contract.

The full-screen client uses Ratatui + Crossterm, with `vt100` parsing the
portable terminal stream. A small, state-driven success reveal uses the
community `tachyonfx` animation library; calm motion can be toggled from the
command palette. Press `Ctrl-Space` for the centered searchable command
palette, which can open docs, challenges, progress, Vim dojo, reset the
current challenge, toggle the pet/rain/motion, change themes, or quit. `Ctrl-H`
focuses the terminal and `Ctrl-L` focuses the always-visible local docs pane.
The active shell has a visible block cursor, arrow/Home/End editing, and Tab
completion for commands and lesson files. There are no function-key controls;
all actions are discoverable through the palette and the visible focus hints.
