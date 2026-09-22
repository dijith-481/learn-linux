# Web client integration

The web client is a dead renderer: challenge content, verifier state, session
ownership, progress persistence, and terminal output remain backend concerns.
It mirrors the existing `learn-protocol` request/event vocabulary through a
small JSON WebSocket adapter in `src/lib/protocol.ts`.

## Run it

From `web/`:

```bash
vp install
vp dev
vp check
vp test
vp build
```

The browser attempts `ws://127.0.0.1:8787/ws` (override with
`VITE_LEARN_BACKEND_URL`). If no bridge is running, the UI switches to a
deterministic renderer preview so the interaction and visual layer can still
be exercised. Preview data is never used by the Rust TUI or daemon.

## Bridge contract gap

The current Rust daemon serves protobuf frames over a Unix socket only
(`learn-protocol` + `Daemon::serve`). Browsers cannot open that socket, and
the repository does not currently contain an HTTP/WebSocket transport. A
future bridge should translate the existing messages as follows:

| WebSocket JSON | Existing daemon request/event |
| --- | --- |
| `hello` | `HelloRequest` → `HelloResponse` plus initial `BackendSnapshot` |
| `open_session` | `OpenSessionRequest` → `ChallengeResponse` |
| `terminal_input` | `TerminalInput` |
| `terminal_resize` | `TerminalResize` |
| `close_session` | `CloseSession` |
| `reset` | `ResetRequest` |
| `search_docs` | `SearchDocsRequest` → `DocsResponse` |
| `terminal_output` event | `TerminalOutput` event, UTF-8 decoded for the browser |
| `challenge_state` event | `ChallengeStateEvent` |

Each request carries a numeric `requestId` and receives `{ type: "response",
requestId, ok, ... }`. The bridge must preserve the daemon's session lifetime
and forward events only to the owning WebSocket connection. It should also
return the initial snapshot from `StatusRequest`, the compiled challenge
metadata, and persisted progress so the browser never reconstructs verifier
state.

## Verification note

The TypeScript/TSX sources pass a parser-level syntax check, and the Rust
workspace passes `cargo test --workspace --offline`. Full `vp check`, `vp test`,
and `vp build` require the project-local dependencies; `vp install` could not
complete in this environment because the configured package registry timed out.
