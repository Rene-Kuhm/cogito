//! Claude Code's PreToolUse guardrail.
//!
//! Claude Code runs `cogito hook claude-pre-tool-use` before `Read` and `Grep`,
//! with the call as JSON on stdin. The first time Claude reads a large file
//! whole, or greps for a plain name, the call is blocked (exit 2) and the
//! message on stderr, which Claude Code hands to the model, carries Cogito's
//! outline or search result instead. Repeating that call goes through: the
//! agent is shown the cheap answer first, never stuck.
//!
//! A whole-file read that was already delivered in full is not delivered again
//! while the file is unchanged. The model gets the outline and the reason
//! instead, up to `MAX_UNCHANGED_BLOCKS` times, and then the read goes through.
//! A file that changed since it was delivered is read in full again, because
//! the text the model holds is no longer true.
//!
//! Every failure path allows the call. A guardrail that errors must not cost
//! the user a tool call.

use cogito_harness_core::{Cogito, index, map};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Files shorter than this are read whole without comment.
const OUTLINE_MIN_LINES: usize = 250;
/// A `Read` with a limit above this still counts as reading the whole file.
const WHOLE_READ_LIMIT: u64 = 400;
/// How many times an unchanged, already delivered file is turned away before a
/// full read goes through. The model is never stuck.
const MAX_UNCHANGED_BLOCKS: u32 = 2;

/// Read the hook input from stdin, decide, and exit: 0 to allow, 2 to block.
pub async fn run() -> ! {
    let mut input = String::new();
    let decision = match std::io::stdin().read_to_string(&mut input) {
        Ok(_) => match serde_json::from_str::<Value>(&input) {
            Ok(event) => decide(&event).await.ok().flatten(),
            Err(_) => None,
        },
        Err(_) => None,
    };
    match decision {
        Some(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
        None => std::process::exit(0),
    }
}

/// The message to block with, or `None` to let the call run.
pub async fn decide(event: &Value) -> anyhow::Result<Option<String>> {
    let Some(root) = project_root(event) else {
        return Ok(None);
    };
    let tool = event["tool_name"].as_str().unwrap_or_default();
    let input = &event["tool_input"];
    let session = event["session_id"].as_str().unwrap_or("default");

    if tool == "Read" {
        return read_guard(&root, session, input).await;
    }
    if tool == "Bash" {
        return bash_read_guard(&root, session, input).await;
    }
    let (key, message) = match tool {
        "Grep" => match grep_nudge(&root, input).await? {
            Some(found) => found,
            None => return Ok(None),
        },
        _ => return Ok(None),
    };

    // Once per call shape and session: a repeat means Claude needs it.
    if !first_time(&root, session, &key)? {
        return Ok(None);
    }
    Ok(Some(message))
}

/// The agent-neutral entry point behind `cogito guard`. Reads one call from
/// stdin as `{"tool": "read" | "grep", "input": {...}, "cwd": "...", "session": "..."}`
/// and prints `{"block": bool, "reason": "..."}`. It always exits 0: a guard
/// that fails must never cost the agent a tool call.
pub async fn run_guard() -> ! {
    let mut input = String::new();
    let reason = match std::io::stdin().read_to_string(&mut input) {
        Ok(_) => match serde_json::from_str::<Value>(&input) {
            Ok(call) => guard_decision(&call).await.ok().flatten(),
            Err(_) => None,
        },
        Err(_) => None,
    };
    let answer = match reason {
        Some(reason) => serde_json::json!({ "block": true, "reason": reason }),
        None => serde_json::json!({ "block": false }),
    };
    println!("{answer}");
    std::process::exit(0)
}

/// Map an agent-neutral call onto the Claude Code event `decide` reads.
async fn guard_decision(call: &Value) -> anyhow::Result<Option<String>> {
    let cwd = call["cwd"].as_str().unwrap_or_default();
    let session = call["session"].as_str().unwrap_or("default");
    let input = &call["input"];
    let event = match call["tool"].as_str().unwrap_or_default() {
        "read" => {
            let Some(path) = input["path"].as_str().or(input["file_path"].as_str()) else {
                return Ok(None);
            };
            // Relative paths are relative to the project, as the agent meant them.
            let file = Path::new(cwd).join(path);
            serde_json::json!({
                "tool_name": "Read",
                "session_id": session,
                "cwd": cwd,
                "tool_input": {
                    "file_path": file.display().to_string(),
                    "offset": input["offset"],
                    "limit": input["limit"],
                },
            })
        }
        "grep" => serde_json::json!({
            "tool_name": "Grep",
            "session_id": session,
            "cwd": cwd,
            "tool_input": input,
        }),
        "bash" => serde_json::json!({
            "tool_name": "Bash",
            "session_id": session,
            "cwd": cwd,
            "tool_input": input,
        }),
        _ => return Ok(None),
    };
    decide(&event).await
}

/// The project the session runs in, if it looks like one.
fn project_root(event: &Value) -> Option<PathBuf> {
    let dir = std::env::var_os("CLAUDE_PROJECT_DIR")
        .map(PathBuf::from)
        .or_else(|| event["cwd"].as_str().map(PathBuf::from))?;
    (dir.join(".git").exists() || dir.join(".cogito").is_dir()).then_some(dir)
}

/// What a whole-file `Read` gets: the message to block with, or `None` to let it run.
async fn read_guard(root: &Path, session: &str, input: &Value) -> anyhow::Result<Option<String>> {
    let whole = input["offset"].is_null()
        && input["limit"]
            .as_u64()
            .is_none_or(|limit| limit > WHOLE_READ_LIMIT);
    let Some(path) = input["file_path"].as_str() else {
        return Ok(None);
    };
    if !whole {
        return Ok(None);
    }
    whole_file_read_guard(root, session, Path::new(path)).await
}

/// What a whole-file Bash read gets: parse the command, apply the same rule as
/// `Read`. Anything we are not sure about (pipes, redirects, multiple commands,
/// unknown commands, etc.) returns `Ok(None)` so the call goes through.
async fn bash_read_guard(
    root: &Path,
    session: &str,
    input: &Value,
) -> anyhow::Result<Option<String>> {
    let Some(command) = input["command"].as_str() else {
        return Ok(None);
    };
    let Some(path) = parse_bash_read(command) else {
        return Ok(None);
    };
    whole_file_read_guard(root, session, &path).await
}

/// The rule that backs both `Read` and the `Bash` whole-file read detector.
/// Given an absolute path, decide whether to show the outline, the unchanged
/// message, or let the read through. Anything outside the project, unreadable,
/// too small, or without declarations passes.
async fn whole_file_read_guard(
    root: &Path,
    session: &str,
    file: &Path,
) -> anyhow::Result<Option<String>> {
    let file = if file.is_absolute() {
        file.to_path_buf()
    } else {
        root.join(file)
    };
    let Ok(rel) = file.strip_prefix(root).map(Path::to_path_buf) else {
        return Ok(None); // outside the project
    };
    let Ok(content) = std::fs::read_to_string(&file) else {
        return Ok(None);
    };
    let lines = content.lines().count();
    let ext = rel.extension().and_then(|e| e.to_str()).unwrap_or("");
    if lines < OUTLINE_MIN_LINES || map::outline::declarations(&content, ext).is_empty() {
        return Ok(None);
    }

    let key = rel.display().to_string();
    let hash = fnv1a(content.as_bytes());
    let mut state = ReadState::load(root, session);
    let verdict = decide_read(state.0.entry(key).or_default(), hash);
    state.save(root, session)?;

    match verdict {
        ReadVerdict::Allow => Ok(None),
        ReadVerdict::Outline => {
            let outline = outline_for(root, &rel).await?;
            Ok(Some(format!(
                "{} has {lines} lines. Cogito's outline instead of the whole file:\n\n{}\n\
                 Read the range you need with offset/limit. Reading the whole file again is allowed \
                 if you really need it.",
                rel.display(),
                outline.trim_end()
            )))
        }
        ReadVerdict::Unchanged => {
            let outline = outline_for(root, &rel).await?;
            Ok(Some(format!(
                "{} has not changed since its full text was read in this session ({lines} lines). \
                 Cogito's outline instead:\n\n{}\n\
                 Read the range you need with offset/limit. A repeated full read is blocked \
                 {MAX_UNCHANGED_BLOCKS} times, then allowed.",
                rel.display(),
                outline.trim_end()
            )))
        }
    }
}

/// Detect a single, simple Bash command that reads a file whole. Returns the
/// file path when the command is `cat FILE`, `nl FILE`, `sed -n '1,$p' FILE`,
/// `head -n N FILE` with N larger than `WHOLE_READ_LIMIT`, or the symmetric
/// `tail`. Anything we are not sure about (pipes, redirects, multiple commands,
/// a small `-n`, a ranged `sed`) returns `None` so the call goes through.
fn parse_bash_read(command: &str) -> Option<PathBuf> {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Reject anything that can change which bytes the agent sees: pipes,
    // redirects, command substitution, chaining, newlines, escapes. Bare `$`
    // (sed range end, positional args) is allowed; only the substitution
    // forms `$(` and `${` are rejected. `str::contains(&[char])` matches the
    // chars as a sequence, not a class, so check substring patterns instead.
    const UNSAFE_PATTERNS: &[&str] = &["|", ">", "<", ";", "&", "`", "$(", "${", "\\", "\n", "\t"];
    if UNSAFE_PATTERNS.iter().any(|p| trimmed.contains(p)) {
        return None;
    }

    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    match tokens.first().copied()? {
        "cat" => parse_simple_one_arg(&tokens, "cat"),
        "nl" => parse_simple_one_arg(&tokens, "nl"),
        "head" => parse_head_tail(&tokens, false),
        "tail" => parse_head_tail(&tokens, true),
        "sed" => parse_sed(&tokens),
        _ => None,
    }
}

/// `cat FILE` or `nl FILE`: a single positional argument that is a path. Any
/// other shape (multiple files, flags) does not match, so the call passes.
fn parse_simple_one_arg(tokens: &[&str], cmd: &str) -> Option<PathBuf> {
    if tokens.len() != 2 {
        return None;
    }
    // `cat` and `nl` accept no flags relevant to whole-file reading; an
    // explicit flag means the caller wanted something we are not modelling.
    if tokens[1].starts_with('-') {
        return None;
    }
    if cmd == "nl" && tokens[1] == "--" {
        return None;
    }
    Some(PathBuf::from(tokens[1]))
}

/// `head -n N FILE` (or `tail`): an explicit `-n` whose `N` is a small
/// positive integer is always a range and passes; `N` larger than
/// `WHOLE_READ_LIMIT` is treated as a whole-file read. No `-n` at all means
/// the default of 10 lines, which is a small range. Anything else passes.
fn parse_head_tail(tokens: &[&str], _is_tail: bool) -> Option<PathBuf> {
    let mut i = 1;
    let mut has_n_flag = false;
    while i < tokens.len() {
        let t = tokens[i];
        if t == "--" {
            i += 1;
            break;
        }
        if t == "-n" {
            let n = tokens.get(i + 1)?.parse::<u64>().ok()?;
            if n <= WHOLE_READ_LIMIT {
                return None;
            }
            has_n_flag = true;
            i += 2;
            continue;
        }
        if let Some(rest) = t.strip_prefix("-n") {
            let n = rest.parse::<u64>().ok()?;
            if n <= WHOLE_READ_LIMIT {
                return None;
            }
            has_n_flag = true;
            i += 1;
            continue;
        }
        if t.starts_with('-') {
            // Any other flag (`-c`, `--bytes`, `-q`, ...) is something we are
            // not modelling; let the call run.
            return None;
        }
        // First non-flag token is the file. Anything else means multiple paths
        // or shell substitution we did not catch: pass.
        let path = PathBuf::from(t);
        if i + 1 != tokens.len() {
            return None;
        }
        if !has_n_flag {
            // No `-n` at all: `head FILE` / `tail FILE` default to 10 lines,
            // which is a small range. Pass.
            return None;
        }
        return Some(path);
    }
    if i == tokens.len() && has_n_flag {
        // `head -n 5000` with no path: pass through.
        return None;
    }
    None
}

/// `sed -n '1,$p' FILE` (or `1,$p` without quotes). The `1,$` range means
/// "from the first to the last line", i.e. the whole file. Any other range
/// (start after 1, end before `$`) passes. No `-n` means sed prints every
/// line by default; we still only guard the explicit whole-file shape.
fn parse_sed(tokens: &[&str]) -> Option<PathBuf> {
    if tokens.get(1) != Some(&"-n") {
        return None;
    }
    let range = tokens.get(2)?;
    let stripped = range.trim_matches(|c| c == '\'' || c == '"').trim();
    // `1,$p` or `1,$ p` (sed accepts the space). Strip trailing whitespace and
    // an optional trailing `p` from a print command.
    let range = stripped.split_whitespace().next()?.trim_end_matches('p');
    let mut parts = range.split(',');
    let start = parts.next()?;
    let end = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    if start != "1" || end != "$" {
        return None;
    }
    let path = tokens.get(3)?;
    if tokens.get(4).is_some() {
        return None;
    }
    Some(PathBuf::from(*path))
}

/// The declaration outline of one project file, from the index when it has one.
async fn outline_for(root: &Path, rel: &Path) -> anyhow::Result<String> {
    let (cogito, project) = open(root)?;
    let (fingerprint, _) = cogito.fingerprint(&project, false).await?;

    // Try the index first.
    let decls = if let Some(conn) = cogito.connection() {
        index::outline(&conn, project.id, rel)
            .map(|ds| ds.into_iter().map(Into::into).collect::<Vec<_>>())
            .unwrap_or_default()
    } else {
        vec![]
    };

    let decls = if decls.is_empty() {
        let content = std::fs::read_to_string(root.join(rel)).unwrap_or_default();
        let ext = rel.extension().and_then(|e| e.to_str()).unwrap_or("");
        map::outline::declarations(&content, ext)
    } else {
        decls
    };

    Ok(map::render_outline(
        root,
        Some(&fingerprint),
        rel,
        &decls,
        0,
    ))
}

/// What the guard answers for one whole-file read.
#[derive(Debug, PartialEq)]
enum ReadVerdict {
    /// First sight of this content: show the outline, deliver nothing yet.
    Outline,
    /// Already delivered unchanged: show the outline and the reason.
    Unchanged,
    /// Let the full read through.
    Allow,
}

/// Per-file state for one session. `outline` is the hash of the content whose
/// outline was shown, `delivered` the hash of the content given in full, and
/// `blocks` how often an unchanged, delivered file was turned away since.
#[derive(Debug, Default, Clone, PartialEq)]
struct ReadEntry {
    outline: Option<u64>,
    delivered: Option<u64>,
    blocks: u32,
}

/// Decide one whole-file read against the entry for that file.
fn decide_read(entry: &mut ReadEntry, hash: u64) -> ReadVerdict {
    if entry.delivered == Some(hash) {
        if entry.blocks >= MAX_UNCHANGED_BLOCKS {
            return ReadVerdict::Allow;
        }
        entry.blocks += 1;
        return ReadVerdict::Unchanged;
    }
    // Changed since it was delivered: the old text is stale, so read it in full.
    // The same holds when the outline was already shown for this content.
    if entry.delivered.is_some() || entry.outline == Some(hash) {
        entry.delivered = Some(hash);
        entry.blocks = 0;
        return ReadVerdict::Allow;
    }
    entry.outline = Some(hash);
    ReadVerdict::Outline
}

/// The read state of one session, one line per file: `path`, outline hash,
/// delivered hash, blocks. A missing hash is written as `-`.
#[derive(Debug, Default)]
struct ReadState(BTreeMap<String, ReadEntry>);

impl ReadState {
    fn load(root: &Path, session: &str) -> Self {
        let text =
            std::fs::read_to_string(session_file(root, session, "reads")).unwrap_or_default();
        let entries = text
            .lines()
            .filter_map(|line| {
                let mut parts = line.split('\t');
                let path = parts.next()?;
                // A hash is hex, or `-` when there is none.
                let hash = |s: Option<&str>| {
                    s.filter(|hex| *hex != "-")
                        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
                };
                let outline = hash(parts.next());
                let delivered = hash(parts.next());
                let blocks = parts.next()?.parse().ok()?;
                Some((
                    path.to_string(),
                    ReadEntry {
                        outline,
                        delivered,
                        blocks,
                    },
                ))
            })
            .collect();
        Self(entries)
    }

    fn save(&self, root: &Path, session: &str) -> anyhow::Result<()> {
        let fmt = |h: Option<u64>| h.map_or("-".to_string(), |h| format!("{h:016x}"));
        let text: String = self
            .0
            .iter()
            .map(|(path, e)| {
                format!(
                    "{path}\t{}\t{}\t{}\n",
                    fmt(e.outline),
                    fmt(e.delivered),
                    e.blocks
                )
            })
            .collect();
        let file = session_file(root, session, "reads");
        std::fs::create_dir_all(file.parent().unwrap_or(root))?;
        std::fs::write(file, text)?;
        Ok(())
    }
}

/// 64-bit FNV-1a: stable across runs, so a stored hash still means the same bytes.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

async fn grep_nudge(root: &Path, input: &Value) -> anyhow::Result<Option<(String, String)>> {
    let Some(pattern) = input["pattern"].as_str() else {
        return Ok(None);
    };
    if !is_plain_name(pattern) {
        return Ok(None);
    }
    let (cogito, project) = open(root)?;

    // Sync the index: on failure, let the call through.
    let _ = cogito.sync_index(&project).await;

    // Query the index: on failure, let the call through.
    let hits = if let Some(conn) = cogito.connection() {
        index::search(&conn, project.id, pattern, 12).unwrap_or_default()
    } else {
        return Ok(None);
    };

    if hits.is_empty() {
        return Ok(None);
    }

    let stored = cogito.stored_fingerprint(project.id).unwrap_or(None);
    let modules: Vec<_> = stored
        .as_ref()
        .map(|fp| fp.modules.clone())
        .unwrap_or_default();

    // A file named exactly like the query puts the Files section first. The rule
    // lives in the index, so this and the other two callers cannot drift apart.
    let files_first = hits
        .iter()
        .any(|h| !h.is_symbol && index::is_named_file_match(&h.path.to_string_lossy(), pattern));

    let text = map::render_search(&hits, &modules, pattern, files_first);
    let key = format!(
        "grep:{pattern}:{}:{}",
        input["path"].as_str().unwrap_or_default(),
        input["glob"].as_str().unwrap_or_default()
    );
    let message = format!(
        "Cogito's index for \"{pattern}\" (symbols and doc sections with file:line, files, \
         modules):\n\n{}\n\
         Use file_outline on a result and read only its range. If you need every textual \
         match, repeat the same Grep call and it will run.",
        text.trim_end()
    );
    Ok(Some((key, message)))
}

/// A pattern that names something rather than a regex over content.
fn is_plain_name(pattern: &str) -> bool {
    let pattern = pattern.trim();
    (2..=80).contains(&pattern.len())
        && pattern
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '-' | '/' | ' '))
}

fn open(root: &Path) -> anyhow::Result<(Cogito, cogito_harness_core::Project)> {
    let cogito = Cogito::for_project(root)?;
    let project = cogito.load_project(root.to_path_buf())?;
    Ok((cogito, project))
}

/// The state file for one session under `.cogito/nudges/`.
fn session_file(root: &Path, session: &str, extension: &str) -> PathBuf {
    let safe: String = session
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(64)
        .collect();
    let name = if safe.is_empty() { "default" } else { &safe };
    root.join(".cogito/nudges")
        .join(format!("{name}.{extension}"))
}

/// Record `key` for this session; `false` if it was already recorded.
fn first_time(root: &Path, session: &str, key: &str) -> anyhow::Result<bool> {
    let file = session_file(root, session, "seen");
    std::fs::create_dir_all(file.parent().unwrap_or(root))?;
    let seen = std::fs::read_to_string(&file).unwrap_or_default();
    if seen.lines().any(|line| line == key) {
        return Ok(false);
    }
    use std::io::Write;
    let mut out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file)?;
    writeln!(out, "{key}")?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_names() {
        assert!(is_plain_name("README"));
        assert!(is_plain_name("executeToolCall"));
        assert!(is_plain_name("auth/server.ts"));
        assert!(!is_plain_name("fn\\s+main"));
        assert!(!is_plain_name("a|b"));
        assert!(!is_plain_name("x"));
    }

    #[test]
    fn test_nudge_fires_once_per_session_and_key() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(first_time(dir.path(), "s1", "read:a.ts").unwrap());
        assert!(!first_time(dir.path(), "s1", "read:a.ts").unwrap());
        assert!(first_time(dir.path(), "s1", "read:b.ts").unwrap());
        assert!(first_time(dir.path(), "s2", "read:a.ts").unwrap());
    }

    #[test]
    fn test_first_read_shows_outline_then_the_full_text_then_turns_repeats_away() {
        let mut entry = ReadEntry::default();
        let h = fnv1a(b"v1");
        assert_eq!(decide_read(&mut entry, h), ReadVerdict::Outline);
        assert_eq!(decide_read(&mut entry, h), ReadVerdict::Allow);
        assert_eq!(decide_read(&mut entry, h), ReadVerdict::Unchanged);
    }

    #[test]
    fn test_unchanged_delivered_file_is_turned_away_twice_then_allowed() {
        let mut entry = ReadEntry::default();
        let h = fnv1a(b"v1");
        decide_read(&mut entry, h); // outline
        assert_eq!(decide_read(&mut entry, h), ReadVerdict::Allow); // full text delivered
        assert_eq!(decide_read(&mut entry, h), ReadVerdict::Unchanged);
        assert_eq!(decide_read(&mut entry, h), ReadVerdict::Unchanged);
        assert_eq!(decide_read(&mut entry, h), ReadVerdict::Allow);
    }

    #[test]
    fn test_changed_file_is_read_in_full_again() {
        let mut entry = ReadEntry::default();
        let v1 = fnv1a(b"v1");
        let v2 = fnv1a(b"v2");
        decide_read(&mut entry, v1);
        decide_read(&mut entry, v1); // delivered v1
        assert_eq!(decide_read(&mut entry, v2), ReadVerdict::Allow);
        assert_eq!(decide_read(&mut entry, v2), ReadVerdict::Unchanged);
    }

    #[test]
    fn test_read_state_round_trips_through_the_session_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut state = ReadState::default();
        state.0.insert(
            "src/a.rs".into(),
            ReadEntry {
                outline: Some(fnv1a(b"x")),
                delivered: None,
                blocks: 0,
            },
        );
        state.0.insert(
            "src/b.rs".into(),
            ReadEntry {
                outline: Some(fnv1a(b"y")),
                delivered: Some(fnv1a(b"y")),
                blocks: 2,
            },
        );
        state.save(dir.path(), "s1").unwrap();
        let loaded = ReadState::load(dir.path(), "s1");
        assert_eq!(loaded.0, state.0);
        assert!(ReadState::load(dir.path(), "other").0.is_empty());
    }

    #[tokio::test]
    async fn test_guard_resolves_relative_paths_and_turns_repeats_away() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(root)
                    .env("GIT_AUTHOR_NAME", "t")
                    .env("GIT_AUTHOR_EMAIL", "t@t")
                    .env("GIT_COMMITTER_NAME", "t")
                    .env("GIT_COMMITTER_EMAIL", "t@t")
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init", "-q"]);
        let body: String = (0..300)
            .map(|i| format!("export function fn{i}(x: number) {{ return x + {i}; }}\n"))
            .collect();
        std::fs::write(root.join("big.ts"), body).unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);

        let call = |tool: &str, input: Value| {
            serde_json::json!({
                "tool": tool,
                "session": "guard-test",
                "cwd": root.display().to_string(),
                "input": input,
            })
        };
        let read = call("read", serde_json::json!({ "path": "big.ts" }));
        assert!(
            guard_decision(&read).await.unwrap().is_some(),
            "outline first"
        );
        assert!(
            guard_decision(&read).await.unwrap().is_none(),
            "full text delivered"
        );
        assert!(
            guard_decision(&read).await.unwrap().is_some(),
            "unchanged repeat"
        );

        let ranged = call(
            "read",
            serde_json::json!({ "path": "big.ts", "offset": 10, "limit": 20 }),
        );
        assert!(
            guard_decision(&ranged).await.unwrap().is_none(),
            "ranges pass"
        );
        let other = call("bash", serde_json::json!({ "command": "ls" }));
        assert!(
            guard_decision(&other).await.unwrap().is_none(),
            "other tools pass"
        );
    }

    #[test]
    fn test_parse_bash_read_detects_one_command_per_reader() {
        assert_eq!(parse_bash_read("cat big.ts"), Some(PathBuf::from("big.ts")));
        assert_eq!(parse_bash_read("nl big.ts"), Some(PathBuf::from("big.ts")));
        assert_eq!(
            parse_bash_read("head -n 500 big.ts"),
            Some(PathBuf::from("big.ts"))
        );
        assert_eq!(
            parse_bash_read("tail -n500 big.ts"),
            Some(PathBuf::from("big.ts"))
        );
        assert_eq!(
            parse_bash_read("sed -n '1,$p' big.ts"),
            Some(PathBuf::from("big.ts"))
        );
        assert_eq!(
            parse_bash_read("sed -n 1,$p big.ts"),
            Some(PathBuf::from("big.ts"))
        );
    }

    #[test]
    fn test_parse_bash_read_lets_explicit_ranges_pass() {
        // Head, tail with a small -n is a range; sed with anything other than
        // 1,$ is a range.
        assert_eq!(parse_bash_read("head -n 20 big.ts"), None);
        assert_eq!(parse_bash_read("head -n20 big.ts"), None);
        assert_eq!(parse_bash_read("tail -n 20 big.ts"), None);
        assert_eq!(parse_bash_read("sed -n '10,30p' big.ts"), None);
        assert_eq!(parse_bash_read("sed -n '2,$p' big.ts"), None);
        assert_eq!(parse_bash_read("sed -n '1,30p' big.ts"), None);
    }

    #[test]
    fn test_parse_bash_read_lets_non_read_commands_pass() {
        assert_eq!(parse_bash_read("ls"), None);
        assert_eq!(parse_bash_read("git status"), None);
        assert_eq!(parse_bash_read("grep -r foo"), None);
        assert_eq!(parse_bash_read("cargo test"), None);
        assert_eq!(parse_bash_read("echo hello"), None);
    }

    #[test]
    fn test_parse_bash_read_rejects_anything_that_can_change_the_bytes() {
        // Pipes, redirects, command substitution, chaining, newlines.
        assert_eq!(parse_bash_read("cat big.ts | head"), None);
        assert_eq!(parse_bash_read("cat big.ts > out.txt"), None);
        assert_eq!(parse_bash_read("cat big.ts < big.ts"), None);
        assert_eq!(parse_bash_read("cat $(echo big.ts)"), None);
        assert_eq!(parse_bash_read("cat `echo big.ts`"), None);
        assert_eq!(parse_bash_read("rm big.ts; cat big.ts"), None);
        assert_eq!(parse_bash_read("cat big.ts && echo done"), None);
        assert_eq!(parse_bash_read("cat big.ts\nhead big.ts"), None);
        assert_eq!(parse_bash_read("cat \\big.ts"), None);
    }

    #[test]
    fn test_parse_bash_read_rejects_extra_arguments_and_flags() {
        // Two files, or a flag we do not model, or a -n without a number.
        assert_eq!(parse_bash_read("cat a.ts b.ts"), None);
        assert_eq!(parse_bash_read("nl a.ts b.ts"), None);
        assert_eq!(parse_bash_read("cat -n big.ts"), None);
        assert_eq!(parse_bash_read("head -c 100 big.ts"), None);
        assert_eq!(parse_bash_read("head -n big.ts"), None);
        assert_eq!(parse_bash_read("sed -n '1,$p' big.ts extra"), None);
        assert_eq!(parse_bash_read("sed -n '10,30p' big.ts"), None);
    }

    #[test]
    fn test_parse_bash_read_keeps_quoted_paths() {
        // Quotes are stripped only on the sed range; the path itself stays as
        // written. The downstream code joins it with the project root.
        assert_eq!(
            parse_bash_read("cat 'src/big.ts'"),
            Some(PathBuf::from("'src/big.ts'"))
        );
    }

    #[tokio::test]
    async fn test_bash_read_guard_uses_the_same_outline_state_as_read() {
        // A bash `cat` and a Read on the same file share the read state: the
        // first call shows the outline, the second goes through, the third is
        // the unchanged message. This is the rule the task asks for, exercised
        // through Bash.
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(root)
                    .env("GIT_AUTHOR_NAME", "t")
                    .env("GIT_AUTHOR_EMAIL", "t@t")
                    .env("GIT_COMMITTER_NAME", "t")
                    .env("GIT_COMMITTER_EMAIL", "t@t")
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init", "-q"]);
        let body: String = (0..300)
            .map(|i| format!("export function fn{i}(x: number) {{ return x + {i}; }}\n"))
            .collect();
        std::fs::write(root.join("big.ts"), body).unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);

        let call = |tool: &str, input: Value| {
            serde_json::json!({
                "tool": tool,
                "session": "guard-bash",
                "cwd": root.display().to_string(),
                "input": input,
            })
        };

        let bash_cat = call("bash", serde_json::json!({ "command": "cat big.ts" }));
        assert!(
            guard_decision(&bash_cat).await.unwrap().is_some(),
            "cat triggers outline first"
        );
        assert!(
            guard_decision(&bash_cat).await.unwrap().is_none(),
            "second cat goes through"
        );
        assert!(
            guard_decision(&bash_cat).await.unwrap().is_some(),
            "third cat shows unchanged"
        );

        let bash_head = call(
            "bash",
            serde_json::json!({ "command": "head -n 500 big.ts" }),
        );
        assert!(
            guard_decision(&bash_head).await.unwrap().is_some(),
            "head -n 500 sees the same unchanged message (state shared)"
        );

        let bash_nl = call("bash", serde_json::json!({ "command": "nl big.ts" }));
        assert!(
            guard_decision(&bash_nl).await.unwrap().is_none(),
            "after MAX_UNCHANGED_BLOCKS the read goes through"
        );

        let bash_ranged = call(
            "bash",
            serde_json::json!({ "command": "head -n 20 big.ts" }),
        );
        assert!(
            guard_decision(&bash_ranged).await.unwrap().is_none(),
            "explicit small -n passes"
        );
        let bash_sed_ranged = call(
            "bash",
            serde_json::json!({ "command": "sed -n '10,30p' big.ts" }),
        );
        assert!(
            guard_decision(&bash_sed_ranged).await.unwrap().is_none(),
            "explicit sed range passes"
        );
        let bash_ls = call("bash", serde_json::json!({ "command": "ls" }));
        assert!(
            guard_decision(&bash_ls).await.unwrap().is_none(),
            "non-read commands pass"
        );
    }

    #[tokio::test]
    async fn test_bash_read_guard_lets_small_and_out_of_project_files_pass() {
        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            assert!(
                std::process::Command::new("git")
                    .args(args)
                    .current_dir(root)
                    .env("GIT_AUTHOR_NAME", "t")
                    .env("GIT_AUTHOR_EMAIL", "t@t")
                    .env("GIT_COMMITTER_NAME", "t")
                    .env("GIT_COMMITTER_EMAIL", "t@t")
                    .status()
                    .unwrap()
                    .success()
            );
        };
        git(&["init", "-q"]);
        std::fs::write(root.join("small.ts"), "export const x = 1;\n").unwrap();
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "init"]);

        let call = |command: &str| {
            serde_json::json!({
                "tool": "bash",
                "session": "guard-bash-small",
                "cwd": root.display().to_string(),
                "input": { "command": command },
            })
        };
        assert!(
            guard_decision(&call("cat small.ts"))
                .await
                .unwrap()
                .is_none(),
            "short file passes"
        );

        let outside = "/tmp/cogito-not-in-project.ts";
        std::fs::write(outside, "export const x = 1;\n").unwrap();
        assert!(
            guard_decision(&call(&format!("cat {outside}")))
                .await
                .unwrap()
                .is_none(),
            "out-of-project file passes"
        );
        let _ = std::fs::remove_file(outside);
    }
}
