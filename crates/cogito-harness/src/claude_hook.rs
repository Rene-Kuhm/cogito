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
    let file = PathBuf::from(path);
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
}
