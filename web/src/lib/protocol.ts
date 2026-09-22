export type ThemeId =
  | "default"
  | "catppuccin"
  | "nord"
  | "tokyo-night"
  | "everforest"
  | "everforest-light";

export type ChallengeState = "available" | "in_progress" | "completed";

export interface Challenge {
  id: string;
  title: string;
  objective: string;
  track: string;
  lesson: string;
  state: ChallengeState;
  attempts: number;
  hintCount: number;
}

export interface ProgressSnapshot {
  challengeId: string;
  state: ChallengeState;
  attempts: number;
  hintsUsed: number;
}

export interface RuntimeStatus {
  ready: boolean;
  summary: string;
  challengeCount: number;
  completedCount: number;
}

export interface BackendSnapshot {
  protocolVersion: number;
  serverVersion: string;
  runtime: RuntimeStatus;
  challenges: Challenge[];
  progress: ProgressSnapshot[];
}

export interface SessionInfo {
  id: string;
  challengeId: string;
  title: string;
  objective: string;
}

export type BackendEvent =
  | { type: "terminal_output"; sessionId: string; data: string }
  | { type: "challenge_state"; challengeId: string; state: ChallengeState };

export interface DocResult {
  id: string;
  title: string;
  snippet: string;
}

export interface LearnBackend {
  readonly kind: "websocket" | "demo";
  connect(): Promise<BackendSnapshot>;
  openSession(challengeId: string, columns: number, rows: number): Promise<SessionInfo>;
  sendTerminalInput(sessionId: string, data: string): Promise<void>;
  resizeTerminal(sessionId: string, columns: number, rows: number): Promise<void>;
  closeSession(sessionId: string): Promise<void>;
  resetChallenge(challengeId: string): Promise<void>;
  searchDocs(query: string, limit: number): Promise<DocResult[]>;
  subscribe(listener: (event: BackendEvent) => void): () => void;
}

type WireRequest = {
  type: string;
  requestId: number;
  [key: string]: unknown;
};

type WireResponse = {
  type: "response";
  requestId: number;
  ok: boolean;
  error?: string;
  [key: string]: unknown;
};

type WireEvent = {
  type: "terminal_output" | "challenge_state";
  [key: string]: unknown;
};

/**
 * Browser-side adapter for the loopback WebSocket bridge served by `learn daemon`.
 *
 * The bridge keeps the Rust daemon's protobuf/Unix-socket contract private;
 * this JSON shape is intentionally one-to-one with the existing request and
 * event names so a bridge can translate without moving verifier state here.
 */
export class WebSocketBackend implements LearnBackend {
  readonly kind = "websocket" as const;
  private socket?: WebSocket;
  private nextRequestId = 1;
  private snapshot?: BackendSnapshot;
  private readonly pending = new Map<
    number,
    {
      resolve: (response: WireResponse) => void;
      reject: (error: Error) => void;
    }
  >();
  private readonly listeners = new Set<(event: BackendEvent) => void>();

  constructor(private readonly url = "ws://127.0.0.1:8787/ws") {}

  async connect(): Promise<BackendSnapshot> {
    this.socket = new WebSocket(this.url);
    this.socket.addEventListener("message", (message) => this.receive(message.data));
    await new Promise<void>((resolve, reject) => {
      const socket = this.socket;
      if (!socket) return reject(new Error("WebSocket was not created"));
      socket.addEventListener("open", () => resolve(), { once: true });
      socket.addEventListener(
        "error",
        () => reject(new Error(`Unable to connect to ${this.url}`)),
        { once: true },
      );
    });
    const response = await this.request({ type: "hello", clientKind: "web" });
    this.snapshot = response.snapshot as BackendSnapshot;
    return this.snapshot;
  }

  async openSession(challengeId: string, columns: number, rows: number): Promise<SessionInfo> {
    const response = await this.request({ type: "open_session", challengeId, columns, rows });
    return response.session as SessionInfo;
  }

  async sendTerminalInput(sessionId: string, data: string): Promise<void> {
    await this.request({ type: "terminal_input", sessionId, data });
  }

  async resizeTerminal(sessionId: string, columns: number, rows: number): Promise<void> {
    await this.request({ type: "terminal_resize", sessionId, columns, rows });
  }

  async closeSession(sessionId: string): Promise<void> {
    await this.request({ type: "close_session", sessionId });
  }

  async resetChallenge(challengeId: string): Promise<void> {
    await this.request({ type: "reset", challengeId });
  }

  async searchDocs(query: string, limit: number): Promise<DocResult[]> {
    const response = await this.request({ type: "search_docs", query, limit });
    return response.results as DocResult[];
  }

  subscribe(listener: (event: BackendEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private request(payload: Omit<WireRequest, "requestId">): Promise<WireResponse> {
    const socket = this.socket;
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return Promise.reject(new Error("backend is not connected"));
    }
    const requestId = this.nextRequestId++;
    return new Promise((resolve, reject) => {
      this.pending.set(requestId, { resolve, reject });
      socket.send(JSON.stringify({ ...payload, requestId }));
    });
  }

  private receive(raw: unknown): void {
    let message: WireResponse | WireEvent;
    try {
      message = JSON.parse(String(raw)) as WireResponse | WireEvent;
    } catch {
      return;
    }
    if (message.type === "response") {
      const pending = this.pending.get(message.requestId);
      if (!pending) return;
      this.pending.delete(message.requestId);
      if (message.ok) pending.resolve(message);
      else pending.reject(new Error(message.error ?? "backend rejected the request"));
      return;
    }
    if (message.type === "terminal_output") {
      const event: BackendEvent = {
        type: "terminal_output",
        sessionId: String(message.sessionId),
        data: typeof message.data === "string" ? message.data : "",
      };
      this.listeners.forEach((listener) => listener(event));
    } else if (message.type === "challenge_state") {
      const event: BackendEvent = {
        type: "challenge_state",
        challengeId: String(message.challengeId),
        state: message.state as ChallengeState,
      };
      this.listeners.forEach((listener) => listener(event));
    }
  }
}

/** A deterministic renderer fixture used when no bridge is running locally. */
export class DemoBackend implements LearnBackend {
  readonly kind = "demo" as const;
  private readonly listeners = new Set<(event: BackendEvent) => void>();
  private sessionId = "demo-session";
  private readonly data: BackendSnapshot = {
    protocolVersion: 1,
    serverVersion: "0.1.0-demo",
    runtime: {
      ready: true,
      summary: "embedded WASM runtime ready",
      challengeCount: 10,
      completedCount: 3,
    },
    challenges: [
      {
        id: "foundation.orientation",
        title: "Find your bearings",
        objective: "Print the working directory and list the files around you.",
        track: "FOUNDATIONS",
        lesson: "01 / 10",
        state: "in_progress",
        attempts: 2,
        hintCount: 1,
      },
      {
        id: "foundation.files",
        title: "Read the room",
        objective: "Inspect a file without changing it.",
        track: "FOUNDATIONS",
        lesson: "02 / 10",
        state: "available",
        attempts: 0,
        hintCount: 0,
      },
      {
        id: "foundation.paths",
        title: "Paths are a language",
        objective: "Move through a nested directory using an absolute path.",
        track: "FOUNDATIONS",
        lesson: "03 / 10",
        state: "available",
        attempts: 0,
        hintCount: 0,
      },
      {
        id: "shell.pipes",
        title: "Pass the stream",
        objective: "Connect two small commands with a pipe.",
        track: "SHELL CRAFT",
        lesson: "04 / 10",
        state: "completed",
        attempts: 1,
        hintCount: 0,
      },
      {
        id: "shell.grep",
        title: "Search with intent",
        objective: "Find one exact line in a file with grep.",
        track: "SHELL CRAFT",
        lesson: "05 / 10",
        state: "completed",
        attempts: 1,
        hintCount: 0,
      },
      {
        id: "shell.redirect",
        title: "Leave a trace",
        objective: "Redirect command output into a new file.",
        track: "SHELL CRAFT",
        lesson: "06 / 10",
        state: "available",
        attempts: 0,
        hintCount: 0,
      },
    ],
    progress: [
      { challengeId: "foundation.orientation", state: "in_progress", attempts: 2, hintsUsed: 1 },
      { challengeId: "shell.pipes", state: "completed", attempts: 1, hintsUsed: 0 },
      { challengeId: "shell.grep", state: "completed", attempts: 1, hintsUsed: 0 },
    ],
  };

  async connect(): Promise<BackendSnapshot> {
    return structuredClone(this.data);
  }

  async openSession(challengeId: string, _columns = 100, _rows = 32): Promise<SessionInfo> {
    const challenge =
      this.data.challenges.find((item) => item.id === challengeId) ?? this.data.challenges[0];
    this.sessionId = `demo-${challenge.id}`;
    queueMicrotask(() =>
      this.emit({
        type: "terminal_output",
        sessionId: this.sessionId,
        data: "Learn Linux sandbox · session ready\r\n$ ",
      }),
    );
    return {
      id: this.sessionId,
      challengeId: challenge.id,
      title: challenge.title,
      objective: challenge.objective,
    };
  }

  async sendTerminalInput(sessionId: string, data: string): Promise<void> {
    const command = data.replace(/\r?\n/g, "").trim();
    queueMicrotask(() => {
      this.emit({ type: "terminal_output", sessionId, data: `${command}\r\n` });
      if (command === "pwd")
        this.emit({ type: "terminal_output", sessionId, data: "/home/learner\r\n$ " });
      else if (command === "ls")
        this.emit({
          type: "terminal_output",
          sessionId,
          data: "notes.txt  projects  welcome.txt\r\n$ ",
        });
      else
        this.emit({
          type: "terminal_output",
          sessionId,
          data: `bash: ${command || ""}: demo shell hint — try pwd or ls\r\n$ `,
        });
    });
  }

  async resizeTerminal(): Promise<void> {}
  async closeSession(): Promise<void> {}
  async resetChallenge(): Promise<void> {}
  async searchDocs(query: string): Promise<DocResult[]> {
    return [
      {
        id: "command.pwd",
        title: "pwd — print working directory",
        snippet: `Use pwd to answer “where am I?”${query ? ` · matched “${query}”` : ""}`,
      },
    ];
  }
  subscribe(listener: (event: BackendEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
  private emit(event: BackendEvent): void {
    this.listeners.forEach((listener) => listener(event));
  }
}

export function createBackend(): LearnBackend {
  return new WebSocketBackend(import.meta.env.VITE_LEARN_BACKEND_URL ?? "ws://127.0.0.1:8787/ws");
}
