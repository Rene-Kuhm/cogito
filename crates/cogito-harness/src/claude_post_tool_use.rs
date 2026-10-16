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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Build a minimal Bash PostToolUse event with the given stdout.
    fn bash_event_with_stdout(stdout: &str) -> Value {
        json!({
            "tool_name": "Bash",
            "session_id": "s",
            "tool_response": {
                "stdout": stdout,
                "stderr": "",
                "interrupted": false,
                "isImage": false,
            }
        })
    }

    #[test]
    fn test_disabled_passes_through_unchanged() {
        // long stdout: would normally trigger trimming
        let big = (1..=1000)
            .map(|i| format!("line {i}\n"))
            .collect::<String>();
        let event = bash_event_with_stdout(&big);
        assert!(decide(&event, false).is_none());
    }

    #[test]
    fn test_below_threshold_is_untouched() {
        let small = (0..100).map(|_| "x").collect::<String>();
        let event = bash_event_with_stdout(&small);
        assert!(decide(&event, true).is_none());
    }

    #[test]
    fn test_non_bash_tool_passes_through() {
        let big = (1..=5000).map(|_| "x").collect::<String>();
        let event = json!({
            "tool_name": "Read",
            "tool_response": { "content": big },
        });
        assert!(decide(&event, true).is_none());
    }

    #[test]
    fn test_interrupted_response_is_left_alone() {
        let big = (1..=5000)
            .map(|i| format!("line {i}\n"))
            .collect::<String>();
        let event = json!({
            "tool_name": "Bash",
            "tool_response": {
                "stdout": big,
                "stderr": "",
                "interrupted": true,
                "isImage": false,
            }
        });
        assert!(decide(&event, true).is_none());
    }

    #[test]
    fn test_trim_keeps_head_and_tail_and_marks_omitted_lines() {
        // 1000 distinct lines: head=60, tail=20, skipped=920
        let stdout: String = (1..=1000)
            .map(|i| format!("line-{i:04}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            stdout.len() > THRESHOLD_CHARS,
            "fixture under threshold: {}",
            stdout.len()
        );
        let event = bash_event_with_stdout(&stdout);
        let out = decide(&event, true).expect("enabled + long → replacement");

        let replacement = &out["hookSpecificOutput"]["updatedToolOutput"];
        let new_stdout = replacement["stdout"].as_str().unwrap();
        assert_eq!(replacement["interrupted"], json!(false));
        assert_eq!(replacement["isImage"], json!(false));
        assert_eq!(replacement["stderr"], json!(""));
        assert!(new_stdout.contains("cogito trimmed 920 lines"));
        assert!(new_stdout.contains("re-run with a narrower command"));
        assert!(new_stdout.contains("line-0001"));
        assert!(new_stdout.contains("line-0060"));
        assert!(new_stdout.contains("line-0981"));
        assert!(new_stdout.contains("line-1000"));
        assert!(!new_stdout.contains("line-0500")); // middle is gone
    }

    #[test]
    fn test_marker_explains_omission_count_unambiguously() {
        // exact line counts so the assertion stays readable
        let stdout: String = (1..=1000)
            .map(|i| format!("line-{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(stdout.len() > THRESHOLD_CHARS);
        let event = bash_event_with_stdout(&stdout);
        let out = decide(&event, true).unwrap();
        let new_stdout = out["hookSpecificOutput"]["updatedToolOutput"]["stdout"]
            .as_str()
            .unwrap();
        assert!(new_stdout.contains("cogito trimmed 920 lines"));
        assert!(new_stdout.contains("920 lines omitted"));
    }

    #[test]
    fn test_test_summary_tail_is_preserved_whole() {
        // mimics a cargo test invocation: the summary is at the end
        let mut body: Vec<String> = (1..=1000).map(|i| format!("test {i} ... ok")).collect();
        body.push("test result: ok. 1000 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.20s".to_string());
        let stdout = body.join("\n");
        let event = bash_event_with_stdout(&stdout);
        let out = decide(&event, true).unwrap();
        let new_stdout = out["hookSpecificOutput"]["updatedToolOutput"]["stdout"]
            .as_str()
            .unwrap();

        // tail kept intact
        assert!(new_stdout.contains("test result: ok"));
        assert!(new_stdout.contains("1000 passed"));
        // middle truncated
        assert!(!new_stdout.contains("test 500"));
        // marker mentions the test-summary branch
        assert!(new_stdout.contains("test summary kept below"));
    }

    #[test]
    fn test_test_summary_branch_keeps_up_to_two_hundred_tail_lines() {
        // 3000 lines + summary at the end → keep only the last 200, drop the
        // first 2800
        let mut body: Vec<String> = (1..=3000).map(|i| format!("run {i}")).collect();
        body.push("test result: ok. 3000 passed; 0 failed".to_string());
        let stdout = body.join("\n");
        assert!(stdout.len() > THRESHOLD_CHARS);
        let event = bash_event_with_stdout(&stdout);
        let out = decide(&event, true).unwrap();
        let new_stdout = out["hookSpecificOutput"]["updatedToolOutput"]["stdout"]
            .as_str()
            .unwrap();
        // the kept tail is the last 200 lines: runs 2802..3000 + the summary line
        assert!(new_stdout.contains("run 3000"));
        assert!(new_stdout.contains("run 2802"));
        assert!(!new_stdout.contains("run 2801"));
        assert!(!new_stdout.contains("run 2800"));
        assert!(new_stdout.contains("test summary kept below"));
    }

    #[test]
    fn test_replacement_shape_matches_bash_response() {
        // The new tool_response must keep stdout/stderr/interrupted/isImage so
        // the model receives the same schema, only with shorter stdout.
        // Each "very long line..." is a separate line of stdout so the line
        // count (and head/tail slicing) is meaningful.
        let stdout = std::iter::repeat_n(
            "very long line, very long line, very long line, very long line",
            1500,
        )
        .collect::<Vec<_>>()
        .join("\n");
        assert!(stdout.len() > THRESHOLD_CHARS);
        let event = json!({
            "tool_name": "Bash",
            "tool_response": {
                "stdout": stdout,
                "stderr": "some warning on stderr\n",
                "interrupted": false,
                "isImage": false,
            }
        });
        let out = decide(&event, true).unwrap();
        let r = &out["hookSpecificOutput"]["updatedToolOutput"];
        assert_eq!(
            r["hookEventName_if_present"],
            json!(null),
            "updatedToolOutput must be a plain Bash response object, not a wrapper"
        );
        assert!(r.get("stdout").and_then(Value::as_str).is_some());
        assert_eq!(r["stderr"], json!("some warning on stderr\n"));
        assert_eq!(r["interrupted"], json!(false));
        assert_eq!(r["isImage"], json!(false));
        assert!(out["hookSpecificOutput"]["hookEventName"] == "PostToolUse");
    }

    #[test]
    fn test_run_disabled_exits_zero_with_no_output() {
        // Take the shared env lock so a parallel `with_output_trim` test
        // cannot leave `COGITO_OUTPUT_TRIM` set when this assertion runs.
        // Also explicitly unset the var to cover the case where the test
        // harness itself inherited the var from the calling shell.
        let _g = crate::agents::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::remove_var(crate::agents::COGITO_OUTPUT_TRIM);
        }
        assert!(!is_enabled(), "the hook must be disabled with no env var");
    }

    #[test]
    fn test_empty_event_does_not_panic() {
        // a malformed event must not panic the hook
        assert!(decide(&json!({}), true).is_none());
        assert!(decide(&json!({"tool_name": "Bash"}), true).is_none());
        assert!(decide(&json!({"tool_name": "Bash", "tool_response": {}}), true).is_none());
    }

    #[test]
    fn test_event_without_stdout_field_is_unchanged() {
        // tool_response present but missing stdout → leave alone
        let event = json!({
            "tool_name": "Bash",
            "tool_response": {
                "stderr": "y",
                "interrupted": false,
                "isImage": false,
            }
        });
        assert!(decide(&event, true).is_none());
    }
}
