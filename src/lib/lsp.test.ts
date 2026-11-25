import { Text } from "@codemirror/state";
import type { CompletionResult } from "@codemirror/autocomplete";
import type { EditorView } from "@codemirror/view";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./api", () => ({
  api: {
    sendLspMessage: vi.fn(async () => {}),
    stopLspSession: vi.fn(async () => true),
    startLspSession: vi.fn(),
    detectLspServers: vi.fn(async () => []),
  },
  events: {
    onLspMessage: vi.fn(async () => () => {}),
    onLspExit: vi.fn(async () => () => {}),
  },
  errText: (error: unknown) => String(error),
}));

import { api } from "./api";
import { lspCapabilities, lspKey, mergeCompletions, offsetToPos, TauriLspTransport } from "./lsp";

describe("offsetToPos", () => {
  const doc = Text.of(["first line", "second"]);

  it("converts offsets to zero-based LSP positions", () => {
    expect(offsetToPos(doc, 0)).toEqual({ line: 0, character: 0 });
    expect(offsetToPos(doc, 11)).toEqual({ line: 1, character: 0 });
    expect(offsetToPos(doc, 14)).toEqual({ line: 1, character: 3 });
  });

  it("clamps out-of-range offsets instead of throwing", () => {
    expect(offsetToPos(doc, -5)).toEqual({ line: 0, character: 0 });
    expect(offsetToPos(doc, 9_999)).toEqual({ line: 1, character: 6 });
  });
});

describe("lspKey", () => {
  it("separates problems, dialects and server choices", () => {
    const base = { problemId: "codeforces-1A", standard: "gnu++17" };
    expect(lspKey(base)).toBe("codeforces-1A::gnu++17::auto");
    expect(lspKey({ ...base, standard: "gnu++20" })).not.toBe(lspKey(base));
    expect(lspKey({ ...base, serverPath: "/usr/bin/clangd" })).not.toBe(lspKey(base));
  });
});

describe("mergeCompletions", () => {
  const local: CompletionResult & { from: number } = {
    from: 5,
    options: [
      { label: "fori", type: "snippet", apply: "for (int i = 0; i < n; i++) {}" },
      { label: "vector", type: "type" },
    ],
  };
  const remote: CompletionResult & { from: number } = {
    from: 7,
    options: [{ label: "vector" }, { label: "vectorize" }],
    filter: false,
  };

  it("returns whichever side produced results", () => {
    expect(mergeCompletions(null, remote)).toBe(remote);
    expect(mergeCompletions(local, null)).toBe(local);
    expect(mergeCompletions(null, null)).toBeNull();
  });

  it("merges from the earliest replaced position without duplicating labels", () => {
    const merged = mergeCompletions(local, remote)!;
    expect(merged.from).toBe(5);
    expect(merged.options.map((option) => option.label)).toEqual([
      "fori",
      "vector",
      "vectorize",
    ]);
    // The server already ranked its list, so CodeMirror must not re-filter it.
    expect(merged.filter).toBe(false);
  });

  it("pins local options to their own range and leaves server options alone", () => {
    const merged = mergeCompletions(local, remote)!;
    const snippet = merged.options[0];
    const serverOption = merged.options[2];
    expect(typeof snippet.apply).toBe("function");
    // LSP options keep the package's own apply (or none at all).
    expect(serverOption.apply).toBeUndefined();

    const dispatches: unknown[] = [];
    const view = { dispatch: (tr: unknown) => dispatches.push(tr) } as unknown as EditorView;
    (snippet.apply as (v: EditorView, c: unknown, from: number, to: number) => void)(
      view,
      snippet,
      0, // merged range starts before the local one
      9,
    );
    expect(dispatches).toEqual([
      { changes: { from: 5, to: 9, insert: "for (int i = 0; i < n; i++) {}" } },
    ]);
  });
});

describe("lspCapabilities", () => {
  it("lists the features clangd advertises", () => {
    expect(
      lspCapabilities({
        completionProvider: {},
        hoverProvider: true,
        definitionProvider: true,
        documentFormattingProvider: true,
        documentHighlightProvider: true,
        renameProvider: true,
      } as never),
    ).toEqual(["补全", "悬停", "跳转定义", "格式化", "诊断"]);
    expect(lspCapabilities(null)).toEqual([]);
  });
});

describe("TauriLspTransport", () => {
  beforeEach(() => {
    vi.mocked(api.sendLspMessage).mockClear();
  });

  it("keeps IPC calls in order even when the first one is slow", async () => {
    const seen: string[] = [];
    let release: (() => void) | null = null;
    vi.mocked(api.sendLspMessage).mockImplementation(async (_session, message) => {
      seen.push(message);
      if (message === "didOpen" && !release) {
        await new Promise<void>((resolve) => {
          release = resolve;
        });
      }
    });

    const transport = new TauriLspTransport("session-1");
    await transport.ready;
    transport.send("didOpen");
    transport.send("didChange");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(seen).toEqual(["didOpen"]);

    release!();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(seen).toEqual(["didOpen", "didChange"]);
  });

  it("stops the backend session and reports close exactly once", async () => {
    const transport = new TauriLspTransport("session-2");
    await transport.ready;
    const onClose = vi.fn();
    const onError = vi.fn();
    transport.onClose(onClose);
    transport.onError(onError);

    transport.close();
    transport.close();
    transport.send("ignored");

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onError).not.toHaveBeenCalled();
    expect(api.stopLspSession).toHaveBeenCalledTimes(1);
    expect(api.sendLspMessage).not.toHaveBeenCalled();
  });
});
