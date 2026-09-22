import { describe, expect, it } from "vite-plus/test";
import { DemoBackend } from "./protocol";

describe("dead renderer backend contract", () => {
  it("keeps challenge and progress state in the snapshot boundary", async () => {
    const backend = new DemoBackend();
    const snapshot = await backend.connect();

    expect(snapshot.protocolVersion).toBe(1);
    expect(snapshot.challenges.length).toBe(snapshot.runtime.challengeCount - 4);
    expect(snapshot.progress.every((item) => item.challengeId)).toBe(true);
    expect(snapshot.challenges.find((item) => item.state === "completed")?.attempts).toBe(1);
  });

  it("emits terminal output through the same subscription used by live clients", async () => {
    const backend = new DemoBackend();
    const events: string[] = [];
    backend.subscribe((event) => {
      if (event.type === "terminal_output") events.push(event.data);
    });
    const session = await backend.openSession("foundation.orientation", 100, 32);
    await backend.sendTerminalInput(session.id, "pwd\r");
    await Promise.resolve();

    expect(events.join(" ")).toContain("/home/learner");
  });
});
