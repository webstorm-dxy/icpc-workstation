import { beforeEach, describe, expect, it, vi } from "vitest";

const listeners = new Map<string, ((event: { payload: unknown }) => void)[]>();
const invocations: { cmd: string; args: Record<string, unknown> }[] = [];

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (channel: string, handler: (event: { payload: unknown }) => void) => {
    const bucket = listeners.get(channel) ?? [];
    bucket.push(handler);
    listeners.set(channel, bucket);
    return () => {};
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    invocations.push({ cmd, args: args ?? {} });
    if (cmd === "start_lsp_session") {
      return {
        sessionId: `session-${invocations.length}`,
        server: "/opt/homebrew/bin/clangd",
        serverVersion: "clangd 20",
        rootUri: "file:///ws",
        documentUri: "file:///ws/main.cpp",
        workspace: "/ws",
        languageId: "cpp",
      };
    }
    if (cmd === "stop_lsp_session") return true;
    if (cmd === "detect_lsp_servers") return [];
    return null;
  }),
}));

import { acquireLspConnection, disposeConnection } from "./lsp";

function sentMessages(): { method?: string; result?: unknown; jsonrpc?: string }[] {
  return invocations
    .filter((entry) => entry.cmd === "send_lsp_message")
    .map((entry) => JSON.parse(String(entry.args.message)));
}

describe("acquireLspConnection", () => {
  beforeEach(() => {
    invocations.length = 0;
    listeners.clear();
  });

  it("starts a session, subscribes first and sends initialize", async () => {
    const connection = await acquireLspConnection({
      problemId: "cf:integration-1",
      standard: "gnu++17",
    });
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(connection.session.documentUri).toBe("file:///ws/main.cpp");
    expect(listeners.get("lsp://message")?.length).toBe(1);
    expect(listeners.get("lsp://exit")?.length).toBe(1);

    const messages = sentMessages();
    expect(messages.length).toBeGreaterThan(0);
    expect(messages[0].method).toBe("initialize");
    expect(messages[0].jsonrpc).toBe("2.0");

    disposeConnection(connection);
    expect(invocations.some((entry) => entry.cmd === "stop_lsp_session")).toBe(true);
  });

  it("reuses a warm session for the same problem and dialect", async () => {
    const first = await acquireLspConnection({
      problemId: "cf:integration-2",
      standard: "gnu++17",
    });
    const second = await acquireLspConnection({
      problemId: "cf:integration-2",
      standard: "gnu++17",
    });
    expect(second).toBe(first);
    expect(invocations.filter((entry) => entry.cmd === "start_lsp_session").length).toBe(1);
    disposeConnection(first);
  });

  it("forwards server messages to the client transport", async () => {
    const connection = await acquireLspConnection({
      problemId: "cf:integration-3",
      standard: "gnu++17",
    });
    const received: string[] = [];
    connection.transport.onMessage((message) => received.push(message));

    const deliver = listeners.get("lsp://message")?.[0];
    expect(deliver).toBeDefined();
    deliver!({ payload: { sessionId: connection.session.sessionId, message: '{"jsonrpc":"2.0"}' } });
    deliver!({ payload: { sessionId: "someone-else", message: '{"jsonrpc":"2.0"}' } });

    expect(received).toEqual(['{"jsonrpc":"2.0"}']);
    disposeConnection(connection);
  });
});
