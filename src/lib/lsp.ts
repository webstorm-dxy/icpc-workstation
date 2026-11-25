/**
 * Language Server Protocol client for the C++ editor.
 *
 * The Rust side (`src-tauri/src/lsp.rs`) owns the clangd process; this module
 * owns the protocol. It provides three things:
 *
 * 1. `TauriLspTransport` — a `Transport` that moves JSON-RPC frames through
 *    Tauri commands/events instead of a WebSocket, so the app keeps working
 *    under the locked-down CSP and never opens a local port.
 * 2. A session cache keyed by problem + standard + server path. CodeMirror
 *    views come and go (React StrictMode, page switches) while a clangd
 *    process is expensive to start, so views share one connection per key.
 * 3. `lspExtensions()` / `useLspConnection()` — the CodeMirror extension stack
 *    and the React binding that keeps the editor's LSP status in sync.
 */
import { autocompletion } from "@codemirror/autocomplete";
import type {
  Completion,
  CompletionContext,
  CompletionResult,
  CompletionSource,
} from "@codemirror/autocomplete";
import { lintGutter } from "@codemirror/lint";
import type { Extension, Text } from "@codemirror/state";
import { hoverTooltip, keymap } from "@codemirror/view";
import {
  LanguageServerClient,
  SynchronizationMethod,
  formatDocument,
  formatSelection,
  jumpToDefinition,
  languageServerPlugin,
} from "codemirror-languageserver";
import type { Transport } from "codemirror-languageserver";
import { CompletionTriggerKind } from "vscode-languageserver-protocol";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api, errText, events } from "./api";
import { cppCompletion } from "./cppCompletion";
import type { LspServer, LspSession } from "../types";

/** How many clangd processes may stay warm at once. */
const MAX_CONNECTIONS = 2;

export type LspPhase = "disabled" | "starting" | "ready" | "error";

export interface LspConnection {
  key: string;
  session: LspSession;
  client: LanguageServerClient;
  transport: TauriLspTransport;
  lastError: string | null;
  lastUsed: number;
  closed: boolean;
  /** Set when the cache tears the connection down on purpose; the UI must not
   *  report a deliberate restart as "the server died". */
  disposing: boolean;
  listeners: Set<() => void>;
}

/**
 * JSON-RPC transport backed by Tauri IPC.
 *
 * `send` calls are chained: Tauri dispatches each `invoke` as its own task, so
 * without the queue a `didOpen` could overtake the `didChange` that follows it.
 */
export class TauriLspTransport implements Transport {
  readonly sessionId: string;
  readonly ready: Promise<void>;
  private messageHandlers: ((message: string) => void)[] = [];
  private closeHandlers: (() => void)[] = [];
  private errorHandlers: ((error: Error) => void)[] = [];
  private unlisten: (() => void)[] = [];
  private queue: Promise<void> = Promise.resolve();
  private closed = false;
  private closeEmitted = false;

  constructor(sessionId: string) {
    this.sessionId = sessionId;
    this.ready = this.subscribe();
  }

  private async subscribe(): Promise<void> {
    const offs = await Promise.all([
      events.onLspMessage((event) => {
        if (event.sessionId !== this.sessionId) return;
        for (const handler of this.messageHandlers) handler(event.message);
      }),
      events.onLspExit((event) => {
        if (event.sessionId !== this.sessionId) return;
        this.emitClose();
      }),
    ]);
    if (this.closed) {
      for (const off of offs) off();
      return;
    }
    this.unlisten = offs;
  }

  send(message: string): void {
    if (this.closed) return;
    this.queue = this.queue
      .then(() => api.sendLspMessage(this.sessionId, message))
      .catch((error) => {
        this.emitError(new Error(errText(error)));
      });
  }

  onMessage(callback: (message: string) => void): void {
    this.messageHandlers.push(callback);
  }

  onClose(callback: () => void): void {
    this.closeHandlers.push(callback);
  }

  onError(callback: (error: Error) => void): void {
    this.errorHandlers.push(callback);
  }

  close(): void {
    if (this.closed) return;
    this.closed = true;
    for (const off of this.unlisten) off();
    this.unlisten = [];
    void api.stopLspSession(this.sessionId).catch(() => {});
    this.emitClose();
  }

  private emitError(error: Error): void {
    for (const handler of this.errorHandlers) handler(error);
  }

  private emitClose(): void {
    if (this.closeEmitted) return;
    this.closeEmitted = true;
    this.closed = true;
    for (const handler of this.closeHandlers) handler();
  }
}

// ---------------------------------------------------------------------------
// Connection cache
// ---------------------------------------------------------------------------

const connections = new Map<string, LspConnection>();
const starting = new Map<string, Promise<LspConnection>>();

export interface LspConfig {
  problemId: string;
  standard: string;
  serverPath?: string | null;
}

export function lspKey(config: LspConfig): string {
  return [config.problemId, config.standard, config.serverPath ?? "auto"].join("::");
}

export function acquireLspConnection(config: LspConfig): Promise<LspConnection> {
  const key = lspKey(config);
  const live = connections.get(key);
  if (live && !live.closed) {
    live.lastUsed = Date.now();
    return Promise.resolve(live);
  }
  const pending = starting.get(key);
  if (pending) return pending;
  const promise = startConnection(key, config).finally(() => starting.delete(key));
  starting.set(key, promise);
  return promise;
}

async function startConnection(key: string, config: LspConfig): Promise<LspConnection> {
  const session = await api.startLspSession({
    problemId: config.problemId,
    standard: config.standard,
    serverPath: config.serverPath ?? null,
  });
  const transport = new TauriLspTransport(session.sessionId);

  const connection: LspConnection = {
    key,
    session,
    transport,
    client: null as unknown as LanguageServerClient,
    lastError: null,
    lastUsed: Date.now(),
    closed: false,
    disposing: false,
    listeners: new Set(),
  };

  try {
    await transport.ready;
    connection.client = new LanguageServerClient({
      transport,
      rootUri: session.rootUri,
      workspaceFolders: null,
      documentUri: session.documentUri,
      languageId: session.languageId,
      // The connection outlives individual editor views, so teardown is driven
      // by the cache (or by the app exiting) rather than by the last detach.
      autoClose: false,
      locale: "zh-CN",
      onError: (error) => {
        connection.lastError = error.message;
      },
      onClose: () => {
        connection.closed = true;
        notifyClosed(connection);
      },
    });
  } catch (error) {
    transport.close();
    throw error;
  }

  connections.set(key, connection);
  enforceLimit(key);
  return connection;
}

export function disposeConnection(connection: LspConnection): void {
  connections.delete(connection.key);
  connection.closed = true;
  connection.disposing = true;
  try {
    connection.client.close();
  } catch {
    // A half-initialized client can throw while rejecting pending requests.
  }
  // `client.close()` already closes the transport; calling it again is a no-op
  // and covers the case where the client never finished constructing.
  connection.transport.close();
}

/** Drops warm connections that are no longer the active one. */
function enforceLimit(keep: string): void {
  for (;;) {
    const alive = [...connections.values()].filter((item) => !item.closed);
    if (alive.length <= MAX_CONNECTIONS) return;
    const victim = alive
      .filter((item) => item.key !== keep)
      .sort((a, b) => a.lastUsed - b.lastUsed)[0];
    if (!victim) return;
    disposeConnection(victim);
  }
}

function notifyClosed(connection: LspConnection): void {
  if (connection.disposing) return;
  for (const listener of connection.listeners) {
    try {
      listener();
    } catch {
      // A listener that throws must not keep the others from running.
    }
  }
}

// ---------------------------------------------------------------------------
// Protocol helpers
// ---------------------------------------------------------------------------

export interface LspPosition {
  line: number;
  character: number;
}

export function offsetToPos(doc: Text, offset: number): LspPosition {
  const clamped = Math.max(0, Math.min(offset, doc.length));
  const line = doc.lineAt(clamped);
  return { line: line.number - 1, character: clamped - line.from };
}

/**
 * Combines the curated snippet/keyword list with clangd's semantic results.
 *
 * CodeMirror only carries one replacement range per result, and the two
 * sources disagree on it (`std::ve` starts at `std` locally but at `ve` for the
 * server). Local options therefore get an `apply` pinned to their own range,
 * while LSP options keep theirs (clangd sends `textEdit` ranges). Filtering is
 * left to the sources: the server already ranked its own list.
 */
export function mergeCompletions(
  local: CompletionResult | null,
  remote: CompletionResult | null,
): CompletionResult | null {
  if (!local) return remote;
  if (!remote) return local;
  const from = Math.min(local.from, remote.from);
  const options: Completion[] = [];
  const seen = new Set<string>();
  const push = (option: Completion, pin: number | null) => {
    const label = String(option.label);
    if (seen.has(label)) return;
    seen.add(label);
    options.push(pin === null ? option : pinApply(option, pin));
  };
  for (const option of local.options) push(option, local.from);
  for (const option of remote.options) push(option, null);
  return { from, options, filter: false };
}

function pinApply(option: Completion, from: number): Completion {
  if (typeof option.apply === "function") return option;
  const insert = typeof option.apply === "string" ? option.apply : String(option.label);
  return {
    ...option,
    apply: (view, _completion, _from, to) => {
      const start = Math.min(from, to);
      view.dispatch({ changes: { from: start, to: Math.max(start, to), insert } });
    },
  };
}

async function requestLspCompletion(
  context: CompletionContext,
): Promise<CompletionResult | null> {
  const view = context.view;
  if (!view) return null;
  const plugin = view.plugin(languageServerPlugin);
  if (!plugin) return null;
  const line = context.state.doc.lineAt(context.pos);
  const previous = context.pos > line.from ? line.text[context.pos - line.from - 1] : "";
  const triggers = plugin.client.capabilities?.completionProvider?.triggerCharacters ?? [];
  let triggerKind: CompletionTriggerKind = CompletionTriggerKind.Invoked;
  let triggerCharacter: string | undefined;
  if (!context.explicit && previous && triggers.includes(previous)) {
    triggerKind = CompletionTriggerKind.TriggerCharacter;
    triggerCharacter = previous;
  }
  if (
    !context.explicit &&
    triggerKind === CompletionTriggerKind.Invoked &&
    !context.matchBefore(/\w+$/)
  ) {
    return null;
  }
  try {
    return await plugin.requestCompletion(
      context,
      offsetToPos(context.state.doc, context.pos),
      { triggerKind, triggerCharacter },
    );
  } catch {
    // A request that dies mid-flight (server restart) must not break typing.
    return null;
  }
}

/** Snippets first, then clangd's semantic list, de-duplicated by label. */
export const cppLspCompletion: CompletionSource = async (context) => {
  const local = await cppCompletion(context);
  const remote = await requestLspCompletion(context);
  return mergeCompletions(local, remote);
};

/**
 * CodeMirror extensions that turn a plain buffer into an LSP-backed one.
 *
 * The package's own `languageServerWithTransport()` is not used because it
 * installs `autocompletion({ override: [...] })` itself; merging its override
 * with the local snippet source would either throw on the facet or hide the
 * server's completions entirely.
 */
export function lspExtensions(connection: LspConnection): Extension[] {
  return [
    languageServerPlugin.of({
      client: connection.client,
      documentUri: connection.session.documentUri,
      languageId: connection.session.languageId,
      allowHTMLContent: false,
      // Full sync: a single competitive-programming translation unit is tiny,
      // and a dropped incremental edit (typing during the initialize
      // round-trip) would leave the server's copy silently out of date.
      synchronizationMethod: SynchronizationMethod.Full,
    }),
    hoverTooltip(
      (view, pos) => {
        const plugin = view.plugin(languageServerPlugin);
        if (!plugin) return null;
        return plugin.requestHoverTooltip(view, offsetToPos(view.state.doc, pos));
      },
      { hoverTime: 250 },
    ),
    autocompletion({ override: [cppLspCompletion] }),
    lintGutter(),
    keymap.of([
      { key: "F12", run: jumpToDefinition, preventDefault: true },
      { key: "Shift-Alt-f", run: formatDocument, preventDefault: true },
      { key: "Shift-Alt-g", run: formatSelection, preventDefault: true },
    ]),
  ];
}

/**
 * Capability chips for the toolbar tooltip. Only features this module actually
 * wires are listed: clangd also advertises rename/highlight, but those
 * extensions are not installed here, so claiming them would be misleading.
 */
export function lspCapabilities(
  capabilities: LanguageServerClient["capabilities"] | null,
): string[] {
  if (!capabilities) return [];
  const out: string[] = [];
  if (capabilities.completionProvider) out.push("补全");
  if (capabilities.hoverProvider) out.push("悬停");
  if (capabilities.definitionProvider) out.push("跳转定义");
  if (capabilities.documentFormattingProvider) out.push("格式化");
  // Diagnostics are pushed as `textDocument/publishDiagnostics`; clangd does
  // not gate them behind a capability flag.
  out.push("诊断");
  return out;
}

// ---------------------------------------------------------------------------
// React binding
// ---------------------------------------------------------------------------

export interface LspStatus {
  phase: LspPhase;
  error: string | null;
  server: string | null;
  serverVersion: string | null;
  capabilities: LanguageServerClient["capabilities"] | null;
}

const IDLE_STATUS: LspStatus = {
  phase: "disabled",
  error: null,
  server: null,
  serverVersion: null,
  capabilities: null,
};

export interface UseLspOptions {
  enabled: boolean;
  problemId: string;
  standard: string;
  serverPath?: string | null;
}

export function useLspConnection({ enabled, problemId, standard, serverPath }: UseLspOptions) {
  const [connection, setConnection] = useState<LspConnection | null>(null);
  const [status, setStatus] = useState<LspStatus>(
    enabled ? { ...IDLE_STATUS, phase: "starting" } : IDLE_STATUS,
  );
  const [attempt, setAttempt] = useState(0);
  const active = useRef<LspConnection | null>(null);

  useEffect(() => {
    if (!enabled || !problemId) {
      if (active.current) {
        disposeConnection(active.current);
        active.current = null;
      }
      setConnection(null);
      setStatus(IDLE_STATUS);
      return;
    }
    let cancelled = false;
    let detach: (() => void) | null = null;
    setStatus((prev) => ({ ...IDLE_STATUS, phase: "starting", server: prev.server }));

    acquireLspConnection({ problemId, standard, serverPath })
      .then(async (conn) => {
        if (cancelled) return;
        active.current = conn;
        setConnection(conn);
        const base = {
          server: conn.session.server,
          serverVersion: conn.session.serverVersion ?? null,
        };
        const onClosed = () => {
          if (cancelled) return;
          setStatus((prev) =>
            prev.phase === "error"
              ? prev
              : { ...prev, phase: "error", error: prev.error ?? "语言服务器已退出" },
          );
        };
        conn.listeners.add(onClosed);
        detach = () => conn.listeners.delete(onClosed);

        if (!conn.client.ready) await conn.client.initializePromise;
        if (cancelled) return;
        if (conn.client.ready) {
          setStatus({
            phase: "ready",
            error: null,
            capabilities: conn.client.capabilities,
            ...base,
          });
        } else {
          setStatus({
            phase: "error",
            error: conn.lastError ?? "语言服务器初始化失败",
            capabilities: null,
            ...base,
          });
        }
      })
      .catch((error) => {
        if (cancelled) return;
        setStatus({ ...IDLE_STATUS, phase: "error", error: errText(error) });
      });

    return () => {
      cancelled = true;
      detach?.();
    };
  }, [enabled, problemId, standard, serverPath, attempt]);

  const restart = useCallback(() => {
    const current = active.current;
    if (current) disposeConnection(current);
    active.current = null;
    setAttempt((value) => value + 1);
  }, []);

  return { status, connection, restart };
}

/** Detected servers, cached for the lifetime of the page. */
export function useLspServers(): {
  servers: LspServer[];
  refresh: (force?: boolean) => Promise<void>;
} {
  const [servers, setServers] = useState<LspServer[]>([]);
  const refresh = useCallback(async (force = true) => {
    try {
      setServers(await api.detectLspServers(force));
    } catch {
      setServers([]);
    }
  }, []);
  useEffect(() => {
    void refresh(false);
  }, [refresh]);
  return useMemo(() => ({ servers, refresh }), [servers, refresh]);
}
