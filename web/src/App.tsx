import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import type { BackendEvent, BackendSnapshot, Challenge, LearnBackend, SessionInfo, ThemeId } from "./lib/protocol";
import { DemoBackend, createBackend } from "./lib/protocol";

const THEMES: Array<{ id: ThemeId; label: string; swatch: string }> = [
  { id: "default", label: "Default", swatch: "#7dd3c0" },
  { id: "catppuccin", label: "Catppuccin", swatch: "#89dceb" },
  { id: "nord", label: "Nord", swatch: "#88c0d0" },
  { id: "tokyo-night", label: "Tokyo Night", swatch: "#7dcfff" },
  { id: "everforest", label: "Everforest", swatch: "#a7c080" },
  { id: "everforest-light", label: "Everforest Light", swatch: "#8da101" },
];

const COMMANDS = ["pwd", "ls", "cat", "cd", "mkdir", "touch", "grep", "echo", "clear"];
const PALETTE_COMMANDS = [
  { id: "challenge", label: "Switch challenge", detail: "Open the challenge navigator", key: "C" },
  { id: "progress", label: "View progress", detail: "Review your learning path", key: "P" },
  { id: "vim", label: "Open Vim dojo", detail: "Practice hjkl, motions, and edits", key: "V" },
  { id: "docs", label: "Search local docs", detail: "Find a command in the offline reference", key: "D" },
  { id: "theme", label: "Change theme", detail: "Default, Catppuccin, Nord, Tokyo Night…", key: "T" },
  { id: "reset", label: "Reset current challenge", detail: "Clear this challenge's local progress", key: "R" },
];

function selectedChallenge(snapshot: BackendSnapshot | undefined, id: string): Challenge | undefined {
  return snapshot?.challenges.find((challenge) => challenge.id === id) ?? snapshot?.challenges[0];
}

function cleanOutput(value: string): string {
  return value
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, "")
    .replace(/\r/g, "");
}

function statusLabel(challenge: Challenge): string {
  return challenge.state === "completed" ? "complete" : challenge.state === "in_progress" ? "active" : "ready";
}

export default function App() {
  const [backend, setBackend] = createSignal<LearnBackend>(createBackend());
  const [snapshot, setSnapshot] = createSignal<BackendSnapshot>();
  const [connection, setConnection] = createSignal<"connecting" | "live" | "demo">("connecting");
  const [selectedId, setSelectedId] = createSignal("foundation.orientation");
  const [session, setSession] = createSignal<SessionInfo>();
  const [terminal, setTerminal] = createSignal("waiting for a shell session…\n");
  const [input, setInput] = createSignal("");
  const [history, setHistory] = createSignal<string[]>([]);
  const [historyIndex, setHistoryIndex] = createSignal(-1);
  const [hint, setHint] = createSignal("Try `pwd` to print your working directory.");
  const [paletteOpen, setPaletteOpen] = createSignal(false);
  const [paletteQuery, setPaletteQuery] = createSignal("");
  const [paletteIndex, setPaletteIndex] = createSignal(0);
  const [theme, setTheme] = createSignal<ThemeId>((localStorage.getItem("learn-linux-theme") as ThemeId | null) ?? "default");
  const [vimOpen, setVimOpen] = createSignal(false);
  const [challengeRailOpen, setChallengeRailOpen] = createSignal(false);
  const [docsOpen, setDocsOpen] = createSignal(false);
  const [docsQuery, setDocsQuery] = createSignal("");
  const [docsResult, setDocsResult] = createSignal("Press / to search the offline command reference.");

  const challenge = createMemo(() => selectedChallenge(snapshot(), selectedId()));
  const completedCount = createMemo(() => snapshot()?.runtime.completedCount ?? 0);
  const progressPercent = createMemo(() => {
    const total = snapshot()?.runtime.challengeCount ?? 1;
    return Math.round((completedCount() / total) * 100);
  });
  const suggestions = createMemo(() => {
    const value = input().trim().toLowerCase();
    if (!value) return COMMANDS.slice(0, 4);
    return COMMANDS.filter((command) => command.startsWith(value)).slice(0, 4);
  });
  const paletteItems = createMemo(() => {
    const query = paletteQuery().trim().toLowerCase();
    if (!query) return PALETTE_COMMANDS;
    return PALETTE_COMMANDS.filter((item) => `${item.label} ${item.detail}`.toLowerCase().includes(query));
  });

  const installSubscription = (client: LearnBackend) => client.subscribe((event) => handleBackendEvent(event));

  const openSelectedSession = async (client = backend()) => {
    const current = challenge();
    if (!current) return;
    try {
      const next = await client.openSession(current.id, 100, 32);
      setSession(next);
      setTerminal("");
    } catch {
      setHint("The backend could not open this session. Check the daemon bridge.");
    }
  };

  const handleBackendEvent = (event: BackendEvent) => {
    if (event.type === "terminal_output" && (!session() || event.sessionId === session()?.id)) {
      setTerminal((value) => `${value}${cleanOutput(event.data)}`.slice(-16000));
    }
    if (event.type === "challenge_state") {
      setSnapshot((value) => {
        if (!value) return value;
        return {
          ...value,
          runtime: { ...value.runtime, completedCount: event.state === "completed" ? value.runtime.completedCount + 1 : value.runtime.completedCount },
          challenges: value.challenges.map((item) => item.id === event.challengeId ? { ...item, state: event.state } : item),
        };
      });
      if (event.state === "completed") setHint("Challenge verified. Nice work — choose the next objective when you're ready.");
    }
  };

  onMount(async () => {
    document.documentElement.dataset.theme = theme();
    let client = backend();
    try {
      const next = await client.connect();
      setSnapshot(next);
      setConnection("live");
      const unsubscribe = installSubscription(client);
      onCleanup(unsubscribe);
      await openSelectedSession(client);
    } catch {
      const demo = new DemoBackend();
      const next = await demo.connect();
      setBackend(demo);
      setSnapshot(next);
      setConnection("demo");
      const unsubscribe = installSubscription(demo);
      onCleanup(unsubscribe);
      await openSelectedSession(demo);
    }
  });

  const setThemeValue = (value: ThemeId) => {
    setTheme(value);
    document.documentElement.dataset.theme = value;
    localStorage.setItem("learn-linux-theme", value);
  };

  const selectChallenge = async (id: string) => {
    setSelectedId(id);
    setChallengeRailOpen(false);
    setHint("Objective loaded. Use the shell to make the verifier happy.");
    await openSelectedSession();
  };

  const submitCommand = async (value = input()) => {
    const command = value.trim();
    if (!command || !session()) return;
    setHistory((items) => [command, ...items.filter((item) => item !== command)].slice(0, 12));
    setHistoryIndex(-1);
    setInput("");
    if (command === "clear") {
      setTerminal("");
      setHint("Terminal cleared. The session is still running.");
      return;
    }
    const matchedHint = command === "pwd" ? "Good start. Now inspect the directory with `ls`." : command === "ls" ? "Look for the file named in the objective, then read it with `cat`." : "Command sent to the sandbox; verifier state stays in the backend.";
    setHint(matchedHint);
    await backend().sendTerminalInput(session()!.id, `${command}\r`);
  };

  const runPaletteItem = async (id: string) => {
    setPaletteOpen(false);
    setPaletteQuery("");
    if (id === "challenge") setChallengeRailOpen(true);
    if (id === "progress") setHint("Progress is persisted by the daemon and mirrored in the status rail.");
    if (id === "vim") setVimOpen(true);
    if (id === "docs") setDocsOpen(true);
    if (id === "theme") setHint("Theme picker is in the header; changes are saved locally in this renderer.");
    if (id === "reset" && challenge()) {
      await backend().resetChallenge(challenge()!.id);
      setHint("Challenge reset requested from the backend.");
    }
  };

  const onShellKeyDown = async (event: KeyboardEvent) => {
    if (event.key === "Tab") {
      event.preventDefault();
      const match = suggestions()[0];
      if (match) setInput(`${match} `);
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      const next = Math.min(historyIndex() + 1, history().length - 1);
      setHistoryIndex(next);
      setInput(history()[next] ?? "");
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      const next = Math.max(historyIndex() - 1, -1);
      setHistoryIndex(next);
      setInput(next === -1 ? "" : history()[next] ?? "");
    } else if (event.key === "Enter") {
      event.preventDefault();
      await submitCommand();
    }
  };

  const onGlobalKeyDown = (event: KeyboardEvent) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      setPaletteOpen((value) => !value);
      setChallengeRailOpen(false);
    }
    if (event.key === "Escape") {
      setPaletteOpen(false);
      setDocsOpen(false);
      setVimOpen(false);
      setChallengeRailOpen(false);
    }
  };

  return (
    <div class="app-shell" onKeyDown={onGlobalKeyDown}>
      <header class="topbar">
        <div class="brand-lockup">
          <span class="brand-mark">//</span>
          <span>learn-linux</span>
          <span class="version">v0.1</span>
        </div>
        <div class="header-context"><span class="status-dot" classList={{ live: connection() === "live" }} />{connection() === "live" ? "daemon connected" : connection() === "demo" ? "renderer preview" : "connecting…"}</div>
        <div class="top-actions">
          <label class="theme-control" title="Select a theme">
            <span class="theme-swatch" style={{ "background-color": THEMES.find((item) => item.id === theme())?.swatch }} />
            <select value={theme()} onChange={(event) => setThemeValue(event.currentTarget.value as ThemeId)} aria-label="Theme">
              <For each={THEMES}>{(item) => <option value={item.id}>{item.label}</option>}</For>
            </select>
          </label>
          <button class="palette-trigger" onClick={() => setPaletteOpen(true)}><kbd>⌘ K</kbd> command palette</button>
        </div>
      </header>

      <main class="workspace">
        <aside class="objective-pane panel active-panel" aria-label="Objective and progress">
          <div class="panel-heading">
            <div><span class="eyebrow">CURRENT OBJECTIVE</span><h1>{challenge()?.title ?? "Loading challenge"}</h1></div>
            <span class="state-chip" classList={{ completed: challenge()?.state === "completed" }}>{challenge() ? statusLabel(challenge()!) : "…"}</span>
          </div>
          <p class="objective-copy">{challenge()?.objective}</p>
          <div class="challenge-meta"><span>{challenge()?.track}</span><span>{challenge()?.lesson}</span></div>
          <div class="progress-block">
            <div class="progress-label"><span>PATH PROGRESS</span><strong>{completedCount()} / {snapshot()?.runtime.challengeCount ?? 0}</strong></div>
            <div class="progress-track"><span style={{ width: `${progressPercent()}%` }} /></div>
            <span class="progress-caption">{progressPercent()}% verified by the daemon</span>
          </div>
          <div class="hint-block"><span class="hint-label">LIVE HINT</span><p>{hint()}</p></div>
          <div class="history-block">
            <div class="section-label">COMMAND HISTORY</div>
            <Show when={history().length} fallback={<div class="empty-state">Your commands will appear here.</div>}>
              <For each={history().slice(0, 4)}>{(item, index) => <button class="history-item" onClick={() => setInput(item)}><span>{String(index() + 1).padStart(2, "0")}</span>{item}</button>}</For>
            </Show>
          </div>
          <div class="objective-actions"><button class="text-button" onClick={() => setChallengeRailOpen(true)}>browse challenges <span>→</span></button><button class="text-button" onClick={() => setVimOpen(true)}>vim dojo <span>↗</span></button></div>
        </aside>

        <section class="terminal-pane panel active-panel" aria-label="Shell session">
          <div class="terminal-header"><div><span class="eyebrow">SANDBOX / {session()?.id ?? "offline"}</span><h2>Shell session</h2></div><div class="terminal-actions"><span class="runtime-state"><span class="state-dot" /> runtime ready</span><button class="icon-button" title="Clear terminal" onClick={() => setTerminal("")}>⌫</button></div></div>
          <div class="terminal-output" aria-live="polite"><pre>{terminal()}</pre><div class="shell-line"><span class="prompt">$</span><input autofocus value={input()} onInput={(event) => setInput(event.currentTarget.value)} onKeyDown={onShellKeyDown} aria-label="Shell command" /><span class="block-cursor" /></div></div>
          <div class="autocomplete-row"><span class="autocomplete-label">TAB TO COMPLETE</span><For each={suggestions()}>{(suggestion, index) => <button classList={{ suggested: index() === 0 }} onClick={() => setInput(`${suggestion} `)}>{suggestion}</button>}</For></div>
          <div class="terminal-footer"><span><kbd>↑↓</kbd> history</span><span><kbd>Tab</kbd> autocomplete</span><span><kbd>Ctrl K</kbd> commands</span><span class="footer-spacer" /><span class="connection-label">{connection() === "demo" ? "bridge unavailable · preview data" : "events synced"}</span></div>
        </section>

        <aside class="session-pane panel quiet-panel" aria-label="Session details">
          <div class="section-label">SESSION</div>
          <div class="session-stat"><span>ATTEMPTS</span><strong>{challenge()?.attempts ?? 0}</strong></div>
          <div class="session-stat"><span>HINTS USED</span><strong>{challenge()?.hintCount ?? 0}</strong></div>
          <div class="session-stat"><span>PROTOCOL</span><strong>v{snapshot()?.protocolVersion ?? 1}</strong></div>
          <div class="rail-divider" />
          <div class="section-label">VIM DOJO</div>
          <div class="vim-mini"><div class="vim-mode">NORMAL</div><div class="vim-keys"><kbd>h</kbd><kbd>j</kbd><kbd>k</kbd><kbd>l</kbd></div><p>Make terminal fluency muscle memory.</p><button class="outline-button" onClick={() => setVimOpen(true)}>enter dojo <span>↗</span></button></div>
          <div class="rail-divider" />
          <div class="section-label">DAEMON</div>
          <p class="daemon-copy">{snapshot()?.runtime.summary ?? "waiting for status"}</p>
          <button class="text-button docs-link" onClick={() => setDocsOpen(true)}>offline docs <span>⌕</span></button>
        </aside>
      </main>

      <Show when={paletteOpen()}><div class="modal-scrim" onClick={() => setPaletteOpen(false)}><section class="palette modal-panel" onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === "ArrowDown") { event.preventDefault(); setPaletteIndex((value) => Math.min(value + 1, paletteItems().length - 1)); } if (event.key === "ArrowUp") { event.preventDefault(); setPaletteIndex((value) => Math.max(value - 1, 0)); } if (event.key === "Enter") { event.preventDefault(); const item = paletteItems()[paletteIndex()]; if (item) void runPaletteItem(item.id); } }}><div class="palette-input"><span>⌕</span><input autofocus value={paletteQuery()} onInput={(event) => { setPaletteQuery(event.currentTarget.value); setPaletteIndex(0); }} placeholder="Search commands…" /></div><div class="palette-list"><For each={paletteItems()}>{(item, index) => <button classList={{ selected: index() === paletteIndex() }} onClick={() => runPaletteItem(item.id)}><span class="palette-glyph">{item.key}</span><span><strong>{item.label}</strong><small>{item.detail}</small></span><span class="palette-enter">↵</span></button>}</For></div><div class="palette-footer"><span><kbd>↑↓</kbd> navigate</span><span><kbd>↵</kbd> select</span><span><kbd>esc</kbd> close</span></div></section></div></Show>

      <Show when={challengeRailOpen()}><div class="drawer-scrim" onClick={() => setChallengeRailOpen(false)}><section class="challenge-drawer" onClick={(event) => event.stopPropagation()}><div class="drawer-heading"><div><span class="eyebrow">LEARNING PATH</span><h2>Choose a challenge</h2></div><button class="icon-button" onClick={() => setChallengeRailOpen(false)}>×</button></div><div class="challenge-list"><For each={snapshot()?.challenges}>{(item) => <button classList={{ selected: item.id === selectedId() }} onClick={() => selectChallenge(item.id)}><span class="challenge-index">{item.lesson}</span><span class="challenge-title"><strong>{item.title}</strong><small>{item.objective}</small></span><span class={`challenge-state ${item.state}`}>{item.state === "completed" ? "✓" : item.state === "in_progress" ? "•" : "○"}</span></button>}</For></div></section></div></Show>

      <Show when={vimOpen()}><div class="drawer-scrim" onClick={() => setVimOpen(false)}><section class="vim-drawer" onClick={(event) => event.stopPropagation()}><div class="drawer-heading"><div><span class="eyebrow">PRACTICE MODE</span><h2>Vim dojo</h2></div><button class="icon-button" onClick={() => setVimOpen(false)}>×</button></div><div class="dojo-stage"><div class="dojo-mode"><span class="state-dot" /> NORMAL</div><div class="dojo-board"><div class="dojo-caret" /><span class="dojo-text">Navigate this line with motions.</span></div><p>Use <kbd>h</kbd> <kbd>j</kbd> <kbd>k</kbd> <kbd>l</kbd> to move. Try <kbd>w</kbd> for the next word or <kbd>/</kbd> to search.</p><div class="dojo-progress"><span style={{ width: "42%" }} /></div><div class="dojo-actions"><button class="outline-button" onClick={() => setVimOpen(false)}>return to shell</button><span>level 02 / 05</span></div></div></section></div></Show>

      <Show when={docsOpen()}><div class="modal-scrim" onClick={() => setDocsOpen(false)}><section class="docs-modal modal-panel" onClick={(event) => event.stopPropagation()}><div class="drawer-heading"><div><span class="eyebrow">LOCAL REFERENCE</span><h2>Command docs</h2></div><button class="icon-button" onClick={() => setDocsOpen(false)}>×</button></div><div class="docs-search"><span>/</span><input autofocus value={docsQuery()} onInput={(event) => setDocsQuery(event.currentTarget.value)} onKeyDown={async (event) => { if (event.key === "Enter") { const result = await backend().searchDocs(docsQuery(), 1); setDocsResult(result[0]?.snippet ?? "No matching entry."); } }} placeholder="grep, pipes, permissions…" /></div><p class="docs-result">{docsResult()}</p></section></div></Show>
    </div>
  );
}
