//! Claude Code's PostToolUse output trim.
//!
//! When the Bash tool returns a long stdout, the model only needs the head and
//! the tail. The middle ends up in every prompt cache read for the rest of the
//! session, billed as `cache_read` on every turn, so the cost is many times the
//! size of the tool result. Cogito trims with an opt-in switch and never
//! silently drops lines: the marker tells the model what happened and how to
//! reach the rest (`git log -n`, `sed -n 'M,Np'`, `| tee`).
//!
//! Opt-in: behind `COGITO_OUTPUT_TRIM=1|true|yes|on` (default off). With the
//! switch off the hook prints nothing and exits 0, exactly as if it were not
//! registered. PostToolUse only fires after a successful tool call; failures
//! reach `PostToolUseFailure`, which this hook does not handle. We still
//! refuse to trim when `tool_response.interrupted` is true (timeouts, aborts)
//! or when the tail carries a test summary the user wants intact.
//!
//! A replacement uses `hookSpecificOutput.updatedToolOutput`, which Claude
//! Code accepts for any tool. The replacement keeps the Bash response shape
//! (stdout/stderr/interrupted/isImage) so the model sees the same content
//! type it would have without us.
//!
//! Every failure path lets the model see the tool's original output. A
//! hook that errors must never cost the agent a tool result.

use serde_json::{Value, json};
use std::io::Read;

/// Stdout length in chars that triggers trimming. 8000 chars ≈ 2000 tokens.
const THRESHOLD_CHARS: usize = 8000;
/// Lines kept from the head of stdout when trimming applies.
const HEAD_KEEP: usize = 60;
/// Lines kept from the tail of stdout when trimming applies.
const TAIL_KEEP: usize = 20;
/// When the last 200 lines look like a test run, that many lines are kept
/// from the tail verbatim — losing `test result: ok` is worse than extra cost.
const TEST_TAIL_KEEP: usize = 200;
/// How many lines from the tail we scan for the test-summary heuristic.
const TEST_TAIL_SCAN: usize = 200;
/// Substrings that flag the tail as a test summary to preserve.
const TEST_TAIL_MARKERS: &[&str] = &["test result", " passed", " failed", " FAILED"];

/// Env var that turns trimming on. Values: 1/true/yes/on (case-insensitive).
const ENV_FLAG: &str = "COGITO_OUTPUT_TRIM";

/// Has the user asked for trimming? Reads the env at call time so tests can
/// flip the switch freely.
fn is_enabled() -> bool {
    std::env::var_os(ENV_FLAG)
        .map(|v| {
            matches!(
                v.to_string_lossy().trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Build the cogito trim response for one PostToolUse event, or `None` to
/// mean "no replacement" so the model keeps the original tool result.
///
/// `enabled` is passed in (rather than read from env) so the caller controls
/// the switch from a single point and tests stay deterministic.
pub fn decide(event: &Value, enabled: bool) -> Option<Value> {
    if !enabled {
        return None;
    }
    let tool = event.get("tool_name").and_then(Value::as_str)?;
    if tool != "Bash" {
        return None;
    }
    let response = event.get("tool_response")?;
    let stdout = response.get("stdout").and_then(Value::as_str)?;
    if stdout.len() <= THRESHOLD_CHARS {
        return None;
    }
    if response
        .get("interrupted")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        // a timeout or abort left the user with partial output they may need
        // in full; leave it untouched
        return None;
    }

    let stderr = response.get("stderr").and_then(Value::as_str).unwrap_or("");
    let interrupted = response.get("interrupted").cloned().unwrap_or(json!(false));
    let is_image = response.get("isImage").cloned().unwrap_or(json!(false));

    let lines: Vec<&str> = stdout.split('\n').collect();
    let line_count = lines.len();
    let has_test_summary = tail_has_test_summary(&lines);

    let new_stdout = if has_test_summary {
        // keep the tail whole — losing the summary costs more than the
        // tokens, and the test runner typically writes the summary last
        let keep = TEST_TAIL_KEEP.min(line_count);
        let kept: Vec<&str> = lines[line_count - keep..].to_vec();
        let kept_text = kept.join("\n");
        let skipped = line_count.saturating_sub(keep);
        format!(
            "[cogito trimmed {skipped} lines; re-run with a narrower command, \
             e.g. git log -n, sed -n, or | tee to a file]\n\n\
             ...[test summary kept below]...\n\n{kept_text}",
        )
    } else {
        let head: Vec<&str> = lines.iter().take(HEAD_KEEP).copied().collect();
        let tail: Vec<&str> = if line_count > HEAD_KEEP + TAIL_KEEP {
            lines[line_count - TAIL_KEEP..].to_vec()
        } else {
            Vec::new()
        };
        let skipped = line_count.saturating_sub(HEAD_KEEP + TAIL_KEEP);
        if skipped == 0 {
            return None; // too short to trim usefully
        }
        let head_text = head.join("\n");
        let tail_text = tail.join("\n");
        format!(
            "[cogito trimmed {skipped} lines; re-run with a narrower command, \
             e.g. git log -n, sed -n, or | tee to a file]\n{head_text}\n\
             ... [{skipped} lines omitted] ...\n{tail_text}",
        )
    };

    Some(json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "updatedToolOutput": {
                "stdout": new_stdout,
                "stderr": stderr,
                "interrupted": interrupted,
                "isImage": is_image,
            }
        }
    }))
}

/// Does any of the last `TEST_TAIL_SCAN` lines (joined) contain a marker that
/// looks like a test-runner summary? Caller passes the pre-split lines so we
/// don't pay for two splits.
fn tail_has_test_summary(lines: &[&str]) -> bool {
    let start = lines.len().saturating_sub(TEST_TAIL_SCAN);
    let joined = lines[start..].join("\n");
    TEST_TAIL_MARKERS.iter().any(|m| joined.contains(m))
}

/// Read the hook input from stdin, decide, print the JSON to stdout, exit 0.
///
/// A hook that errors must never block the agent, so every failure path
/// exits 0 with no output. With no output Claude Code keeps the original
/// tool result, which is exactly what we want when something goes wrong.
pub async fn run() -> ! {
    let mut input = String::new();
    let enabled = is_enabled();
    let decision = match std::io::stdin().read_to_string(&mut input) {
        Ok(_) => match serde_json::from_str::<Value>(&input) {
            Ok(event) => decide(&event, enabled),
            Err(_) => None,
        },
        Err(_) => None,
    };
    if let Some(answer) = decision
        && let Ok(s) = serde_json::to_string(&answer)
    {
        println!("{s}");
    }
    std::process::exit(0)
}

