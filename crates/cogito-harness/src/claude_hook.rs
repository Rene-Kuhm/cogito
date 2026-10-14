//! Claude Code's PreToolUse guardrail.
//!
//! Claude Code runs `cogito hook claude-pre-tool-use` before `Read` and `Grep`,
//! with the call as JSON on stdin. The first time Claude reads a large file
//! whole, or greps for a plain name, the call is blocked (exit 2) and the
//! message on stderr, which Claude Code hands to the model, carries Cogito's
//! outline or search result instead. Repeating the exact same call goes
//! through: the agent is shown the cheap answer first, never stuck.
//!
//! Every failure path allows the call. A guardrail that errors must not cost
//! the user a tool call.

use cogito_harness_core::{Cogito, index, map};
use serde_json::Value;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Files shorter than this are read whole without comment.
const OUTLINE_MIN_LINES: usize = 250;
/// A `Read` with a limit above this still counts as reading the whole file.
const WHOLE_READ_LIMIT: u64 = 400;

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

    let (key, message) = match tool {
        "Read" => match read_nudge(&root, input).await? {
            Some(found) => found,
            None => return Ok(None),
        },
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

async fn read_nudge(root: &Path, input: &Value) -> anyhow::Result<Option<(String, String)>> {
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

    let (cogito, project) = open(root)?;
    let (fingerprint, _) = cogito.fingerprint(&project, false).await?;

    // Try the index first.
    let decls = if let Some(conn) = cogito.connection() {
        index::outline(&conn, project.id, &rel)
            .map(|ds| ds.into_iter().map(Into::into).collect::<Vec<_>>())
            .unwrap_or_default()
    } else {
        vec![]
    };

    let decls = if decls.is_empty() {
        let content = std::fs::read_to_string(root.join(&rel)).unwrap_or_default();
        let ext = rel.extension().and_then(|e| e.to_str()).unwrap_or("");
        map::outline::declarations(&content, ext)
    } else {
        decls
    };

    let outline = map::render_outline(root, Some(&fingerprint), &rel, &decls, 0);
    let message = format!(
        "{} has {lines} lines. Cogito's outline instead of the whole file:\n\n{}\n\
         Read the range you need with offset/limit. Reading the whole file again is allowed \
         if you really need it.",
        rel.display(),
        outline.trim_end()
    );
    Ok(Some((format!("read:{}", rel.display()), message)))
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

/// Record `key` for this session; `false` if it was already recorded.
fn first_time(root: &Path, session: &str, key: &str) -> anyhow::Result<bool> {
    let safe: String = session
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(64)
        .collect();
    let dir = root.join(".cogito/nudges");
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(if safe.is_empty() { "default" } else { &safe });
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
}
