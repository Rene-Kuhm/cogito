//! Measures `context_pack` recall against the labelled gold set.
//!
//! This is an opt-in measurement, not a correctness test: it prints a per-task
//! table and the average, and only asserts the mechanics, so a broken harness
//! fails loudly instead of quietly reporting a good number.
//!
//! Run it with:
//!
//! ```text
//! COGITO_RECALL=1 cargo test --release -p cogito-harness \
//!     --test context_pack_recall -- --ignored --nocapture
//! ```
//!
//! The repositories have to be present in `.cogito/bench-cache`, which is where
//! `cogito bench` puts them; a missing checkout is reported and skipped rather
//! than counted as a miss.
//!
//! It drives the real MCP tool through a real process, so the measurement uses
//! the same tokenizer and the same code path an agent gets. Each task is run
//! twice and the two answers must be identical, which checks determinism at the
//! interface rather than only in the library.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("workspace root")
}

fn cogito_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cogito"))
}

#[derive(Deserialize)]
struct Gold {
    tasks: Vec<Task>,
}

#[derive(Deserialize)]
struct Task {
    id: String,
    repo: String,
    budget: usize,
    task: String,
    expect: Expect,
}

#[derive(Deserialize, Default)]
struct Expect {
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    symbols: Vec<String>,
}

/// One fragment header, parsed back out of the rendered pack.
struct Header {
    path: String,
    start: usize,
    end: usize,
    name: String,
}

/// The fragment headers in a rendered pack.
///
/// The header is `path:start-end · kind · name · reason`. A line that does not
/// have that shape is the task line, the trailer, or a line of source.
fn headers(pack: &str) -> Vec<Header> {
    let mut out = Vec::new();
    for line in pack.lines() {
        let Some((location, rest)) = line.split_once(" · ") else {
            continue;
        };
        let Some((path, span)) = location.rsplit_once(':') else {
            continue;
        };
        if line.starts_with("context_pack ") {
            continue;
        }
        let Some((start, end)) = span.split_once('-') else {
            continue;
        };
        let (Ok(start), Ok(end)) = (start.parse::<usize>(), end.parse::<usize>()) else {
            continue;
        };
        let name = rest.split(" · ").nth(1).unwrap_or_default().to_string();
        out.push(Header {
            path: path.to_string(),
            start,
            end,
            name,
        });
    }
    out
}

/// The `used` figure the header reports.
fn used_tokens(pack: &str) -> usize {
    pack.split("used ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Ask the tool one question and return its answer.
///
/// stdin stays open until the answer for `id: 2` has been read. Closing it first
/// and then draining stdout happens to work, but it leans on the server treating
/// an early end of input as "finish what you have": a server that instead
/// cancels in-flight requests when its input closes would drop the answer. This
/// reads the answer, and only then closes stdin and reaps the process.
fn ask(path: &Path, task: &str, budget: usize) -> String {
    let mut child = Command::new(cogito_bin())
        .args(["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn cogito mcp");

    let path_arg = path.display().to_string();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"recall","version":"1.0"}}}"#;
    let ready = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
    let call = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "context_pack",
            "arguments": { "path": path_arg, "task": task, "budget": budget }
        }
    })
    .to_string();

    {
        let stdin = child.stdin.as_mut().expect("stdin");
        for message in [init, ready, &call] {
            writeln!(stdin, "{message}").expect("write");
        }
        stdin.flush().expect("flush");
    }

    // Collect stdout until the answer arrives, with stdin still open.
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let mut answer: Option<String> = None;
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // the server closed its output
            Ok(_) => {}
            Err(e) => panic!("reading from cogito mcp: {e}"),
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if value.get("id") != Some(&serde_json::json!(2)) {
            continue;
        }
        answer = Some(
            value["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
        );
        break;
    }

    // Only now close stdin, then reap the process.
    drop(child.stdin.take());
    let status = child.wait().expect("wait");
    answer.unwrap_or_else(|| {
        panic!("no answer for the task {task:?}: the server exited with {status}")
    })
}

#[test]
#[ignore = "opt-in measurement: needs the pinned checkouts and prints a table"]
fn test_context_pack_recall() {
    if std::env::var("COGITO_RECALL").ok().as_deref() != Some("1") {
        eprintln!("skipping: set COGITO_RECALL=1 to run the recall measurement");
        return;
    }

    let root = workspace_root();
    let gold_path = root.join("benchmarks/context_pack_gold.json");
    let gold: Gold = serde_json::from_str(
        &std::fs::read_to_string(&gold_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", gold_path.display())),
    )
    .expect("parse the gold set");

    let mut per_repo: BTreeMap<String, usize> = BTreeMap::new();
    let mut recall_sum = 0.0f64;
    let mut counted = 0usize;

    println!("\ncontext_pack recall@budget over the labelled set\n");
    println!(
        "{:<7} {:<8} {:>7} {:>6} {:>9} {:>6} {:>8}",
        "task", "repo", "budget", "used", "expected", "hits", "recall"
    );
    println!("{}", "-".repeat(60));

    for task in &gold.tasks {
        let repo_root = root.join(".cogito/bench-cache").join(&task.repo);
        if !repo_root.is_dir() {
            println!(
                "{:<7} {:<8}   checkout missing at {}, skipped",
                task.id,
                task.repo,
                repo_root.display()
            );
            continue;
        }

        // Warm the index once per repository so the first task is not paying a
        // cold build, then measure.
        let first = ask(&repo_root, &task.task, task.budget);
        let second = ask(&repo_root, &task.task, task.budget);
        assert_eq!(
            first, second,
            "task {} answered differently twice: the tool must be deterministic",
            task.id
        );
        // A second process is a fresh index state; both runs agreeing is also
        // evidence that the pack does not depend on process-local state.
        assert!(
            first.contains("context_pack \""),
            "task {} did not return a pack:\n{first}",
            task.id
        );

        let found = headers(&first);
        // Every header must describe a range that can exist: the acceptance rule
        // checked at the real interface, not only in the library tests.
        for h in &found {
            assert!(
                h.start >= 1 && h.start <= h.end,
                "task {} returned {}:{}-{} which is not a usable range",
                task.id,
                h.path,
                h.start,
                h.end
            );
        }
        let expected: Vec<String> = task
            .expect
            .files
            .iter()
            .cloned()
            .chain(task.expect.symbols.iter().cloned())
            .collect();

        let mut hits = 0usize;
        let mut missing: Vec<String> = Vec::new();
        for want in &task.expect.files {
            if found.iter().any(|h| &h.path == want) {
                hits += 1;
            } else {
                missing.push(format!("file {want}"));
            }
        }
        for want in &task.expect.symbols {
            let want_lower = want.to_lowercase();
            if found.iter().any(|h| h.name.to_lowercase() == want_lower) {
                hits += 1;
            } else {
                missing.push(format!("symbol {want}"));
            }
        }

        let recall = hits as f64 / expected.len() as f64;
        recall_sum += recall;
        counted += 1;
        *per_repo.entry(task.repo.clone()).or_insert(0) += 1;

        let used = used_tokens(&first);
        assert!(
            used <= task.budget,
            "task {} spent {used} tokens of a {} budget",
            task.id,
            task.budget
        );

        println!(
            "{:<7} {:<8} {:>7} {:>6} {:>9} {:>6} {:>7.0}%",
            task.id,
            task.repo,
            task.budget,
            used,
            expected.len(),
            hits,
            recall * 100.0
        );
        if !missing.is_empty() {
            println!("        missed: {}", missing.join(", "));
        }
    }

    println!("{}", "-".repeat(60));
    assert!(
        counted > 0,
        "no task was measured: are the checkouts present?"
    );
    println!(
        "average recall@budget over {counted} tasks: {:.1}%\n",
        (recall_sum / counted as f64) * 100.0
    );
}
