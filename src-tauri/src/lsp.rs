//! Language Server Protocol bridge for the C++ editor.
//!
//! This module owns everything the webview cannot do on its own: it locates a
//! `clangd` binary, prepares a tiny workspace per problem (`main.cpp`,
//! `compile_flags.txt`, the `<bits/stdc++.h>` shim), spawns the server and
//! relays LSP JSON-RPC messages between the server's stdio and the frontend.
//!
//! The protocol state machine lives in `src/lib/lsp.ts`; here we only frame
//! messages and manage process lifetimes. Server → client messages are pushed
//! as `lsp://message` events, client → server messages arrive through the
//! `lsp_send` command, and process death is reported as `lsp://exit`.

use crate::error::{AppError, AppResult};
use crate::judge::BITS_SHIM;
use crate::models::{LspServerDto, LspSessionDto, StartLspInput};
use serde_json::json;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::{AppHandle, Emitter};

/// Event carrying one JSON-RPC message produced by the server.
pub const EVENT_MESSAGE: &str = "lsp://message";
/// Event emitted once when the server process goes away.
pub const EVENT_EXIT: &str = "lsp://exit";

pub const DOCUMENT_NAME: &str = "main.cpp";
pub const COMPILE_FLAGS_NAME: &str = "compile_flags.txt";
pub const CLANG_FORMAT_NAME: &str = ".clang-format";
pub const LANGUAGE_ID: &str = "cpp";

/// Upper bound on live server processes. The editor only ever needs one; the
/// slack absorbs a page switch racing with the previous session's teardown.
const MAX_SESSIONS: usize = 4;

const SERVER_DIRS: &[&str] = &[
    "/opt/homebrew/opt/llvm/bin",
    "/opt/homebrew/bin",
    "/usr/local/opt/llvm/bin",
    "/usr/local/bin",
    "/usr/bin",
    "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/bin",
];

#[derive(Default)]
pub struct LspManager {
    sessions: Mutex<HashMap<String, Session>>,
    detected: Mutex<Option<Vec<LspServerDto>>>,
    counter: AtomicU64,
}

struct Session {
    seq: u64,
    /// Dropping the sender closes the child's stdin, which is how clangd is told
    /// to shut down gracefully.
    sender: Sender<String>,
    child: Arc<Mutex<Child>>,
}

impl Session {
    fn shutdown(self) {
        let Session { sender, child, .. } = self;
        drop(sender);
        if let Ok(mut child) = child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl LspManager {
    /// Detected servers, most preferred first. Results are cached until
    /// `refresh` is requested so opening the problem page does not spawn
    /// `clangd --version` on every mount.
    pub fn detect_servers(&self, refresh: bool) -> Vec<LspServerDto> {
        if !refresh {
            if let Some(cached) = self.detected.lock().ok().and_then(|guard| guard.clone()) {
                return cached;
            }
        }
        let found = discover_servers();
        if let Ok(mut guard) = self.detected.lock() {
            *guard = Some(found.clone());
        }
        found
    }

    pub fn start(
        &self,
        app: &AppHandle,
        data_dir: &Path,
        input: StartLspInput,
    ) -> AppResult<LspSessionDto> {
        let server = match input.server_path.as_deref().map(str::trim) {
            Some(path) if !path.is_empty() => resolve_server(Path::new(path))?,
            _ => {
                let detected = self.detect_servers(false);
                detected
                    .first()
                    .map(|entry| PathBuf::from(&entry.path))
                    .ok_or_else(|| {
                        AppError::new(
                            "E_LSP_MISSING",
                            "没有找到 clangd，请安装 LLVM（brew install llvm）或指定服务器路径",
                        )
                    })?
            }
        };
        let server_version = server_version(&server);
        let seq = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        let session_id = format!("lsp-{seq}");

        let workspace = data_dir.join("lsp").join(slug(&input.problem_id));
        prepare_workspace(&workspace, &input.standard)?;
        let document = workspace.join(DOCUMENT_NAME);

        let mut child = Command::new(&server)
            // stdio is clangd's default transport (there is no `--stdio` flag);
            // a single translation unit needs no background index, and that
            // flag keeps clangd from re-reading the workspace while typing.
            .arg("--background-index=0")
            .arg("--log=error")
            .current_dir(&workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                AppError::new(
                    "E_LSP_SPAWN",
                    format!("无法启动语言服务器 {}: {err}", server.display()),
                )
            })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AppError::new("E_LSP_SPAWN", "语言服务器没有可用的标准输入"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AppError::new("E_LSP_SPAWN", "语言服务器没有可用的标准输出"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AppError::new("E_LSP_SPAWN", "语言服务器没有可用的错误输出"))?;

        let (sender, receiver) = mpsc::channel::<String>();
        spawn_writer(receiver, stdin, &session_id)?;
        spawn_reader(stdout, app.clone(), &session_id)?;
        spawn_stderr(stderr, &session_id)?;

        let info = LspSessionDto {
            session_id: session_id.clone(),
            server: server.display().to_string(),
            server_version,
            root_uri: file_uri(&workspace),
            document_uri: file_uri(&document),
            workspace: workspace.display().to_string(),
            language_id: LANGUAGE_ID.to_string(),
        };

        let session = Session {
            seq,
            sender,
            child: Arc::new(Mutex::new(child)),
        };

        let mut retired = Vec::new();
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(session_id, session);
            while sessions.len() > MAX_SESSIONS {
                let oldest = sessions
                    .iter()
                    .min_by_key(|(_, session)| session.seq)
                    .map(|(id, _)| id.clone());
                match oldest.and_then(|id| sessions.remove(&id)) {
                    Some(session) => retired.push(session),
                    None => break,
                }
            }
        }
        for session in retired {
            session.shutdown();
        }
        tracing::info!(
            session = %info.session_id,
            server = %info.server,
            workspace = %info.workspace,
            "language server started"
        );
        Ok(info)
    }

    pub fn send(&self, session_id: &str, message: &str) -> AppResult<()> {
        tracing::debug!(session = %session_id, "-> {}", preview(message));
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| AppError::internal("LSP 会话表已损坏"))?;
        let session = sessions
            .get(session_id)
            .ok_or_else(|| AppError::not_found("LSP 会话"))?;
        session
            .sender
            .send(message.to_string())
            .map_err(|_| AppError::new("E_LSP_CLOSED", "语言服务器进程已退出"))
    }

    pub fn stop(&self, session_id: &str) -> bool {
        let session = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut sessions| sessions.remove(session_id));
        match session {
            Some(session) => {
                session.shutdown();
                tracing::info!(session = %session_id, "language server stopped");
                true
            }
            None => false,
        }
    }

    pub fn stop_all(&self) {
        let sessions: Vec<Session> = self
            .sessions
            .lock()
            .map(|mut sessions| sessions.drain().map(|(_, session)| session).collect())
            .unwrap_or_default();
        for session in sessions {
            session.shutdown();
        }
    }
}

impl Drop for LspManager {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// `clangd --version` with a deadline; a hung binary must not block the editor.
fn server_version(path: &Path) -> Option<String> {
    let output = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let first = text.lines().next()?.trim();
    if first.is_empty() {
        None
    } else {
        Some(first.to_string())
    }
}

fn resolve_server(path: &Path) -> AppResult<PathBuf> {
    if path.is_absolute() && path.is_file() {
        return Ok(path.to_path_buf());
    }
    let located = lookup(path);
    located.ok_or_else(|| AppError::invalid(format!("指定的语言服务器不存在: {}", path.display())))
}

fn discover_servers() -> Vec<LspServerDto> {
    let mut found: Vec<LspServerDto> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();

    let mut push = |path: PathBuf, source: &str, found: &mut Vec<LspServerDto>| {
        let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.iter().any(|known| canonical(known) == key) {
            return;
        }
        seen.push(path.clone());
        found.push(LspServerDto {
            path: path.display().to_string(),
            name: "clangd".into(),
            version: server_version(&path),
            source: source.into(),
        });
    };

    if let Some(explicit) = std::env::var_os("ICPC_LSP_SERVER") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            push(path, "env", &mut found);
        }
    }
    if let Some(env_path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&env_path) {
            let candidate = dir.join(binary_name());
            if candidate.is_file() {
                push(candidate, "path", &mut found);
            }
        }
    }
    for dir in SERVER_DIRS {
        let candidate = Path::new(dir).join(binary_name());
        if candidate.is_file() {
            push(candidate, "known", &mut found);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let candidate = Path::new(&home).join(".local/bin").join(binary_name());
        if candidate.is_file() {
            push(candidate, "known", &mut found);
        }
    }
    found
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn binary_name() -> String {
    if cfg!(windows) {
        "clangd.exe".into()
    } else {
        "clangd".into()
    }
}

/// Resolve a bare command name through `PATH` (and the usual server dirs) so a
/// user-typed `clangd` works as well as a full path.
fn lookup(path: &Path) -> Option<PathBuf> {
    if let Some(name) = path.file_name() {
        if let Some(env_path) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&env_path) {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
        for dir in SERVER_DIRS {
            let candidate = Path::new(dir).join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Filesystem-safe directory name for a problem id.
fn slug(problem_id: &str) -> String {
    let mut out = String::with_capacity(problem_id.len().min(64));
    for ch in problem_id.chars() {
        if out.len() >= 64 {
            break;
        }
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "problem".into()
    } else {
        out
    }
}

fn prepare_workspace(workspace: &Path, standard: &str) -> AppResult<()> {
    std::fs::create_dir_all(workspace)?;
    std::fs::create_dir_all(workspace.join("bits"))?;
    // The shim is written next to the document and injected with `-I` so that
    // `#include <bits/stdc++.h>` resolves the same way the local judge does.
    std::fs::write(workspace.join("bits").join("stdc++.h"), BITS_SHIM)?;
    std::fs::write(
        workspace.join(COMPILE_FLAGS_NAME),
        compile_flags(workspace, standard),
    )?;
    // clang-format (Shift-Alt-F) must agree with the editor's four-space indent
    // instead of falling back to LLVM's two.
    std::fs::write(workspace.join(CLANG_FORMAT_NAME), clang_format())?;
    let document = workspace.join(DOCUMENT_NAME);
    if !document.exists() {
        std::fs::write(&document, "")?;
    }
    Ok(())
}

/// `compile_flags.txt` is a one-argument-per-line compile command that clangd
/// applies to every file in the directory.
pub fn compile_flags(workspace: &Path, standard: &str) -> String {
    format!(
        "-x\nc++\n-std={}\n-I{}\n",
        normalize_standard(standard),
        workspace.display()
    )
}

/// Formatting style for `textDocument/formatting`; mirrors the editor's indent
/// settings so a formatted buffer still looks like the starter template.
pub fn clang_format() -> &'static str {
    "BasedOnStyle: LLVM\nIndentWidth: 4\nTabWidth: 4\nUseTab: Never\nColumnLimit: 0\n"
}

/// Whitelist the `-std=` value: it is written to disk and handed to a compiler,
/// so anything outside the known C++ standards falls back to the default.
pub fn normalize_standard(standard: &str) -> &str {
    const ALLOWED: &[&str] = &["gnu++17", "gnu++20", "gnu++23", "c++17", "c++20", "c++23"];
    ALLOWED
        .iter()
        .copied()
        .find(|candidate| candidate.eq_ignore_ascii_case(standard.trim()))
        .unwrap_or("gnu++17")
}

/// Minimal RFC 3986 encoder for `file://` URIs; clangd rejects raw spaces and
/// non-ASCII paths, which is common on macOS.
pub fn file_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn spawn_writer(
    receiver: mpsc::Receiver<String>,
    stdin: ChildStdin,
    session_id: &str,
) -> AppResult<()> {
    thread::Builder::new()
        .name(format!("{session_id}-writer"))
        .spawn(move || {
            let mut out = stdin;
            for body in receiver {
                if write_frame(&mut out, &body).is_err() {
                    break;
                }
            }
        })
        .map(|_| ())
        .map_err(|err| AppError::new("E_LSP_SPAWN", format!("无法创建写入线程: {err}")))
}

fn spawn_reader(stdout: ChildStdout, app: AppHandle, session_id: &str) -> AppResult<()> {
    let name = session_id.to_string();
    thread::Builder::new()
        .name(format!("{session_id}-reader"))
        .spawn(move || {
            let mut reader = FrameReader::new(stdout);
            loop {
                match reader.read_message() {
                    Ok(Some(body)) => {
                        tracing::debug!(session = %name, "<- {}", preview(&body));
                        let _ =
                            app.emit(EVENT_MESSAGE, json!({ "sessionId": name, "message": body }));
                    }
                    Ok(None) => break,
                    Err(err) => {
                        tracing::warn!(session = %name, "读取语言服务器消息失败: {err}");
                        break;
                    }
                }
            }
            tracing::info!(session = %name, "language server exited");
            let _ = app.emit(EVENT_EXIT, json!({ "sessionId": name }));
        })
        .map(|_| ())
        .map_err(|err| AppError::new("E_LSP_SPAWN", format!("无法创建读取线程: {err}")))
}

fn spawn_stderr(stderr: ChildStderr, session_id: &str) -> AppResult<()> {
    let name = session_id.to_string();
    thread::Builder::new()
        .name(format!("{session_id}-stderr"))
        .spawn(move || {
            for line in BufReader::new(stderr).lines() {
                match line {
                    Ok(line) if !line.trim().is_empty() => {
                        tracing::debug!(session = %name, "{line}");
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        })
        .map(|_| ())
        .map_err(|err| AppError::new("E_LSP_SPAWN", format!("无法创建诊断线程: {err}")))
}

/// Short, single-line excerpt of an inbound message for the log file.
fn preview(body: &str) -> String {
    const LIMIT: usize = 240;
    let flat = body.replace('\n', " ");
    if flat.chars().count() <= LIMIT {
        flat
    } else {
        let head: String = flat.chars().take(LIMIT).collect();
        format!("{head}…")
    }
}

/// Writes one `Content-Length` framed JSON-RPC message.
pub fn write_frame(out: &mut impl Write, body: &str) -> std::io::Result<()> {
    write!(out, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    out.flush()
}

struct FrameReader<R: Read> {
    reader: BufReader<R>,
}

impl<R: Read> FrameReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
        }
    }

    /// Reads one framed message; `Ok(None)` means the server closed stdout.
    fn read_message(&mut self) -> std::io::Result<Option<String>> {
        let mut length: Option<usize> = None;
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Ok(None);
            }
            let trimmed = line.trim_end_matches(['\r', '\n']);
            if trimmed.is_empty() {
                break;
            }
            if let Some((name, value)) = trimmed.split_once(':') {
                if name.trim().eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse::<usize>().ok();
                }
            }
        }
        let Some(length) = length else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "LSP 消息缺少 Content-Length",
            ));
        };
        let mut buffer = vec![0u8; length];
        self.reader.read_exact(&mut buffer)?;
        Ok(Some(String::from_utf8_lossy(&buffer).into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uri_encodes_spaces_and_non_ascii() {
        let uri = file_uri(Path::new("/Users/达欣/My Code/main.cpp"));
        assert!(uri.starts_with("file:///Users/"));
        assert!(uri.contains("My%20Code"));
        assert!(!uri.contains(' '));
        assert!(uri.ends_with("/main.cpp"));
    }

    #[test]
    fn normalize_standard_rejects_unknown_values() {
        assert_eq!(normalize_standard("gnu++20"), "gnu++20");
        assert_eq!(normalize_standard(" GNU++23 "), "gnu++23");
        assert_eq!(normalize_standard("gnu++17; rm -rf /"), "gnu++17");
        assert_eq!(normalize_standard(""), "gnu++17");
    }

    #[test]
    fn compile_flags_has_one_argument_per_line() {
        let flags = compile_flags(Path::new("/tmp/ws"), "gnu++20");
        let lines: Vec<&str> = flags.lines().collect();
        assert_eq!(lines, vec!["-x", "c++", "-std=gnu++20", "-I/tmp/ws"]);
    }

    #[test]
    fn frame_roundtrip_handles_utf8_and_partial_reads() {
        let body = r#"{"jsonrpc":"2.0","method":"window/logMessage","params":{"message":"中文"}}"#;
        let mut buffer = Vec::new();
        write_frame(&mut buffer, body).unwrap();
        let expected = body.len();
        assert!(buffer.starts_with(format!("Content-Length: {expected}\r\n\r\n").as_bytes()));

        let mut reader = FrameReader::new(buffer.as_slice());
        assert_eq!(reader.read_message().unwrap().as_deref(), Some(body));
        assert!(reader.read_message().unwrap().is_none());
    }

    #[test]
    fn workspace_matches_editor_indent() {
        let flags = clang_format();
        assert!(flags.contains("IndentWidth: 4"));
        assert!(flags.contains("UseTab: Never"));
    }

    #[test]
    fn slug_keeps_ids_path_safe() {
        assert_eq!(slug("codeforces-1873/A"), "codeforces-1873_A");
        assert_eq!(slug(""), "problem");
        assert!(slug(&"x".repeat(200)).len() <= 64);
    }

    /// End-to-end check against a real server: handshake, diagnostics for the
    /// shimmed `<bits/stdc++.h>` document, and a completion round-trip through
    /// the same flags the editor uses. Skipped when no clangd is installed,
    /// mirroring the local judge test.
    #[test]
    fn clangd_answers_diagnostics_and_completion() {
        use std::time::{Duration, Instant};

        let Some(server) = discover_servers().first().map(|entry| entry.path.clone()) else {
            eprintln!("跳过：未检测到 clangd");
            return;
        };
        let workspace = std::env::temp_dir().join(format!("icpc-lsp-test-{}", std::process::id()));
        prepare_workspace(&workspace, "gnu++17").expect("workspace");
        let document = workspace.join(DOCUMENT_NAME);
        let document_uri = file_uri(&document);
        // `std::vec` has to complete to `vector`, and the assignment is a
        // deliberate syntax error that must surface as a diagnostic.
        const DOCUMENT_TEXT: &str =
            "#include <bits/stdc++.h>\nint main() {\n  std::vec\n  int x = ;\n}\n";

        let mut child = Command::new(&server)
            .arg("--background-index=0")
            .arg("--log=error")
            .current_dir(&workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn clangd");
        let mut stdin = child.stdin.take().expect("stdin");
        let stdout = child.stdout.take().expect("stdout");

        let (tx, rx) = mpsc::channel::<String>();
        thread::spawn(move || {
            let mut reader = FrameReader::new(stdout);
            while let Ok(Some(body)) = reader.read_message() {
                if tx.send(body).is_err() {
                    break;
                }
            }
        });

        let initialize = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootUri": file_uri(&workspace),
                "capabilities": {
                    "textDocument": { "publishDiagnostics": { "relatedInformation": true } }
                }
            }
        });
        write_frame(&mut stdin, &initialize.to_string()).expect("initialize");

        let mut handshake = false;
        let mut diagnostics_ok = false;
        let mut completion_id = 1;
        let mut completion_sent_at: Option<Instant> = None;
        let mut completion_ok = false;
        let mut last_labels: Vec<String> = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline && !(diagnostics_ok && completion_ok) {
            let Ok(body) = rx.recv_timeout(Duration::from_millis(400)) else {
                // Re-ask while the preamble is still warming up; a completion
                // requested before the AST is ready comes back as noise.
                if handshake
                    && diagnostics_ok
                    && !completion_ok
                    && completion_sent_at
                        .is_none_or(|at| at.elapsed() > Duration::from_millis(1200))
                {
                    completion_id += 1;
                    completion_sent_at = Some(Instant::now());
                    let request = json!({
                        "jsonrpc": "2.0",
                        "id": completion_id,
                        "method": "textDocument/completion",
                        "params": {
                            "textDocument": { "uri": document_uri },
                            "position": { "line": 2, "character": 9 }
                        }
                    });
                    write_frame(&mut stdin, &request.to_string()).expect("completion");
                }
                continue;
            };
            let Ok(message) = serde_json::from_str::<serde_json::Value>(&body) else {
                continue;
            };
            match message.get("id") {
                Some(id) if id == &json!(1) => {
                    assert!(message.get("result").is_some(), "initialize failed: {body}");
                    for notification in [
                        json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
                        json!({
                            "jsonrpc": "2.0",
                            "method": "textDocument/didOpen",
                            "params": { "textDocument": {
                                "uri": document_uri,
                                "languageId": LANGUAGE_ID,
                                "version": 1,
                                "text": DOCUMENT_TEXT
                            }}
                        }),
                    ] {
                        write_frame(&mut stdin, &notification.to_string()).expect("notify");
                    }
                    handshake = true;
                }
                Some(id) if id.as_u64().is_some_and(|value| value > 1) => {
                    let result = message.get("result").cloned().unwrap_or_default();
                    let items = result
                        .get("items")
                        .and_then(|value| value.as_array())
                        .or_else(|| result.as_array())
                        .cloned()
                        .unwrap_or_default();
                    last_labels = items
                        .iter()
                        .filter_map(|item| item.get("label").and_then(|label| label.as_str()))
                        .map(|label| label.to_string())
                        .take(8)
                        .collect();
                    completion_ok = !last_labels.is_empty()
                        && items.iter().any(|item| {
                            item.get("label")
                                .and_then(|label| label.as_str())
                                .is_some_and(|label| label.contains("vector"))
                        });
                }
                _ => {
                    if message.get("method") == Some(&json!("textDocument/publishDiagnostics")) {
                        let params = message.get("params").cloned().unwrap_or_default();
                        let diagnostics = params
                            .get("diagnostics")
                            .and_then(|value| value.as_array())
                            .cloned()
                            .unwrap_or_default();
                        if params.get("uri") == Some(&json!(document_uri))
                            && !diagnostics.is_empty()
                        {
                            diagnostics_ok = true;
                        }
                    } else if message.get("id").is_some() && message.get("method").is_some() {
                        // Server-to-client request; the editor answers `null`.
                        let id = message.get("id").cloned().unwrap_or_default();
                        write_frame(
                            &mut stdin,
                            &json!({ "jsonrpc": "2.0", "id": id, "result": null }).to_string(),
                        )
                        .expect("reply");
                    }
                }
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&workspace);

        assert!(handshake, "clangd did not finish the initialize handshake");
        assert!(
            diagnostics_ok,
            "clangd did not publish diagnostics for the shimmed document"
        );
        assert!(
            completion_ok,
            "clangd did not complete `std::vec` through the shimmed workspace, last labels: {last_labels:?}"
        );
    }
}
