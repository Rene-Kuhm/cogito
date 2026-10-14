//! Measures `context_pack` recall against the labelled gold set.
//!
//! This is an opt-in measurement, not a correctness test: it prints its tables
//! and only asserts the mechanics, so a broken harness fails loudly instead of
//! quietly reporting a good number.
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
//!
//! Three tables come out of one run:
//!
//! 1. every task with its budget and what it spent,
//! 2. the same results split by the language the task was written in,
//! 3. the tier weights varied, because the choice of ten / three / one is a
//!    judgement and should be measured rather than asserted.

use serde::Deserialize;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Spread across the tier-weight variants, in percentage points, above which the
/// choice of weights is reported as not validated by this set.
const UNSTABLE_SPREAD_PP: f64 = 10.0;

/// The tier-weight sets the sensitivity table compares. `None` is the default.
const TIER_VARIANTS: [Option<&str>; 3] = [None, Some("5/2/1"), Some("20/5/1")];

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
    /// The language the task text is written in: `en` or `es`.
    language: String,
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
        if line.starts_with("context_pack ") {
            continue;
        }
        let Some((path, span)) = location.rsplit_once(':') else {
            continue;
        };
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
/// cancelled in-flight requests when its input closed would drop the answer.
fn ask(path: &Path, task: &str, budget: usize, tiers: Option<&str>) -> String {
    let mut command = Command::new(cogito_bin());
    command
        .args(["mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(spec) = tiers {
        command.env("COGITO_PACK_TIERS", spec);
    }
    let mut child = command.spawn().expect("spawn cogito mcp");

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

/// One task, measured.
struct Measured {
    id: String,
    language: String,
    budget: usize,
    used: usize,
    expected: usize,
    hits: usize,
    recall: f64,
    missing: Vec<String>,
}

/// Run every task once and collect what each scored.
fn measure(gold: &Gold, root: &Path, tiers: Option<&str>) -> Vec<Measured> {
    let mut results = Vec::new();
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

        let first = ask(&repo_root, &task.task, task.budget, tiers);
        let second = ask(&repo_root, &task.task, task.budget, tiers);
        assert_eq!(
            first, second,
            "task {} answered differently twice: the tool must be deterministic",
            task.id
        );
        assert!(
            first.contains("context_pack \""),
            "task {} did not return a pack:\n{first}",
            task.id
        );

        let found = headers(&first);
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
        let mut missing = Vec::new();
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

        let used = used_tokens(&first);
        assert!(
            used <= task.budget,
            "task {} spent {used} tokens of a {} budget",
            task.id,
            task.budget
        );

        results.push(Measured {
            id: task.id.clone(),
            language: task.language.clone(),
            budget: task.budget,
            used,
            expected: expected.len(),
            hits,
            recall: hits as f64 / expected.len() as f64,
            missing,
        });
    }
    results
}

/// Mean recall, in percent, over a selection of results.
fn average(results: &[&Measured]) -> Option<f64> {
    if results.is_empty() {
        return None;
    }
    let sum: f64 = results.iter().map(|r| r.recall).sum();
    Some(sum / results.len() as f64 * 100.0)
}

fn by_language<'a>(results: &'a [Measured], language: &str) -> Vec<&'a Measured> {
    results.iter().filter(|r| r.language == language).collect()
}

fn print_task_table(results: &[Measured]) {
    println!(
        "\n{:<7} {:<4} {:>7} {:>6} {:>9} {:>6} {:>8}",
        "task", "lang", "budget", "used", "expected", "hits", "recall"
    );
    println!("{}", "-".repeat(64));
    for r in results {
        println!(
            "{:<7} {:<4} {:>7} {:>6} {:>9} {:>6} {:>7.0}%",
            r.id,
            r.language,
            r.budget,
            r.used,
            r.expected,
            r.hits,
            r.recall * 100.0
        );
        if !r.missing.is_empty() {
            println!("        missed: {}", r.missing.join(", "));
        }
    }
}

fn print_language_split(results: &[Measured]) {
    println!("\nby language\n");
    println!("{:<6} {:>6} {:>9}", "lang", "tasks", "recall");
    println!("{}", "-".repeat(24));
    let mut languages: BTreeMap<&str, usize> = BTreeMap::new();
    for r in results {
        *languages.entry(r.language.as_str()).or_insert(0) += 1;
    }
    for (language, count) in &languages {
        let group = by_language(results, language);
        let avg = average(&group).unwrap_or(0.0);
        println!("{language:<6} {count:>6} {avg:>8.1}%");
    }
    if let Some(overall) = average(&results.iter().collect::<Vec<_>>()) {
        println!("{:<6} {:>6} {:>8.1}%", "all", results.len(), overall);
    }
}

fn print_sensitivity(rows: &[(String, f64, f64, f64)]) {
    println!("\ntier weight sensitivity\n");
    println!("{:<10} {:>9} {:>9} {:>9}", "weights", "overall", "en", "es");
    println!("{}", "-".repeat(40));
    for (label, overall, en, es) in rows {
        println!("{label:<10} {overall:>8.1}% {en:>8.1}% {es:>8.1}%");
    }
    let values: Vec<f64> = rows.iter().map(|(_, overall, _, _)| *overall).collect();
    let max = values.iter().cloned().fold(f64::MIN, f64::max);
    let min = values.iter().cloned().fold(f64::MAX, f64::min);
    let spread = max - min;
    println!(
        "\nspread across the three sets: {spread:.1} percentage points \
         (declared unstable above {UNSTABLE_SPREAD_PP:.0})"
    );
    println!(
        "{}\n",
        if spread > UNSTABLE_SPREAD_PP {
            "VERDICT: the weight choice is NOT validated by this set; it moves the result."
        } else {
            "VERDICT: the weight choice is stable on this set within the declared margin."
        }
    );
}

#[test]
#[ignore = "opt-in measurement: needs the pinned checkouts and prints tables"]
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

    println!("\ncontext_pack recall@budget over the labelled set");

    let default_run = measure(&gold, &root, None);
    assert!(
        !default_run.is_empty(),
        "no task was measured: are the checkouts present?"
    );
    print_task_table(&default_run);
    print_language_split(&default_run);

    // The weights are a judgement, so measure them rather than assert them. The
    // default run is reused instead of repeated.
    let mut rows = Vec::new();
    for variant in TIER_VARIANTS {
        let label = variant.unwrap_or("10/3/1").to_string();
        let results = if variant.is_none() {
            None
        } else {
            Some(measure(&gold, &root, variant))
        };
        let results = results.as_ref().unwrap_or(&default_run);
        assert_eq!(
            results.len(),
            default_run.len(),
            "every variant must measure every task"
        );
        let en = by_language(results, "en");
        let es = by_language(results, "es");
        rows.push((
            label,
            average(&results.iter().collect::<Vec<_>>()).unwrap_or(0.0),
            average(&en).unwrap_or(0.0),
            average(&es).unwrap_or(0.0),
        ));
    }
    print_sensitivity(&rows);
}
