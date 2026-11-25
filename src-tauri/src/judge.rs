use crate::db::Db;
use crate::error::{AppError, AppResult};
use crate::models::{
    JudgeResultDto, RunSamplesRequest, SampleResultDto, SettingsDto, ToolchainDto,
};
use crate::util::{NormalizeOptions, normalize_output, sha256_hex};
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const MAX_OUTPUT: usize = 1024 * 1024;

pub async fn detect_toolchains() -> Vec<ToolchainDto> {
    tokio::task::spawn_blocking(detect_toolchains_blocking)
        .await
        .unwrap_or_default()
}

fn detect_toolchains_blocking() -> Vec<ToolchainDto> {
    let mut candidates: Vec<String> = Vec::new();
    if let Ok(value) = std::env::var("ICPC_CXX") {
        if !value.trim().is_empty() {
            candidates.push(value);
        }
    }
    for name in [
        "g++-17", "g++-16", "g++-15", "g++-14", "g++", "clang++", "c++",
    ] {
        candidates.push(name.to_string());
    }
    let mut out: Vec<ToolchainDto> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in candidates {
        let output = Command::new(&name).arg("--version").output();
        let Ok(output) = output else { continue };
        if !output.status.success() {
            continue;
        }
        let version = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if version.is_empty() {
            continue;
        }
        let kind =
            if version.to_ascii_lowercase().contains("clang") || version.contains("Apple LLVM") {
                "clang"
            } else if version
                .to_ascii_lowercase()
                .contains("free software foundation")
            {
                "gcc"
            } else {
                "unknown"
            };
        let path = which_path(&name).unwrap_or(name.clone());
        if !seen.insert(path.clone()) {
            continue;
        }
        let supports_bits = probe_bits(&path);
        out.push(ToolchainDto {
            path,
            kind: kind.to_string(),
            version,
            supports_bits,
        });
    }
    // GNU compilers first: ICPC uses g++ semantics.
    out.sort_by_key(|t| {
        let rank = if t.supports_bits {
            0
        } else if t.kind == "gcc" {
            1
        } else if t.kind == "clang" {
            2
        } else {
            3
        };
        (rank, t.path.clone())
    });
    out
}

fn which_path(name: &str) -> Option<String> {
    if name.contains('/') {
        return Some(name.to_string());
    }
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

fn probe_bits(compiler: &str) -> bool {
    let dir = std::env::temp_dir().join(format!("icpc-probe-{}", uuid::Uuid::now_v7()));
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let source = dir.join("probe.cpp");
    let _ = std::fs::write(&source, "#include <bits/stdc++.h>\nint main(){return 0;}\n");
    let ok = Command::new(compiler)
        .args(["-std=gnu++17", "-fsyntax-only"])
        .arg(&source)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

pub async fn run_samples(db: &Db, req: RunSamplesRequest) -> AppResult<JudgeResultDto> {
    let samples = crate::platforms::load_problem_samples(db, &req.problem_id).await?;
    if samples.is_empty() {
        return Err(AppError::new(
            "E_JUDGE_NO_SAMPLES",
            "这道题还没有样例。请刷新题面，或在题目页面手动查看样例。",
        ));
    }
    let settings = db
        .get_json::<SettingsDto>("settings")
        .await
        .unwrap_or_default();
    let compiler = req
        .compiler_path
        .clone()
        .or(settings.compiler_path.clone())
        .unwrap_or_default();
    let standard = req
        .standard
        .clone()
        .or(Some(settings.compiler_standard.clone()))
        .unwrap_or_else(|| "gnu++17".into());
    let problem = sqlx::query("SELECT time_limit_ms, memory_limit_mb FROM problems WHERE id = ?1")
        .bind(&req.problem_id)
        .fetch_optional(&db.pool)
        .await?
        .ok_or_else(|| AppError::not_found("题目"))?;
    use sqlx::Row;
    let time_limit = problem
        .get::<Option<i64>, _>("time_limit_ms")
        .unwrap_or(2000)
        .clamp(200, 15000);
    let samples_for_run = samples.clone();
    let code = req.code.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_blocking(compiler, standard, code, samples_for_run, time_limit, None)
    })
    .await
    .map_err(AppError::internal)?;

    if let Ok(dto) = &result {
        for sample in &dto.samples {
            let _ = sqlx::query(
                "INSERT INTO run_results(id, problem_id, sample_index, status, time_ms, stdout, stderr, expected, code_sha256, created_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&req.problem_id)
            .bind(sample.index)
            .bind(&sample.status)
            .bind(sample.time_ms)
            .bind(&sample.stdout)
            .bind(&sample.stderr)
            .bind(&sample.expected)
            .bind(sha256_hex(&req.code))
            .bind(crate::util::now_iso())
            .execute(&db.pool)
            .await;
        }
    }
    if let Some(save_draft) = req.save_draft {
        if save_draft {
            let _ = save_draft_code(db, &req.problem_id, &req.code, "cpp").await;
        }
    }
    result
}

pub async fn save_draft_code(
    db: &Db,
    problem_id: &str,
    code: &str,
    language: &str,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO drafts(problem_id, language, code_text, updated_at) VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(problem_id) DO UPDATE SET language = excluded.language, code_text = excluded.code_text, updated_at = excluded.updated_at",
    )
    .bind(problem_id)
    .bind(language)
    .bind(code)
    .bind(crate::util::now_iso())
    .execute(&db.pool)
    .await?;
    Ok(())
}

pub async fn get_draft(db: &Db, problem_id: &str) -> Option<(String, String)> {
    let row = sqlx::query("SELECT language, code_text FROM drafts WHERE problem_id = ?1")
        .bind(problem_id)
        .fetch_optional(&db.pool)
        .await
        .ok()
        .flatten()?;
    use sqlx::Row;
    Some((
        row.get::<String, _>("language"),
        row.get::<String, _>("code_text"),
    ))
}

pub fn pick_compiler(preferred: Option<String>) -> AppResult<ToolchainDto> {
    if let Some(path) = preferred {
        if !path.trim().is_empty() {
            return Ok(ToolchainDto {
                path,
                kind: "custom".into(),
                version: "自定义编译器".into(),
                supports_bits: true,
            });
        }
    }
    let list = detect_toolchains_blocking();
    list.into_iter().next().ok_or_else(|| {
        AppError::new(
            "E_JUDGE_COMPILER",
            "没有找到 C++ 编译器，请安装 g++ 或在设置中填写路径",
        )
    })
}

fn run_blocking(
    compiler: String,
    standard: String,
    code: String,
    samples: Vec<crate::models::SampleDto>,
    time_limit_ms: i64,
    _submission_id: Option<String>,
) -> AppResult<JudgeResultDto> {
    let toolchain = pick_compiler(if compiler.trim().is_empty() {
        None
    } else {
        Some(compiler.clone())
    })?;
    let standard = if standard.trim().is_empty() {
        "gnu++17".to_string()
    } else {
        standard
    };
    let workdir = std::env::temp_dir().join(format!("icpc-judge-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&workdir)?;
    let source = workdir.join("main.cpp");
    std::fs::write(&source, &code)?;
    let needs_bits_shim = code.contains("bits/stdc++.h") && !toolchain.supports_bits;
    if needs_bits_shim {
        std::fs::write(workdir.join("bits/stdc++.h"), BITS_SHIM)?;
    }
    let binary = workdir.join("main");
    let started = Instant::now();
    let mut compile = Command::new(&toolchain.path);
    compile
        .arg("-O2")
        .arg(format!("-std={standard}"))
        .arg("-pipe")
        .arg("-o")
        .arg(&binary)
        .arg(&source)
        .stdin(Stdio::null());
    if needs_bits_shim {
        compile.arg("-I").arg(&workdir);
    }
    let mut compile_result = run_process(&mut compile, Duration::from_secs(20), None);
    if !compile_result.success
        && !compile_result.timed_out
        && code.contains("bits/stdc++.h")
        && !needs_bits_shim
    {
        // A custom/preferred compiler may be clang without libstdc++'s bits header.
        if std::fs::write(workdir.join("bits/stdc++.h"), BITS_SHIM).is_ok() {
            let mut retry = Command::new(&toolchain.path);
            retry
                .arg("-O2")
                .arg(format!("-std={standard}"))
                .arg("-pipe")
                .arg("-I")
                .arg(&workdir)
                .arg("-o")
                .arg(&binary)
                .arg(&source)
                .stdin(Stdio::null());
            compile_result = run_process(&mut retry, Duration::from_secs(20), None);
        }
    }
    let compile_output = format!("{}{}", compile_result.stdout, compile_result.stderr);
    if compile_result.timed_out {
        let _ = std::fs::remove_dir_all(&workdir);
        return Ok(JudgeResultDto {
            ok: false,
            compile_ok: false,
            compile_output: "编译超时（20 秒）".into(),
            samples: vec![],
            total_ms: started.elapsed().as_millis() as i64,
            compiler: Some(toolchain.path),
        });
    }
    if !compile_result.success {
        let _ = std::fs::remove_dir_all(&workdir);
        return Ok(JudgeResultDto {
            ok: false,
            compile_ok: false,
            compile_output,
            samples: vec![],
            total_ms: started.elapsed().as_millis() as i64,
            compiler: Some(toolchain.path),
        });
    }
    let mut sample_results = Vec::new();
    for (index, sample) in samples.iter().enumerate() {
        let input_path = workdir.join(format!("in{index}.txt"));
        if let Err(err) = std::fs::write(&input_path, &sample.input) {
            sample_results.push(SampleResultDto {
                index: index as i64 + 1,
                status: "RE".into(),
                time_ms: 0,
                stdout: String::new(),
                stderr: format!("无法写入测试输入: {err}"),
                expected: sample.output.clone(),
                diff: String::new(),
            });
            continue;
        }
        let mut run = Command::new(&binary);
        let run_timeout = Duration::from_millis(time_limit_ms.max(200) as u64 + 1_500);
        let result = run_process(&mut run, run_timeout, Some(&input_path));
        let expected = sample.output.clone();
        let (status, diff) = if result.timed_out {
            ("TLE".to_string(), String::new())
        } else if !result.success {
            ("RE".to_string(), String::new())
        } else {
            let actual = normalize_output(&result.stdout, NormalizeOptions::default());
            let expected_norm = normalize_output(&expected, NormalizeOptions::default());
            if actual == expected_norm {
                ("OK".to_string(), String::new())
            } else {
                ("WA".to_string(), make_diff(&expected_norm, &actual))
            }
        };
        sample_results.push(SampleResultDto {
            index: index as i64 + 1,
            status,
            time_ms: result.elapsed_ms,
            stdout: truncate(&result.stdout, MAX_OUTPUT),
            stderr: truncate(&result.stderr, MAX_OUTPUT),
            expected,
            diff,
        });
    }
    let _ = std::fs::remove_dir_all(&workdir);
    let ok = sample_results.iter().all(|s| s.status == "OK");
    Ok(JudgeResultDto {
        ok,
        compile_ok: true,
        compile_output,
        samples: sample_results,
        total_ms: started.elapsed().as_millis() as i64,
        compiler: Some(toolchain.path),
    })
}

struct ProcessResult {
    success: bool,
    timed_out: bool,
    stdout: String,
    stderr: String,
    elapsed_ms: i64,
}

fn run_process(
    command: &mut Command,
    timeout: Duration,
    stdin_path: Option<&Path>,
) -> ProcessResult {
    if let Some(path) = stdin_path {
        if let Ok(file) = std::fs::File::open(path) {
            command.stdin(Stdio::from(file));
        }
    } else {
        command.stdin(Stdio::null());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return ProcessResult {
                success: false,
                timed_out: false,
                stdout: String::new(),
                stderr: format!("无法启动进程: {err}"),
                elapsed_ms: started.elapsed().as_millis() as i64,
            };
        }
    };
    let stdout = child
        .stdout
        .take()
        .map(|mut s| std::thread::spawn(move || read_capped(&mut s)));
    let stderr = child
        .stderr
        .take()
        .map(|mut s| std::thread::spawn(move || read_capped(&mut s)));
    let mut timed_out = false;
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                break None;
            }
        }
    };
    let stdout = stdout.and_then(|h| h.join().ok()).unwrap_or_default();
    let stderr = stderr.and_then(|h| h.join().ok()).unwrap_or_default();
    ProcessResult {
        success: status.map(|s| s.success()).unwrap_or(false),
        timed_out,
        stdout,
        stderr,
        elapsed_ms: started.elapsed().as_millis() as i64,
    }
}

fn read_capped(reader: &mut impl Read) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if buf.len() < MAX_OUTPUT {
                    let take = (MAX_OUTPUT - buf.len()).min(n);
                    buf.extend_from_slice(&chunk[..take]);
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

fn truncate(input: &str, max: usize) -> String {
    if input.len() <= max {
        input.to_string()
    } else {
        let mut cut = max;
        while cut > 0 && !input.is_char_boundary(cut) {
            cut -= 1;
        }
        format!("{}…（输出过长已截断）", &input[..cut])
    }
}

fn make_diff(expected: &str, actual: &str) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    let max = expected_lines.len().max(actual_lines.len()).min(80);
    let mut out = String::new();
    for i in 0..max {
        let e = expected_lines.get(i).copied().unwrap_or("<缺失>");
        let a = actual_lines.get(i).copied().unwrap_or("<缺失>");
        if e != a {
            out.push_str(&format!("第 {} 行\n期望: {e}\n实际: {a}\n", i + 1));
            if out.len() > 2000 {
                out.push('…');
                break;
            }
        }
    }
    if out.is_empty() {
        out.push_str("输出内容不同（可能是行尾空白差异）");
    }
    out
}

/// Minimal stand-in for libstdc++'s `<bits/stdc++.h>` so clang builds and
/// clangd diagnostics work on toolchains that do not ship the umbrella header.
pub(crate) const BITS_SHIM: &str = r#"#pragma once
#include <algorithm>
#include <array>
#include <bitset>
#include <cassert>
#include <cctype>
#include <cerrno>
#include <cfloat>
#include <chrono>
#include <climits>
#include <cmath>
#include <complex>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <deque>
#include <exception>
#include <fstream>
#include <functional>
#include <iomanip>
#include <ios>
#include <iostream>
#include <istream>
#include <iterator>
#include <limits>
#include <list>
#include <map>
#include <memory>
#include <numeric>
#include <ostream>
#include <queue>
#include <random>
#include <set>
#include <sstream>
#include <stack>
#include <stdexcept>
#include <string>
#include <tuple>
#include <type_traits>
#include <unordered_map>
#include <unordered_set>
#include <utility>
#include <vector>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_reports_first_difference() {
        let diff = make_diff("1\n2\n", "1\n3\n");
        assert!(diff.contains("第 2 行"));
    }

    #[tokio::test]
    async fn compiles_and_runs_a_sample() {
        let tools = detect_toolchains().await;
        if tools.is_empty() {
            return;
        }
        let chosen = tools
            .iter()
            .find(|tool| tool.supports_bits)
            .or_else(|| tools.first())
            .cloned();
        let dir = std::env::temp_dir().join(format!("icpc-judge-test-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = crate::db::Db::open(&dir.join("test.db")).await.unwrap();
        sqlx::query(
            "INSERT INTO problems(id, platform, native_id, title, url, normalized_rating, tags_json, skill_tags_json, samples_json, time_limit_ms, memory_limit_mb)
             VALUES('t:1','codeforces','1A','Test','https://example.com',800,'[]','[]',?1,2000,256)",
        )
        .bind(r#"[{"input":"1 2\n","output":"3\n"}]"#)
        .execute(&db.pool)
        .await
        .unwrap();
        let result = run_samples(
            &db,
            crate::models::RunSamplesRequest {
                problem_id: "t:1".into(),
                code: "#include <bits/stdc++.h>\nint main(){int a,b;std::cin>>a>>b;std::cout<<a+b<<\"\\n\";}".into(),
                compiler_path: chosen.map(|tool| tool.path),
                standard: Some("gnu++17".into()),
                save_draft: Some(false),
            },
        )
        .await
        .unwrap();
        assert!(result.compile_ok, "{}", result.compile_output);
        assert!(result.ok, "{:?}", result.samples);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncates_on_char_boundary() {
        let s = "一二三四五";
        let t = truncate(s, 4);
        assert!(t.starts_with("一"));
        assert!(t.contains("截断"));
    }
}
