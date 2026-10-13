//! Agent integration registry.
//!
//! Knows how to detect, install and uninstall Cogito for each supported AI
//! agent. Every mutation backs up the target file first and merges into the
//! existing configuration instead of overwriting it.
//!
//! ## Formats
//!
//! | Agent | File | Shape |
//! |---|---|---|
//! | Gentle Shell / pi | `~/.gentle-shell/agent/settings.json` | `packages: ["npm:cogito-harness"]` |
//! | Claude Code | `~/.claude.json` | `mcpServers.cogito` |
//! | Codex | `~/.codex/config.toml` | `[mcp_servers.cogito]` |
//! | Gemini CLI | `~/.gemini/settings.json` | `mcpServers.cogito` |
//! | opencode | `~/.config/opencode/opencode.json` | `mcp.cogito` |
//! | Crush | `~/.config/crush/crush.json` | `mcp.cogito` |

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// Home directory resolution, honouring `COGITO_HOME` for tests.
pub fn home_dir() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var("COGITO_HOME") {
        return Ok(PathBuf::from(custom));
    }
    #[allow(deprecated)]
    std::env::home_dir().context("Cannot determine the home directory")
}

/// The MCP server name registered with every agent.
pub const SERVER_KEY: &str = "cogito";

/// The npm package that gives Gentle Shell first-class support.
pub const PI_PACKAGE: &str = "npm:cogito-harness";

/// Bare npm name behind [`PI_PACKAGE`].
pub const PI_PACKAGE_NAME: &str = "cogito-harness";

/// What `install` should do about Pi's npm package.
///
/// A bare boolean is not enough: `Leave` has to exist so that a registry we
/// could not reach does not silently delete a declaration the user added by
/// hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PiPackageAction {
    /// Ensure the package is declared.
    Declare,
    /// Ensure it is not declared. Used when the registry positively says 404,
    /// so a declaration that would break Pi can be repaired.
    Remove,
    /// Touch nothing. Used when npm could not be reached.
    Leave,
}

/// Outcome of asking npm whether a package exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NpmCheck {
    /// The registry has it.
    Published,
    /// The registry answered 404.
    NotFound,
    /// npm is missing, the registry is unreachable, or the check timed out.
    /// The reason is for display only.
    Unavailable(String),
}

/// Ask npm whether `name` exists in the configured registry.
///
/// Declaring a package Pi cannot install is not harmless: Pi runs
/// `npm install` for every entry in `packages`, so a 404 aborts package
/// resolution and the agent fails to start. Setup therefore checks first.
///
/// `npm view` is used rather than a direct request to registry.npmjs.org so
/// that whatever registry, mirror or scope the user configured is respected.
fn npm_check(name: &str) -> NpmCheck {
    npm_check_with_registry(name, None)
}

/// `npm_check`, optionally against another registry (used by tests).
fn npm_check_with_registry(name: &str, registry: Option<&str>) -> NpmCheck {
    use std::io::Read;
    use std::process::{Command, Stdio};

    let mut command = Command::new("npm");
    command
        .args([
            "view",
            name,
            "version",
            // Do not let a slow registry hang setup.
            "--fetch-timeout=8000",
            "--fetch-retries=0",
            "--no-audit",
            "--no-fund",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(registry) = registry {
        command.env("npm_config_registry", registry);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => return NpmCheck::Unavailable(format!("npm not runnable: {e}")),
    };

    // Drain stderr on its own thread so a chatty npm can never fill the pipe
    // and stall while we wait for it to exit.
    let mut stderr_pipe = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_string(&mut text);
        }
        text
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let stderr = reader.join().unwrap_or_default();
                return classify_npm_view(status.success(), &stderr);
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return NpmCheck::Unavailable("npm timed out after 12s".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return NpmCheck::Unavailable(format!("npm did not run: {e}")),
        }
    }
}

/// Turn the outcome of `npm view` into an `NpmCheck`.
///
/// npm exits non-zero for a missing package and for a network failure alike,
/// so the exit code alone cannot tell them apart. The error code npm prints
/// can: only `E404` proves the package is not there. Anything else (offline,
/// DNS, proxy, auth) is `Unavailable`, because treating it as "not found"
/// would make setup remove a declaration that is fine.
fn classify_npm_view(success: bool, stderr: &str) -> NpmCheck {
    if success {
        return NpmCheck::Published;
    }
    // npm 7+ prints `npm error code E404`; older npm printed `npm ERR! code E404`.
    let code = stderr.lines().find_map(|line| {
        let mut words = line.split_whitespace().skip_while(|w| *w != "code");
        words.next()?;
        words.next()
    });
    match code {
        Some("E404") => NpmCheck::NotFound,
        Some(code) => NpmCheck::Unavailable(format!("npm failed with {code}")),
        None => NpmCheck::Unavailable("npm failed without an error code".to_string()),
    }
}

/// Public wrapper: is the Pi package installable right now?
pub fn pi_package_check() -> NpmCheck {
    npm_check(PI_PACKAGE_NAME)
}

/// Whether Pi's settings already declare the Cogito package.
pub fn pi_package_declared(home: &Path) -> bool {
    let path = Agent::GentleShell.config_path(home);
    read_json(&path)
        .ok()
        .and_then(|v| {
            v.get("packages")
                .and_then(|p| p.as_array())
                .map(|arr| arr.iter().any(|e| e.as_str() == Some(PI_PACKAGE)))
        })
        .unwrap_or(false)
}

/// Every agent Cogito knows how to configure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    GentleShell,
    ClaudeCode,
    Codex,
    Gemini,
    Opencode,
    Crush,
}

/// All supported agents, in display order.
pub const ALL: &[Agent] = &[
    Agent::GentleShell,
    Agent::ClaudeCode,
    Agent::Codex,
    Agent::Gemini,
    Agent::Opencode,
    Agent::Crush,
];

impl Agent {
    /// Stable identifier used on the command line.
    pub fn id(self) -> &'static str {
        match self {
            Agent::GentleShell => "gentle-shell",
            Agent::ClaudeCode => "claude-code",
            Agent::Codex => "codex",
            Agent::Gemini => "gemini",
            Agent::Opencode => "opencode",
            Agent::Crush => "crush",
        }
    }

    /// Human-readable name.
    pub fn display(self) -> &'static str {
        match self {
            Agent::GentleShell => "Gentle Shell (pi)",
            Agent::ClaudeCode => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Gemini => "Gemini CLI",
            Agent::Opencode => "opencode",
            Agent::Crush => "Crush",
        }
    }

    /// Parse an identifier, accepting a few aliases.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "gentle-shell" | "gentle" | "pi" | "gentleshell" => Some(Agent::GentleShell),
            "claude-code" | "claude" | "claudecode" => Some(Agent::ClaudeCode),
            "codex" | "openai-codex" => Some(Agent::Codex),
            "gemini" | "gemini-cli" => Some(Agent::Gemini),
            "opencode" | "open-code" => Some(Agent::Opencode),
            "crush" => Some(Agent::Crush),
            _ => None,
        }
    }

    /// Whether this agent uses the npm pi-package instead of an MCP entry.
    pub fn uses_pi_package(self) -> bool {
        matches!(self, Agent::GentleShell)
    }

    /// For agents whose MCP configuration lives in its own file, separate from
    /// their main settings. Only Pi / Gentle Shell works this way today; every
    /// other agent reads MCP servers from the file returned by `config_path`.
    pub fn mcp_config_path(self, home: &Path) -> Option<PathBuf> {
        if self != Agent::GentleShell {
            return None;
        }
        let gentle = home.join(".gentle-shell/agent");
        if gentle.exists() || !home.join(".pi").exists() {
            Some(gentle.join("mcp.json"))
        } else {
            Some(home.join(".pi/agent/mcp.json"))
        }
    }

    /// Path of the file this agent is configured in.
    pub fn config_path(self, home: &Path) -> PathBuf {
        match self {
            Agent::GentleShell => home.join(".gentle-shell/agent/settings.json"),
            Agent::ClaudeCode => home.join(".claude.json"),
            Agent::Codex => home.join(".codex/config.toml"),
            Agent::Gemini => home.join(".gemini/settings.json"),
            Agent::Opencode => home.join(".config/opencode/opencode.json"),
            Agent::Crush => home.join(".config/crush/crush.json"),
        }
    }

    /// Whether the agent itself appears to be installed on this machine.
    pub fn agent_present(self, home: &Path) -> bool {
        match self {
            Agent::GentleShell => home.join(".gentle-shell").exists(),
            Agent::ClaudeCode => home.join(".claude.json").exists(),
            Agent::Codex => home.join(".codex").exists(),
            Agent::Gemini => home.join(".gemini").exists(),
            Agent::Opencode => home.join(".config/opencode").exists(),
            Agent::Crush => home.join(".config/crush").exists(),
        }
    }
}

/// A file Cogito needs to create alongside an agent's main config.
#[derive(Debug, Clone)]
pub struct ExtraFile {
    pub path: PathBuf,
    pub content: String,
    pub executable: bool,
}

/// One-line server summary Pi shows in the system prompt, uses for tool
/// search ranking, and returns from codemode's `describeNamespace()`.
const SERVER_DESCRIPTION: &str =
    "Project context harness: code fingerprint, semantic tags and architectural decisions.";

/// The Claude Code SessionStart hook script.
const CLAUDE_HOOK_SCRIPT: &str = r#"#!/bin/sh
# Cogito SessionStart hook.
#
# Prints the project card. Claude Code adds a SessionStart hook's output to the
# session context, so every session starts knowing the project's modules,
# key docs and how to look code up with Cogito, without spending a tool call
# on orientation. `cogito map --card` reuses the stored index and rescans only
# when HEAD moved, so the index is kept current as a side effect.
#
# This script always exits 0. A missing `cogito`, a directory that is not a
# project, or a failure all print nothing, so it can never block or fail a
# session.

command -v cogito >/dev/null 2>&1 || exit 0
cd "${CLAUDE_PROJECT_DIR:-.}" 2>/dev/null || exit 0

# Only touch directories that already look like a project. Scanning
# unconditionally would create a `.cogito/` in $HOME, or in any unrelated tree
# an agent happens to start in, and walk the whole thing.
#
# `-e`, not `-d`: in a git worktree or a submodule `.git` is a *file*
# containing `gitdir: …`, so a directory test rejects the very repositories
# most likely to be large.
[ -e .git ] || [ -d .cogito ] || exit 0

# Bounded, so a first scan of a huge repository cannot hold the session.
if command -v timeout >/dev/null 2>&1; then
  card=$(timeout 20 cogito map --card 2>/dev/null)
else
  card=$(cogito map --card 2>/dev/null)
fi
[ -n "$card" ] || exit 0

printf '<cogito_project>\n%s\n</cogito_project>\n' "$card"
exit 0
"#;

/// The opencode plugin, loaded from a real file so it can be linted and
/// type-checked instead of living as an opaque string constant.
const OPENCODE_PLUGIN: &str = include_str!("../assets/opencode/cogito.js");

/// The Claude Code PreToolUse guardrail. The decision lives in the binary
/// (`cogito hook claude-pre-tool-use`) so it is tested Rust, not shell, and
/// needs no `jq`; Claude Code's hook JSON passes through on stdin.
const CLAUDE_GUARD_SCRIPT: &str = r#"#!/bin/sh
# Cogito PreToolUse hook (Read, Grep).
#
# The first time Claude reads a large file whole, or greps for a plain name,
# it gets Cogito's outline or search result instead (exit 2, message on
# stderr). Repeating the exact same call goes through. Any problem exits 0, so
# a tool call is never blocked by accident.

command -v cogito >/dev/null 2>&1 || exit 0
exec cogito hook claude-pre-tool-use
"#;

/// One hook Cogito registers in an agent's settings.
#[derive(Debug, Clone)]
pub struct HookSpec {
    /// Settings key, e.g. `SessionStart`.
    pub event: &'static str,
    /// Which sources or tools the hook applies to (a regex in Claude Code).
    pub matcher: &'static str,
    pub script: PathBuf,
    pub content: &'static str,
    /// Seconds the agent waits for the hook.
    pub timeout: u64,
}

impl Agent {
    /// Hooks Cogito installs for this agent.
    pub fn hooks(self, home: &Path) -> Vec<HookSpec> {
        match self {
            Agent::ClaudeCode => vec![
                HookSpec {
                    event: "SessionStart",
                    matcher: "^(startup|resume|clear|compact|fork)$",
                    script: home.join(".claude/hooks/cogito-session-start.sh"),
                    content: CLAUDE_HOOK_SCRIPT,
                    timeout: HOOK_TIMEOUT_SECS,
                },
                HookSpec {
                    event: "PreToolUse",
                    matcher: "^(Read|Grep)$",
                    script: home.join(".claude/hooks/cogito-pre-tool-use.sh"),
                    content: CLAUDE_GUARD_SCRIPT,
                    timeout: GUARD_TIMEOUT_SECS,
                },
            ],
            _ => Vec::new(),
        }
    }

    /// Files Cogito writes besides the agent's main configuration file.
    pub fn extra_files(self, home: &Path, _binary: &Path) -> Vec<ExtraFile> {
        match self {
            Agent::ClaudeCode => self
                .hooks(home)
                .into_iter()
                .map(|hook| ExtraFile {
                    path: hook.script,
                    content: hook.content.to_string(),
                    executable: true,
                })
                .collect(),
            Agent::Opencode => vec![ExtraFile {
                path: home.join(".config/opencode/plugins/cogito.js"),
                content: OPENCODE_PLUGIN.to_string(),
                executable: false,
            }],
            _ => Vec::new(),
        }
    }

    /// Whether this agent gets hooks in its config file.
    fn has_session_hook(self) -> bool {
        !self.hooks(Path::new("")).is_empty()
    }

    /// Canonical path of this agent's session-start hook script.
    ///
    /// Independent of the installed binary so that detecting, writing and
    /// removing the hook entry always agree on the same file.
    #[cfg(test)]
    pub fn hook_script_path(self, home: &Path) -> Option<PathBuf> {
        self.hooks(home)
            .into_iter()
            .find(|h| h.event == "SessionStart")
            .map(|h| h.script)
    }
}

/// Write (or refresh) every extra file for `agent`.
fn write_extra_files(agent: Agent, home: &Path, binary: &Path) -> Result<()> {
    for file in agent.extra_files(home, binary) {
        if let Some(parent) = file.path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create {}", parent.display()))?;
        }
        std::fs::write(&file.path, &file.content)
            .with_context(|| format!("Failed to write {}", file.path.display()))?;

        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&file.path)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&file.path, perms)?;
        }
    }
    Ok(())
}

/// Remove every extra file belonging to `agent`.
fn remove_extra_files(agent: Agent, home: &Path, binary: &Path) -> Result<()> {
    for file in agent.extra_files(home, binary) {
        if file.path.exists() {
            std::fs::remove_file(&file.path)
                .with_context(|| format!("Failed to remove {}", file.path.display()))?;
        }
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The agent is not installed on this machine.
    AgentMissing,
    /// The agent is installed but Cogito is not configured.
    NotConfigured,
    /// Cogito is configured, but not the way this binary would write it:
    /// a stale command path, a missing field, or an old hook script.
    Outdated,
    /// Cogito is configured exactly as this binary would write it.
    Configured,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::AgentMissing => "not installed",
            Status::NotConfigured => "installed, cogito missing",
            Status::Outdated => "configured but outdated",
            Status::Configured => "cogito configured",
        }
    }

    /// Whether Cogito's configuration is present in any form.
    pub fn is_present(self) -> bool {
        matches!(self, Status::Configured | Status::Outdated)
    }
}

/// What an install or uninstall did to the file.
#[derive(Debug, Clone)]
pub enum Change {
    /// The file was modified. Contains the backup path.
    Written { backup: Option<PathBuf> },
    /// Nothing changed because the desired state already held.
    AlreadyCorrect,
}

/// The MCP server entry Cogito wants for `agent` at `binary`.
///
/// Keeping this in one place is what lets `status` detect a stale entry and
/// `install` repair it, instead of treating "a key named cogito exists" as
/// success. See `OWNED_KEYS` for which of these fields are enforced.
fn desired_server_entry(agent: Agent, bin: &str) -> Value {
    match agent {
        Agent::GentleShell => json!({
            "command": bin,
            "args": ["mcp"],
            "exposure": "direct",
            "description": SERVER_DESCRIPTION,
        }),
        Agent::Opencode => json!({
            "type": "local",
            "command": [bin, "mcp"],
            "enabled": true,
        }),
        Agent::Crush => json!({
            "type": "stdio",
            "command": bin,
            "args": ["mcp"],
        }),
        Agent::Gemini => json!({
            "command": bin,
            "args": ["mcp"],
        }),
        // Claude Code and any other client using the `mcpServers` shape.
        _ => json!({
            "type": "stdio",
            "command": bin,
            "args": ["mcp"],
            "env": {},
        }),
    }
}

/// Keys that say how to launch the server. Cogito owns them: a different value
/// means the entry is stale, and `install` overwrites it.
///
/// Every other key in the desired entry (`exposure`, `enabled`, `env`, …) is a
/// default: written when missing, but a value already there is the user's
/// choice — for example Pi's `/mcp` writes `"enabled": false` into this same
/// entry — and is kept.
const OWNED_KEYS: &[&str] = &["type", "command", "args"];

/// Keys older versions wrote that no agent reads. Their presence means stale.
const RETIRED_KEYS: &[&str] = &["lifecycle"];

/// Whether an existing JSON server entry already has what Cogito would write.
fn entry_current(entry: &Value, want: &Value) -> bool {
    let (Some(have), Some(want)) = (entry.as_object(), want.as_object()) else {
        return false;
    };
    if RETIRED_KEYS.iter().any(|key| have.contains_key(*key)) {
        return false;
    }
    want.iter().all(|(key, value)| match have.get(key) {
        None => false,
        Some(existing) => !OWNED_KEYS.contains(&key.as_str()) || existing == value,
    })
}

/// Merge the desired entry into an existing one: owned keys are overwritten,
/// defaults only fill gaps, retired keys go, and anything else stays.
fn merge_server_entry(existing: Option<&Value>, want: Value) -> Value {
    let mut merged = existing
        .and_then(|e| e.as_object())
        .cloned()
        .unwrap_or_default();
    for key in RETIRED_KEYS {
        merged.remove(*key);
    }
    if let Value::Object(want) = want {
        for (key, value) in want {
            if OWNED_KEYS.contains(&key.as_str()) || !merged.contains_key(&key) {
                merged.insert(key, value);
            }
        }
    }
    Value::Object(merged)
}

/// The TOML fields Cogito owns for Codex, as `(key, expected literal)`.
fn desired_codex_fields(bin: &str) -> [(&'static str, String); 2] {
    [
        ("command", format!("\"{bin}\"")),
        ("args", "[\"mcp\"]".to_string()),
    ]
}

/// Compare an existing Codex `[mcp_servers.cogito]` table with the desired
/// fields. `enabled` only has to exist: its value is the user's choice.
fn codex_entry_matches(existing: &toml_edit::Item, bin: &str) -> bool {
    let Some(table) = existing.as_table_like() else {
        return false;
    };
    table.contains_key("enabled")
        && desired_codex_fields(bin).iter().all(|(key, want)| {
            table
                .get(key)
                .is_some_and(|have| normalize_toml(&have.to_string()) == normalize_toml(want))
        })
}

/// Collapse whitespace so `[ "mcp" ]` and `["mcp"]` compare equal.
fn normalize_toml(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join("")
}

/// Whether every companion artifact exists with the expected content.
fn extras_current(agent: Agent, home: &Path, binary: &Path) -> bool {
    agent.extra_files(home, binary).iter().all(|file| {
        std::fs::read_to_string(&file.path)
            .map(|existing| existing == file.content)
            .unwrap_or(false)
    })
}

/// Whether the agent's settings still reference Cogito's session hook. The
/// script existing on disk is not enough: without the entry it never runs.
fn hook_registered(agent: Agent, home: &Path) -> Result<bool> {
    if !agent.has_session_hook() {
        return Ok(true);
    }
    let Some(hook_path) = hook_config_path(agent, home) else {
        return Ok(true);
    };
    if !hook_path.exists() {
        return Ok(false);
    }
    let settings = read_json(&hook_path)?;
    // Every hook registered and synchronous: an `async` hook's output never
    // reaches the session, so an entry from an earlier version is outdated.
    Ok(agent.hooks(home).iter().all(|spec| {
        settings
            .get("hooks")
            .and_then(|h| h.get(spec.event))
            .and_then(|s| s.as_array())
            .is_some_and(|entries| {
                entries.iter().any(|e| {
                    e.get("hooks")
                        .and_then(|h| h.as_array())
                        .is_some_and(|hooks| {
                            hooks.iter().any(|h| {
                                owns_command(h, &spec.script)
                                    && h.get("async").and_then(|a| a.as_bool()) != Some(true)
                            })
                        })
                })
            })
    }))
}

/// Inspect how `agent` is configured, relative to what `binary` would write.
pub fn status(agent: Agent, home: &Path, binary: &Path) -> Result<Status> {
    if !agent.agent_present(home) {
        return Ok(Status::AgentMissing);
    }
    let path = agent.config_path(home);
    if !path.exists() {
        return Ok(Status::NotConfigured);
    }

    let bin = binary.display().to_string();
    let want = desired_server_entry(agent, &bin);

    if agent.uses_pi_package() {
        // The npm package is optional: it adds the status line and `/cogito`,
        // while the MCP entry is what provides the tools. Requiring the
        // package here would report a working setup as missing whenever the
        // package is not published or not declared.
        let Some(mcp_path) = agent.mcp_config_path(home) else {
            return Ok(Status::NotConfigured);
        };
        if !mcp_path.exists() {
            return Ok(Status::NotConfigured);
        }
        let mcp = read_json(&mcp_path)?;
        let Some(entry) = mcp.get("mcpServers").and_then(|m| m.get(SERVER_KEY)) else {
            return Ok(Status::NotConfigured);
        };
        if !entry_current(entry, &want) || !extras_current(agent, home, binary) {
            return Ok(Status::Outdated);
        }
        return Ok(Status::Configured);
    }

    if agent == Agent::Codex {
        let text = std::fs::read_to_string(&path)?;
        let doc: toml_edit::DocumentMut = text.parse()?;
        let Some(existing) = doc.get("mcp_servers").and_then(|t| t.get(SERVER_KEY)) else {
            return Ok(Status::NotConfigured);
        };
        return Ok(if codex_entry_matches(existing, &bin) {
            Status::Configured
        } else {
            Status::Outdated
        });
    }

    let value = read_json(&path)?;
    let section = match agent {
        Agent::Opencode | Agent::Crush => "mcp",
        _ => "mcpServers",
    };
    let Some(entry) = value.get(section).and_then(|m| m.get(SERVER_KEY)) else {
        return Ok(Status::NotConfigured);
    };

    if !entry_current(entry, &want)
        || !extras_current(agent, home, binary)
        || !hook_registered(agent, home)?
    {
        return Ok(Status::Outdated);
    }
    Ok(Status::Configured)
}

// ============================================================
// AGENTS.md block management (Codex and opencode)
// ============================================================

/// Markers that delimit Cogito's section in AGENTS.md.
const AGENTS_START: &str = "<!-- cogito:start -->";
const AGENTS_END: &str = "<!-- cogito:end -->";

/// The block inserted into AGENTS.md for Codex and opencode.
///
/// Kept short: Codex limits the total file to 32 KiB and opencode concatenates
/// all AGENTS.md files from project root to the working directory.
const AGENTS_BLOCK_CONTENT: &str = "
**This project is indexed by Cogito** — run `cogito map --card` to see the
project card.

**Use these MCP tools instead of whole-file reads and greps:**
- `project_map` — module overview (once per session to orient)
- `search_code` — find symbols, files and modules by name (returns file:line)
- `file_outline` — a file's declarations with line ranges
- `related_files` — what a file imports and what imports it

**Guardrail:** Before reading a large file, call `file_outline` to get its
declarations with line ranges, then read only the range you need. Reading the
same range twice is always allowed.
";

/// Returns the path to the AGENTS.md file in `project_root`.
pub fn agents_md_path(project_root: &Path) -> PathBuf {
    project_root.join("AGENTS.md")
}

/// Whether `agent` reads AGENTS.md at session start.
pub fn reads_agents_md(agent: Agent) -> bool {
    matches!(agent, Agent::Codex | Agent::Opencode)
}

/// Install or refresh the Cogito block in `project_root/AGENTS.md`.
///
/// Idempotent: a second install with the same content is byte-identical.
/// Returns the change that was (or would be) made, or `AlreadyCorrect` if
/// the file is already correct.
pub fn install_agents_md_block(project_root: &Path, dry_run: bool) -> Result<Change> {
    let path = agents_md_path(project_root);
    let new_block = format!(
        "{}\n{}{}\n",
        AGENTS_START,
        AGENTS_BLOCK_CONTENT.trim(),
        AGENTS_END
    );

    let existing = if path.exists() {
        std::fs::read_to_string(&path)?
    } else {
        String::new()
    };

    if existing.contains(AGENTS_START) && existing.contains(AGENTS_END) {
        // Block exists: replace only the content between the markers.
        let start_idx = existing.find(AGENTS_START).unwrap();
        let end_idx = existing.find(AGENTS_END).unwrap() + AGENTS_END.len();

        let before = &existing[..start_idx];
        let after = &existing[end_idx..];

        // Strip any trailing newline from `after` so we control the separator.
        let after = after.trim_end_matches('\n');

        let replacement = if after.is_empty() {
            format!("{}{}", before, new_block.trim_end())
        } else {
            format!("{}{}\n{}", before, new_block.trim_end(), after)
        };

        if replacement == existing {
            return Ok(Change::AlreadyCorrect);
        }

        if dry_run {
            println!("  Would update {}", path.display());
            return Ok(Change::AlreadyCorrect);
        }

        let backup = backup_file(&path)?;
        std::fs::write(&path, &replacement)?;
        return Ok(Change::Written { backup });
    }

    // No block: append one.
    let separator = if existing.is_empty() || existing.ends_with('\n') {
        String::new()
    } else {
        "\n".to_string()
    };

    let new_content = format!("{}{}{}", existing, separator, new_block.trim_end());

    if new_content == existing {
        return Ok(Change::AlreadyCorrect);
    }

    if dry_run {
        println!("  Would write {}", path.display());
        return Ok(Change::AlreadyCorrect);
    }

    // No backup for AGENTS.md: the block is reversible via remove_agents_md_block,
    // and timestamped backups would pollute the user's tracked working tree.
    let backup = if path.exists() {
        backup_file(&path)?
    } else {
        None
    };
    std::fs::write(&path, &new_content)?;
    Ok(Change::Written { backup })
}

/// Remove the Cogito block from `project_root/AGENTS.md`.
///
/// Idempotent: removing when there is no block is a no-op and does not create
/// the file. Returns `AlreadyCorrect` when nothing was changed.
pub fn remove_agents_md_block(project_root: &Path) -> Result<Change> {
    let path = agents_md_path(project_root);

    if !path.exists() {
        return Ok(Change::AlreadyCorrect);
    }

    let existing = std::fs::read_to_string(&path)?;

    if !existing.contains(AGENTS_START) {
        return Ok(Change::AlreadyCorrect);
    }

    // Find the block boundaries.
    let start_idx = existing.find(AGENTS_START).unwrap();
    let end_idx = existing.find(AGENTS_END).unwrap() + AGENTS_END.len();

    let before = &existing[..start_idx];
    let after = &existing[end_idx..];

    // Remove trailing blank lines from `before` and leading blank lines from
    // `after`, but preserve at least one newline between the sections.
    let before = before.trim_end_matches('\n');
    let after = after.trim_start_matches('\n');

    let replacement = if before.is_empty() && after.is_empty() {
        String::new()
    } else if before.is_empty() {
        after.to_string()
    } else if after.is_empty() {
        before.to_string()
    } else {
        format!("{}\n{}", before, after)
    };

    if replacement == existing {
        return Ok(Change::AlreadyCorrect);
    }

    let backup = backup_file(&path)?;
    // If the file is now empty, remove it rather than leaving a blank file.
    if replacement.is_empty() {
        std::fs::remove_file(&path)?;
    } else {
        std::fs::write(&path, &replacement)?;
    }
    Ok(Change::Written { backup })
}

// ============================================================
// Agent install / uninstall
// ============================================================

/// Configure `agent` to use the `cogito` binary at `binary`.
/// Install or repair `agent`'s configuration.
///
/// `pi_package` only affects Pi / Gentle Shell: it decides what happens to the
/// npm package in `settings.json`. The caller resolves that from the registry,
/// because declaring a package npm cannot install makes Pi fail to start.
pub fn install(
    agent: Agent,
    home: &Path,
    binary: &Path,
    pi_package: PiPackageAction,
) -> Result<Change> {
    let package_settled = if !agent.uses_pi_package() {
        true
    } else {
        let declared = pi_package_declared(home);
        match pi_package {
            PiPackageAction::Declare => declared,
            PiPackageAction::Remove => !declared,
            PiPackageAction::Leave => true,
        }
    };

    if status(agent, home, binary)? == Status::Configured && package_settled {
        return Ok(Change::AlreadyCorrect);
    }

    let path = agent.config_path(home);
    let backup = backup_file(&path)?;
    let bin = binary.display().to_string();

    if agent.uses_pi_package() {
        // (1) Reconcile the pi package that provides the session adapter.
        if pi_package != PiPackageAction::Leave {
            let mut value = read_json_or_default(&path);
            let obj = value
                .as_object_mut()
                .context("settings.json is not an object")?;
            let packages = obj.entry("packages").or_insert_with(|| json!([]));
            let arr = packages
                .as_array_mut()
                .context("`packages` is not an array")?;
            let present = arr.iter().any(|v| v.as_str() == Some(PI_PACKAGE));
            match pi_package {
                PiPackageAction::Declare if !present => arr.push(json!(PI_PACKAGE)),
                PiPackageAction::Remove if present => {
                    arr.retain(|v| v.as_str() != Some(PI_PACKAGE));
                }
                _ => {}
            }
            write_json(&path, &value)?;
        }

        // (2) Register the MCP server. Pi reads MCP servers from its own file.
        if let Some(mcp_path) = agent.mcp_config_path(home) {
            let _ = backup_file(&mcp_path)?;
            let mut mcp = read_json_or_default(&mcp_path);
            let obj = mcp.as_object_mut().context("mcp.json is not an object")?;
            let servers = obj
                .entry("mcpServers")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .context("`mcpServers` is not an object")?;
            let merged =
                merge_server_entry(servers.get(SERVER_KEY), desired_server_entry(agent, &bin));
            servers.insert(SERVER_KEY.to_string(), merged);
            write_json(&mcp_path, &mcp)?;
        }
        return Ok(Change::Written { backup });
    }

    if agent == Agent::Codex {
        let text = if path.exists() {
            std::fs::read_to_string(&path)?
        } else {
            String::new()
        };
        let mut doc: toml_edit::DocumentMut = text.parse().unwrap_or_default();

        if doc.get("mcp_servers").is_none() {
            doc["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let servers = doc["mcp_servers"]
            .as_table_mut()
            .context("`mcp_servers` is not a table")?;
        // Render as `[mcp_servers.cogito]` rather than a bare `[mcp_servers]`.
        servers.set_implicit(true);

        // Edit the existing table in place so fields Cogito does not own (a
        // user's `enabled = false`, `env`, timeouts) survive a repair.
        if !servers.get(SERVER_KEY).is_some_and(|item| item.is_table()) {
            servers.insert(SERVER_KEY, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        let server = servers[SERVER_KEY]
            .as_table_mut()
            .context("`mcp_servers.cogito` is not a table")?;
        server["command"] = toml_edit::value(bin);
        server["args"] = toml_edit::value(toml_edit::Array::from_iter(["mcp"]));
        if !server.contains_key("enabled") {
            server["enabled"] = toml_edit::value(true);
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, doc.to_string())?;
        return Ok(Change::Written { backup });
    }

    let mut value = read_json_or_default(&path);
    let section = match agent {
        Agent::Opencode | Agent::Crush => "mcp",
        _ => "mcpServers",
    };
    let obj = value.as_object_mut().context("config is not an object")?;
    let servers = obj
        .entry(section)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .with_context(|| format!("`{section}` is not an object"))?;
    let merged = merge_server_entry(servers.get(SERVER_KEY), desired_server_entry(agent, &bin));
    servers.insert(SERVER_KEY.to_string(), merged);

    write_json(&path, &value)?;

    // Companion artifacts: hook scripts and plugins that run without the model
    // asking. Written once the MCP entry is in place so a failure here leaves a
    // working (if less automatic) setup.
    write_extra_files(agent, home, binary)?;

    if agent.has_session_hook()
        && let Some(hook_path) = hook_config_path(agent, home)
    {
        let _ = backup_file(&hook_path)?;
        let mut settings = read_json_or_default(&hook_path);
        for spec in agent.hooks(home) {
            upsert_hook(&mut settings, &spec)?;
        }
        write_json(&hook_path, &settings)?;
    }

    Ok(Change::Written { backup })
}

/// Remove Cogito's configuration from `agent`.
pub fn uninstall(agent: Agent, home: &Path) -> Result<Change> {
    let path = agent.config_path(home);
    if !path.exists() {
        return Ok(Change::AlreadyCorrect);
    }

    let backup = backup_file(&path)?;

    if agent.uses_pi_package() {
        if path.exists() {
            let mut value = read_json(&path)?;
            if let Some(arr) = value.get_mut("packages").and_then(|p| p.as_array_mut()) {
                arr.retain(|v| v.as_str() != Some(PI_PACKAGE));
            }
            write_json(&path, &value)?;
        }
        if let Some(mcp_path) = agent.mcp_config_path(home)
            && mcp_path.exists()
        {
            let _ = backup_file(&mcp_path)?;
            let mut mcp = read_json(&mcp_path)?;
            if let Some(table) = mcp.get_mut("mcpServers").and_then(|m| m.as_object_mut()) {
                table.remove(SERVER_KEY);
            }
            write_json(&mcp_path, &mcp)?;
        }
        return Ok(Change::Written { backup });
    }

    if agent == Agent::Codex {
        let text = std::fs::read_to_string(&path)?;
        let mut doc: toml_edit::DocumentMut = text.parse()?;
        if let Some(servers) = doc.get_mut("mcp_servers").and_then(|t| t.as_table_mut()) {
            servers.remove(SERVER_KEY);
        }
        std::fs::write(&path, doc.to_string())?;
        return Ok(Change::Written { backup });
    }

    let mut value = read_json(&path)?;
    let section = match agent {
        Agent::Opencode | Agent::Crush => "mcp",
        _ => "mcpServers",
    };
    if let Some(table) = value.get_mut(section).and_then(|m| m.as_object_mut()) {
        table.remove(SERVER_KEY);
    }
    write_json(&path, &value)?;

    if agent.has_session_hook()
        && let Some(hook_path) = hook_config_path(agent, home)
        && hook_path.exists()
    {
        let _ = backup_file(&hook_path)?;
        let mut settings = read_json(&hook_path)?;
        for spec in agent.hooks(home) {
            remove_hook(&mut settings, &spec);
        }
        write_json(&hook_path, &settings)?;
    }

    remove_extra_files(agent, home, &PathBuf::from("cogito"))?;
    Ok(Change::Written { backup })
}

// ============================================================
// Helpers
// ============================================================

/// Insert (or repair) one of Cogito's hooks in a Claude-Code-style settings
/// object. Idempotent.
fn upsert_hook(value: &mut Value, spec: &HookSpec) -> Result<()> {
    let script = spec.script.as_path();
    let obj = value.as_object_mut().context("settings is not an object")?;
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("`hooks` is not an object")?;
    let session_start = hooks
        .entry(spec.event)
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .with_context(|| format!("`hooks.{}` is not an array", spec.event))?;

    // Match on the exact script path. A substring match on "cogito" would
    // also claim an unrelated hook that merely mentions the word, for example
    // a user script living under a checkout at ~/proyectos/cogito/.
    let wanted = format!("sh '{}'", script.display());
    match session_start
        .iter_mut()
        .find(|entry| entry_owns_script(entry, script))
    {
        // Repair our entry in place: earlier versions ran the hook `async`,
        // and Claude Code ignores an async hook's output, which is now the
        // whole point of the hook.
        Some(entry) => {
            if let Some(hooks) = entry.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                for hook in hooks.iter_mut().filter(|h| owns_command(h, script)) {
                    if let Some(hook) = hook.as_object_mut() {
                        hook.remove("async");
                        hook.insert("timeout".into(), json!(spec.timeout));
                        hook.insert("command".into(), json!(wanted));
                    }
                }
            }
        }
        None => session_start.push(json!({
            "matcher": spec.matcher,
            "hooks": [{
                "type": "command",
                "command": wanted,
                "timeout": spec.timeout,
            }]
        })),
    }
    Ok(())
}

/// Seconds Claude Code waits for the hook. The card prints in milliseconds
/// from a stored index; this covers a first scan of a large repository.
const HOOK_TIMEOUT_SECS: u64 = 30;

/// Seconds Claude Code waits for the guardrail before running the tool anyway.
const GUARD_TIMEOUT_SECS: u64 = 15;

/// Whether one hook object runs Cogito's script.
fn owns_command(hook: &Value, script: &Path) -> bool {
    let wanted = script.display().to_string();
    hook.get("command")
        .and_then(|c| c.as_str())
        .is_some_and(|c| c.contains(&wanted))
}

/// Whether a `SessionStart` entry is the one Cogito installed for `script`.
fn entry_owns_script(entry: &Value, script: &Path) -> bool {
    let wanted = script.display().to_string();
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .is_some_and(|hooks| {
            hooks.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|c| c.contains(&wanted))
            })
        })
}

/// Remove one of Cogito's hooks from a Claude-Code-style settings object.
fn remove_hook(value: &mut Value, spec: &HookSpec) {
    if let Some(arr) = value
        .get_mut("hooks")
        .and_then(|h| h.get_mut(spec.event))
        .and_then(|s| s.as_array_mut())
    {
        arr.retain(|entry| !entry_owns_script(entry, &spec.script));
    }
}

/// Configuration file that holds session hooks, when it differs from the file
/// that holds MCP servers (Claude Code keeps them in two files).
fn hook_config_path(agent: Agent, home: &Path) -> Option<PathBuf> {
    match agent {
        Agent::ClaudeCode => Some(home.join(".claude/settings.json")),
        _ => None,
    }
}

fn read_json(path: &Path) -> Result<Value> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;
    serde_json::from_str(&text)
        .with_context(|| format!("Failed to parse {} as JSON", path.display()))
}

fn read_json_or_default(path: &Path) -> Value {
    if path.exists() {
        read_json(path).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    }
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    // Leave an unchanged file untouched. Agents rewrite some of these files
    // while running (Claude Code and ~/.claude.json), so every write Cogito
    // skips is a race it cannot lose.
    if path.exists() && read_json(path).is_ok_and(|current| current == *value) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value)?;
    std::fs::write(path, format!("{text}\n"))
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

/// Copy `path` to a timestamped sibling before mutating it.
///
/// Returns `None` when there is nothing to back up (the file does not exist).
fn backup_file(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S");
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config");
    let backup = path.with_file_name(format!("{name}.cogito-backup-{stamp}"));
    std::fs::copy(path, &backup).with_context(|| {
        format!(
            "Failed to back up {} to {}",
            path.display(),
            backup.display()
        )
    })?;
    Ok(Some(backup))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn bin() -> PathBuf {
        PathBuf::from("/usr/local/bin/cogito")
    }

    #[test]
    fn test_parse_aliases() {
        assert_eq!(Agent::parse("pi"), Some(Agent::GentleShell));
        assert_eq!(Agent::parse("claude"), Some(Agent::ClaudeCode));
        assert_eq!(Agent::parse("gemini-cli"), Some(Agent::Gemini));
        assert_eq!(Agent::parse("nope"), None);
    }

    #[test]
    fn test_claude_install_and_uninstall() {
        let home = TempDir::new().unwrap();
        std::fs::write(
            home.path().join(".claude.json"),
            "{\n  \"numStartups\": 3\n}",
        )
        .unwrap();

        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::NotConfigured
        );

        let change = install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();
        assert!(matches!(change, Change::Written { backup: Some(_) }));
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Configured
        );

        // Existing keys must survive
        let value = read_json(&home.path().join(".claude.json")).unwrap();
        assert_eq!(value["numStartups"], 3);
        assert_eq!(
            value["mcpServers"]["cogito"]["command"],
            "/usr/local/bin/cogito"
        );
        assert_eq!(value["mcpServers"]["cogito"]["args"][0], "mcp");

        // Idempotent
        assert!(matches!(
            install(
                Agent::ClaudeCode,
                home.path(),
                &bin(),
                PiPackageAction::Leave
            )
            .unwrap(),
            Change::AlreadyCorrect
        ));

        uninstall(Agent::ClaudeCode, home.path()).unwrap();
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::NotConfigured
        );
        let value = read_json(&home.path().join(".claude.json")).unwrap();
        assert_eq!(value["numStartups"], 3, "unrelated keys must survive");
    }

    #[test]
    fn test_codex_install_preserves_toml() {
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(
            home.path().join(".codex/config.toml"),
            "model = \"gpt-5\"\n\n[mcp_servers.other]\ncommand = \"other\"\n",
        )
        .unwrap();

        install(Agent::Codex, home.path(), &bin(), PiPackageAction::Leave).unwrap();
        let text = std::fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();

        assert!(
            text.contains("model = \"gpt-5\""),
            "existing keys survive: {text}"
        );
        assert!(
            text.contains("[mcp_servers.other]"),
            "existing servers survive: {text}"
        );
        assert!(text.contains("mcp_servers.cogito"), "cogito added: {text}");
        assert_eq!(
            status(Agent::Codex, home.path(), &bin()).unwrap(),
            Status::Configured
        );

        uninstall(Agent::Codex, home.path()).unwrap();
        let text = std::fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();
        assert!(!text.contains("cogito"));
        assert!(text.contains("[mcp_servers.other]"));
    }

    #[test]
    fn test_gentle_shell_adds_package_and_mcp() {
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".gentle-shell/agent")).unwrap();
        std::fs::write(
            home.path().join(".gentle-shell/agent/settings.json"),
            r#"{"tuiMode":"fullscreen","packages":["npm:other"]}"#,
        )
        .unwrap();

        assert_eq!(
            status(Agent::GentleShell, home.path(), &bin()).unwrap(),
            Status::NotConfigured
        );

        install(
            Agent::GentleShell,
            home.path(),
            &bin(),
            PiPackageAction::Declare,
        )
        .unwrap();

        // 1. the pi package is declared
        let settings = read_json(&home.path().join(".gentle-shell/agent/settings.json")).unwrap();
        assert_eq!(settings["tuiMode"], "fullscreen");
        let packages = settings["packages"].as_array().unwrap();
        assert!(
            packages.iter().any(|v| v == "npm:other"),
            "existing package kept"
        );
        assert!(packages.iter().any(|v| v == PI_PACKAGE));

        // 2. the MCP server is registered in pi's own file
        let mcp = read_json(&home.path().join(".gentle-shell/agent/mcp.json")).unwrap();
        assert_eq!(
            mcp["mcpServers"][SERVER_KEY]["command"],
            "/usr/local/bin/cogito"
        );
        assert_eq!(mcp["mcpServers"][SERVER_KEY]["args"][0], "mcp");

        // Both halves present -> configured
        assert_eq!(
            status(Agent::GentleShell, home.path(), &bin()).unwrap(),
            Status::Configured
        );

        uninstall(Agent::GentleShell, home.path()).unwrap();
        let settings = read_json(&home.path().join(".gentle-shell/agent/settings.json")).unwrap();
        let packages = settings["packages"].as_array().unwrap();
        assert!(packages.iter().any(|v| v == "npm:other"));
        assert!(!packages.iter().any(|v| v == PI_PACKAGE));
        let mcp = read_json(&home.path().join(".gentle-shell/agent/mcp.json")).unwrap();
        assert!(mcp["mcpServers"][SERVER_KEY].is_null());
    }

    #[test]
    fn test_opencode_and_crush_use_mcp_section() {
        for (agent, dir) in [
            (Agent::Opencode, ".config/opencode/opencode.json"),
            (Agent::Crush, ".config/crush/crush.json"),
        ] {
            let home = TempDir::new().unwrap();
            let path = home.path().join(dir);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, r#"{"$schema":"x"}"#).unwrap();

            install(agent, home.path(), &bin(), PiPackageAction::Leave).unwrap();
            let value = read_json(&path).unwrap();
            assert_eq!(value["$schema"], "x", "existing keys survive");
            assert!(
                value["mcp"]["cogito"].is_object(),
                "{agent:?} gets mcp.cogito"
            );

            uninstall(agent, home.path()).unwrap();
            let value = read_json(&path).unwrap();
            assert!(value["mcp"]["cogito"].is_null());
        }
    }

    #[test]
    fn test_gemini_uses_mcpservers() {
        let home = TempDir::new().unwrap();
        let path = home.path().join(".gemini/settings.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"security":{}}"#).unwrap();

        install(Agent::Gemini, home.path(), &bin(), PiPackageAction::Leave).unwrap();
        let value = read_json(&path).unwrap();
        assert_eq!(value["security"], json!({}));
        assert_eq!(
            value["mcpServers"]["cogito"]["command"],
            "/usr/local/bin/cogito"
        );
    }

    #[test]
    fn test_claude_install_writes_hook_script_and_registers_it() {
        let home = TempDir::new().unwrap();
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"numStartups":1,"mcpServers":{}}"#,
        )
        .unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(
            home.path().join(".claude/settings.json"),
            r#"{"model":"opus","hooks":{"SessionStart":[{"matcher":"*","hooks":[{"type":"command","command":"bash other.sh"}]}]}}"#,
        )
        .unwrap();

        // Nothing configured yet, and the hook script does not exist.
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::NotConfigured
        );

        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();

        // 1. The hook script exists and is executable.
        let script = home.path().join(".claude/hooks/cogito-session-start.sh");
        assert!(script.exists(), "hook script written");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&script).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "hook script must be executable");
        }

        // 2. It is registered in settings.json without disturbing the rest.
        let settings = read_json(&home.path().join(".claude/settings.json")).unwrap();
        assert_eq!(settings["model"], "opus");
        let session_start = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(session_start.len(), 2, "existing hook kept, cogito added");
        assert!(
            session_start
                .iter()
                .any(|e| e["hooks"][0]["command"] == "bash other.sh"),
            "pre-existing hook preserved"
        );
        assert!(
            session_start.iter().any(|e| e["hooks"][0]["command"]
                .as_str()
                .is_some_and(|c| c.contains("cogito-session-start"))),
            "cogito hook registered"
        );

        // Both halves present -> configured
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Configured
        );

        // Idempotent: installing again must not duplicate the hook
        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();
        let settings = read_json(&home.path().join(".claude/settings.json")).unwrap();
        assert_eq!(
            settings["hooks"]["SessionStart"].as_array().unwrap().len(),
            2,
            "no duplicate hook"
        );

        // Uninstall removes the hook and the script, keeps the other hook
        uninstall(Agent::ClaudeCode, home.path()).unwrap();
        assert!(!script.exists(), "hook script removed");
        let settings = read_json(&home.path().join(".claude/settings.json")).unwrap();
        let session_start = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(session_start.len(), 1);
        assert_eq!(session_start[0]["hooks"][0]["command"], "bash other.sh");
    }

    #[test]
    fn test_opencode_install_writes_plugin() {
        let home = TempDir::new().unwrap();
        let cfg = home.path().join(".config/opencode/opencode.json");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, r#"{"$schema":"x"}"#).unwrap();

        install(Agent::Opencode, home.path(), &bin(), PiPackageAction::Leave).unwrap();

        let plugin = home.path().join(".config/opencode/plugins/cogito.js");
        assert!(plugin.exists(), "opencode plugin written");
        let source = std::fs::read_to_string(&plugin).unwrap();
        assert!(source.contains("CogitoPlugin"), "{source}");
        assert!(source.contains("cogito scan"), "{source}");

        // Defect 1 fix: injectCard must push into output.system, not a dangling variable.
        assert!(
            source.contains("output.system.push"),
            "plugin must push the card into output.system: {source}"
        );

        // Defect 2 fix: guardrail hooks must use opencode's (input, output) signature.
        // The correct shape reads input.tool and output.args; Claude Code's
        // tool_name / tool_input must not appear anywhere.
        assert!(
            source.contains("input.tool"),
            "tool.execute.before must read input.tool: {source}"
        );
        assert!(
            source.contains("output.args"),
            "tool.execute.before must read output.args: {source}"
        );
        assert!(
            !source.contains("tool_name"),
            "plugin must not use Claude Code's tool_name: {source}"
        );
        assert!(
            !source.contains("tool_input"),
            "plugin must not use Claude Code's tool_input: {source}"
        );

        assert_eq!(
            status(Agent::Opencode, home.path(), &bin()).unwrap(),
            Status::Configured
        );

        uninstall(Agent::Opencode, home.path()).unwrap();
        assert!(!plugin.exists(), "plugin removed on uninstall");
    }

    #[test]
    fn test_status_detects_a_missing_companion_file() {
        let home = TempDir::new().unwrap();
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();
        std::fs::write(
            home.path().join(".claude/settings.json"),
            r#"{"hooks":{"SessionStart":[{"hooks":[{"command":"sh cogito-session-start.sh"}]}]}}"#,
        )
        .unwrap();

        // MCP entry present but the plugin file is not -> not configured yet.
        assert_eq!(
            status(Agent::Opencode, home.path(), &bin()).unwrap(),
            Status::AgentMissing
        );
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::NotConfigured
        );
    }

    #[test]
    fn test_foreign_hook_mentioning_cogito_is_not_claimed_or_deleted() {
        let home = TempDir::new().unwrap();
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
        std::fs::create_dir_all(home.path().join(".claude")).unwrap();

        // A user hook that merely mentions "cogito" in its path — for example a
        // script living in a checkout at ~/proyectos/cogito/. Substring matching
        // would wrongly treat this as Cogito's own hook.
        let foreign = "sh /home/me/proyectos/cogito/scripts/my-hook.sh";
        std::fs::write(
            home.path().join(".claude/settings.json"),
            format!(
                r#"{{"hooks":{{"SessionStart":[{{"matcher":"*","hooks":[{{"type":"command","command":"{foreign}"}}]}}]}}}}"#
            ),
        )
        .unwrap();

        // Install must ADD ours, not consider the foreign one sufficient.
        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();
        let settings = read_json(&home.path().join(".claude/settings.json")).unwrap();
        let entries = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(
            entries.len(),
            2,
            "cogito hook must be added alongside: {settings}"
        );
        assert!(
            entries.iter().any(|e| {
                e["hooks"][0]["command"]
                    .as_str()
                    .is_some_and(|c| c.contains("cogito-session-start.sh"))
            }),
            "our hook present"
        );

        // Uninstall must remove only ours.
        uninstall(Agent::ClaudeCode, home.path()).unwrap();
        let settings = read_json(&home.path().join(".claude/settings.json")).unwrap();
        let entries = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "foreign hook must survive: {settings}");
        assert_eq!(entries[0]["hooks"][0]["command"].as_str().unwrap(), foreign);
    }

    #[test]
    fn test_pi_mcp_entry_uses_direct_exposure_without_invented_fields() {
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".gentle-shell/agent")).unwrap();
        std::fs::write(home.path().join(".gentle-shell/agent/settings.json"), "{}").unwrap();

        install(
            Agent::GentleShell,
            home.path(),
            &bin(),
            PiPackageAction::Declare,
        )
        .unwrap();

        let mcp = read_json(&home.path().join(".gentle-shell/agent/mcp.json")).unwrap();
        let entry = &mcp["mcpServers"][SERVER_KEY];

        // Gentle Shell ships `-builtin:codemode`; under Pi's default `codemode`
        // exposure the model could never reach these tools.
        assert_eq!(
            entry["exposure"], "direct",
            "must use direct exposure so the tools reach the model: {entry}"
        );
        assert!(
            entry["description"].is_string(),
            "description is used by tool search"
        );
        // `lifecycle` is an Engram field; Pi ignores it.
        assert!(
            entry.get("lifecycle").is_none(),
            "must not invent Pi fields"
        );
    }

    #[test]
    fn test_json_key_order_is_preserved() {
        let home = TempDir::new().unwrap();
        // Deliberately not alphabetical, as a real ~/.claude.json is.
        std::fs::write(
            home.path().join(".claude.json"),
            r#"{"zeta":1,"alpha":2,"middle":{"b":1,"a":2}}"#,
        )
        .unwrap();

        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();

        let text = std::fs::read_to_string(home.path().join(".claude.json")).unwrap();
        let zeta = text.find("\"zeta\"").expect("zeta present");
        let alpha = text.find("\"alpha\"").expect("alpha present");
        assert!(zeta < alpha, "key order must be preserved:\n{text}");

        let b = text.find("\"b\"").expect("b present");
        let a = text.find("\"a\"").expect("a present");
        assert!(b < a, "nested key order must be preserved:\n{text}");
    }

    #[test]
    fn test_existing_but_stale_entry_is_reported_and_repaired() {
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".gentle-shell/agent")).unwrap();
        std::fs::write(
            home.path().join(".gentle-shell/agent/settings.json"),
            format!(r#"{{"packages":["{PI_PACKAGE}"]}}"#),
        )
        .unwrap();
        // An entry written by an older version: no exposure, and the Engram
        // field that Pi ignores.
        std::fs::write(
            home.path().join(".gentle-shell/agent/mcp.json"),
            r#"{"mcpServers":{"cogito":{"command":"/old/cogito","args":["mcp"],"lifecycle":"lazy"}}}"#,
        )
        .unwrap();

        assert_eq!(
            status(Agent::GentleShell, home.path(), &bin()).unwrap(),
            Status::Outdated,
            "a stale entry must not be reported as configured"
        );

        let change = install(
            Agent::GentleShell,
            home.path(),
            &bin(),
            PiPackageAction::Declare,
        )
        .unwrap();
        assert!(
            matches!(change, Change::Written { .. }),
            "install must repair a stale entry"
        );

        let mcp = read_json(&home.path().join(".gentle-shell/agent/mcp.json")).unwrap();
        let entry = &mcp["mcpServers"][SERVER_KEY];
        assert_eq!(entry["command"], "/usr/local/bin/cogito", "path refreshed");
        assert_eq!(entry["exposure"], "direct", "missing field added");
        assert!(entry.get("lifecycle").is_none(), "stale field removed");
        assert_eq!(
            status(Agent::GentleShell, home.path(), &bin()).unwrap(),
            Status::Configured
        );
    }

    #[test]
    fn test_user_choices_in_the_entry_survive_status_and_repair() {
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".gentle-shell/agent")).unwrap();
        std::fs::write(
            home.path().join(".gentle-shell/agent/settings.json"),
            format!(r#"{{"packages":["{PI_PACKAGE}"]}}"#),
        )
        .unwrap();
        install(
            Agent::GentleShell,
            home.path(),
            &bin(),
            PiPackageAction::Declare,
        )
        .unwrap();

        // What Pi's `/mcp` writes when the user turns the server off and picks
        // another exposure, plus a field of their own.
        let mcp_path = home.path().join(".gentle-shell/agent/mcp.json");
        let mut mcp = read_json(&mcp_path).unwrap();
        let entry = mcp["mcpServers"][SERVER_KEY].as_object_mut().unwrap();
        entry.insert("enabled".into(), json!(false));
        entry.insert("exposure".into(), json!("deferred"));
        entry.insert("timeout".into(), json!(120));
        write_json(&mcp_path, &mcp).unwrap();

        assert_eq!(
            status(Agent::GentleShell, home.path(), &bin()).unwrap(),
            Status::Configured,
            "user choices are not staleness"
        );

        // A moved binary makes it stale; the repair must keep the choices.
        let other = PathBuf::from("/opt/other/cogito");
        assert_eq!(
            status(Agent::GentleShell, home.path(), &other).unwrap(),
            Status::Outdated
        );
        install(
            Agent::GentleShell,
            home.path(),
            &other,
            PiPackageAction::Leave,
        )
        .unwrap();
        let entry = &read_json(&mcp_path).unwrap()["mcpServers"][SERVER_KEY];
        assert_eq!(entry["command"], "/opt/other/cogito");
        assert_eq!(entry["enabled"], false);
        assert_eq!(entry["exposure"], "deferred");
        assert_eq!(entry["timeout"], 120);
    }

    #[test]
    fn test_codex_repair_keeps_user_fields() {
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::fs::write(
            home.path().join(".codex/config.toml"),
            "[mcp_servers.cogito]\ncommand = \"/old/cogito\"\nargs = [\"mcp\"]\nenabled = false\nstartup_timeout_sec = 30\n",
        )
        .unwrap();

        assert_eq!(
            status(Agent::Codex, home.path(), &bin()).unwrap(),
            Status::Outdated
        );
        install(Agent::Codex, home.path(), &bin(), PiPackageAction::Leave).unwrap();

        let text = std::fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();
        assert!(text.contains("/usr/local/bin/cogito"), "{text}");
        assert!(text.contains("enabled = false"), "{text}");
        assert!(text.contains("startup_timeout_sec = 30"), "{text}");
        assert_eq!(
            status(Agent::Codex, home.path(), &bin()).unwrap(),
            Status::Configured
        );
    }

    #[test]
    fn test_missing_claude_hook_entry_is_outdated_and_restored() {
        let home = TempDir::new().unwrap();
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();

        // The script is still on disk, but settings no longer run it.
        let settings_path = home.path().join(".claude/settings.json");
        std::fs::write(&settings_path, "{}").unwrap();

        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Outdated,
            "a hook script nothing runs is not configured"
        );

        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();
        let script = Agent::ClaudeCode.hook_script_path(home.path()).unwrap();
        let settings = read_json(&settings_path).unwrap();
        assert!(
            settings["hooks"]["SessionStart"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| entry_owns_script(e, &script)),
            "hook entry restored: {settings}"
        );
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Configured
        );
    }

    #[test]
    fn test_repairing_the_hook_leaves_claude_json_untouched() {
        let home = TempDir::new().unwrap();
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();
        let claude_json = home.path().join(".claude.json");
        let before = std::fs::metadata(&claude_json).unwrap().modified().unwrap();

        // Only the hook script is stale; the MCP entry is current.
        std::fs::write(
            Agent::ClaudeCode.hook_script_path(home.path()).unwrap(),
            "old",
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();

        let after = std::fs::metadata(&claude_json).unwrap().modified().unwrap();
        assert_eq!(
            before, after,
            "an unchanged ~/.claude.json must not be rewritten"
        );
    }

    #[test]
    fn test_claude_gets_the_guardrail_hook_and_loses_both_on_uninstall() {
        let home = TempDir::new().unwrap();
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();

        let settings_path = home.path().join(".claude/settings.json");
        let settings = read_json(&settings_path).unwrap();
        let pre = &settings["hooks"]["PreToolUse"][0];
        assert_eq!(pre["matcher"], "^(Read|Grep)$");
        assert!(
            pre["hooks"][0]["command"]
                .as_str()
                .unwrap()
                .contains("cogito-pre-tool-use.sh")
        );
        assert!(
            home.path()
                .join(".claude/hooks/cogito-pre-tool-use.sh")
                .exists()
        );
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Configured
        );

        // Missing only the guardrail entry: outdated, then repaired.
        let mut partial = settings.clone();
        partial["hooks"]
            .as_object_mut()
            .unwrap()
            .remove("PreToolUse");
        write_json(&settings_path, &partial).unwrap();
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Outdated
        );
        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Configured
        );

        uninstall(Agent::ClaudeCode, home.path()).unwrap();
        let settings = read_json(&settings_path).unwrap();
        assert_eq!(settings["hooks"]["SessionStart"], json!([]));
        assert_eq!(settings["hooks"]["PreToolUse"], json!([]));
        assert!(
            !home
                .path()
                .join(".claude/hooks/cogito-pre-tool-use.sh")
                .exists()
        );
    }

    #[test]
    fn test_async_hook_entry_from_an_older_version_is_repaired() {
        let home = TempDir::new().unwrap();
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();

        // What earlier versions wrote: the same script, but `async`, whose
        // output Claude Code never adds to the session.
        let settings_path = home.path().join(".claude/settings.json");
        let mut settings = read_json(&settings_path).unwrap();
        settings["hooks"]["SessionStart"][0]["hooks"][0]["async"] = json!(true);
        settings["hooks"]["SessionStart"][0]["hooks"][0]["timeout"] = json!(10);
        write_json(&settings_path, &settings).unwrap();

        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Outdated
        );
        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();

        let settings = read_json(&settings_path).unwrap();
        let entries = settings["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "repaired in place, not duplicated");
        let hook = &entries[0]["hooks"][0];
        assert!(hook.get("async").is_none(), "{hook}");
        assert_eq!(hook["timeout"], HOOK_TIMEOUT_SECS);
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Configured
        );
    }

    #[test]
    fn test_stale_hook_script_is_detected_and_rewritten() {
        let home = TempDir::new().unwrap();
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();
        std::fs::create_dir_all(home.path().join(".claude/hooks")).unwrap();

        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Configured
        );

        // Simulate a script left behind by an older version.
        let script = home.path().join(".claude/hooks/cogito-session-start.sh");
        std::fs::write(&script, "#!/bin/sh\n# old version\n").unwrap();

        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Outdated,
            "an out-of-date hook script must be reported"
        );

        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();
        let rewritten = std::fs::read_to_string(&script).unwrap();
        assert!(
            rewritten.contains("-e .git"),
            "script refreshed: {rewritten}"
        );
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Configured
        );
    }

    #[test]
    fn test_changed_binary_path_is_reported_outdated() {
        let home = TempDir::new().unwrap();
        std::fs::write(home.path().join(".claude.json"), "{}").unwrap();

        install(
            Agent::ClaudeCode,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Status::Configured
        );

        // Same file, different binary: the entry no longer matches.
        let other = PathBuf::from("/opt/other/cogito");
        assert_eq!(
            status(Agent::ClaudeCode, home.path(), &other).unwrap(),
            Status::Outdated
        );
    }

    #[test]
    fn test_hook_script_detects_worktrees_and_submodules() {
        // `.git` is a file in a worktree or submodule, so the guard must use
        // `-e`, not `-d`.
        let script = CLAUDE_HOOK_SCRIPT;
        assert!(
            script.contains("[ -e .git ]"),
            "hook must accept .git as a file:\n{script}"
        );
        assert!(
            !script.contains("[ -d .git ]"),
            "a directory test would reject worktrees"
        );

        let plugin = OPENCODE_PLUGIN;
        assert!(
            plugin.contains("test -e .git"),
            "opencode plugin must accept .git as a file:\n{plugin}"
        );
        assert!(!plugin.contains("test -d .git"));
    }

    #[test]
    fn test_pi_package_is_not_declared_by_default() {
        // Pi runs `npm install` for every declared package, and a 404 aborts
        // package resolution. Declaring an unpublished name would stop the
        // agent from starting, so the MCP half is configured on its own.
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".gentle-shell/agent")).unwrap();
        std::fs::write(
            home.path().join(".gentle-shell/agent/settings.json"),
            r#"{"tuiMode":"fullscreen","packages":["npm:other"]}"#,
        )
        .unwrap();

        install(
            Agent::GentleShell,
            home.path(),
            &bin(),
            PiPackageAction::Remove,
        )
        .unwrap();

        let settings = read_json(&home.path().join(".gentle-shell/agent/settings.json")).unwrap();
        let packages = settings["packages"].as_array().unwrap();
        assert!(packages.iter().any(|v| v == "npm:other"), "existing kept");
        assert!(
            !packages.iter().any(|v| v == PI_PACKAGE),
            "must not declare an unverified package: {settings}"
        );

        // The MCP half still lands, so the tools work.
        let mcp = read_json(&home.path().join(".gentle-shell/agent/mcp.json")).unwrap();
        assert_eq!(mcp["mcpServers"][SERVER_KEY]["exposure"], "direct");

        // And the agent must read as configured: the package is optional.
        assert_eq!(
            status(Agent::GentleShell, home.path(), &bin()).unwrap(),
            Status::Configured
        );
        assert!(!pi_package_declared(home.path()));
    }

    #[test]
    fn test_pi_package_declarable_on_request() {
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".gentle-shell/agent")).unwrap();
        std::fs::write(home.path().join(".gentle-shell/agent/settings.json"), "{}").unwrap();

        install(
            Agent::GentleShell,
            home.path(),
            &bin(),
            PiPackageAction::Declare,
        )
        .unwrap();
        assert!(pi_package_declared(home.path()));

        // Repairing with the package off must remove it again, otherwise a
        // bad declaration could never be backed out.
        install(
            Agent::GentleShell,
            home.path(),
            &bin(),
            PiPackageAction::Remove,
        )
        .unwrap();
        assert!(
            !pi_package_declared(home.path()),
            "package must be removable by re-running setup without it"
        );
    }

    #[test]
    fn test_classify_npm_view_only_e404_means_not_found() {
        assert_eq!(classify_npm_view(true, ""), NpmCheck::Published);
        assert_eq!(
            classify_npm_view(false, "npm error code E404\nnpm error 404 Not Found"),
            NpmCheck::NotFound
        );
        // Older npm wording.
        assert_eq!(
            classify_npm_view(false, "npm ERR! code E404\n"),
            NpmCheck::NotFound
        );
        // A network failure exits non-zero too, but proves nothing about the
        // package: it must never read as "not found".
        for code in ["ECONNREFUSED", "ENOTFOUND", "ETIMEDOUT", "E401"] {
            let stderr = format!("npm error code {code}\nnpm error network request failed");
            assert!(
                matches!(classify_npm_view(false, &stderr), NpmCheck::Unavailable(_)),
                "{code} must be Unavailable"
            );
        }
        assert!(matches!(
            classify_npm_view(false, "something went wrong"),
            NpmCheck::Unavailable(_)
        ));
    }

    #[test]
    fn test_unreachable_registry_is_unavailable_not_not_found() {
        // Nothing listens on port 9, so npm fails fast with ECONNREFUSED. If npm
        // is not installed the result is Unavailable as well.
        let check = npm_check_with_registry("commander", Some("http://127.0.0.1:9"));
        assert!(
            matches!(check, NpmCheck::Unavailable(_)),
            "an unreachable registry must not read as not found, got {check:?}"
        );
    }

    #[test]
    fn test_unavailable_registry_keeps_an_existing_declaration() {
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".gentle-shell/agent")).unwrap();
        std::fs::write(
            home.path().join(".gentle-shell/agent/settings.json"),
            format!(r#"{{"packages":["npm:other","{PI_PACKAGE}"]}}"#),
        )
        .unwrap();

        // What setup does when the registry cannot be reached.
        install(
            Agent::GentleShell,
            home.path(),
            &bin(),
            PiPackageAction::Leave,
        )
        .unwrap();
        assert!(
            pi_package_declared(home.path()),
            "an unreachable registry must not remove the declaration"
        );
    }

    #[test]
    fn test_npm_check_reports_missing_package_as_not_found() {
        // A name that cannot exist must not read as published.
        let check = npm_check("cogito-harness-definitely-not-published-xyz");
        assert!(
            !matches!(check, NpmCheck::Published),
            "an unpublished package must not read as published, got {check:?}"
        );
    }

    #[test]
    fn test_missing_agent_reports_not_installed() {
        let home = TempDir::new().unwrap();
        assert_eq!(
            status(Agent::Crush, home.path(), &bin()).unwrap(),
            Status::AgentMissing
        );
    }

    #[test]
    fn test_install_creates_missing_parent_dirs() {
        let home = TempDir::new().unwrap();
        // No .config/crush at all; agent_present() is false but install must still work
        let change = install(Agent::Crush, home.path(), &bin(), PiPackageAction::Leave).unwrap();
        assert!(matches!(change, Change::Written { .. }));
        assert!(home.path().join(".config/crush/crush.json").exists());
    }

    // ============================================================
    // AGENTS.md block management
    // ============================================================

    #[test]
    fn test_agents_md_install_into_empty_dir_creates_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        assert!(!path.exists());

        let result = install_agents_md_block(dir.path(), false).unwrap();
        assert!(matches!(result, Change::Written { .. }));
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("<!-- cogito:start -->"));
        assert!(content.contains("<!-- cogito:end -->"));
        assert!(content.contains("project_map"));
        assert!(content.contains("search_code"));
        assert!(content.contains("file_outline"));
    }

    #[test]
    fn test_agents_md_install_preserves_existing_prose() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, "Existing project notes.\n").unwrap();

        install_agents_md_block(dir.path(), false).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Existing project notes."));
        assert!(content.contains("<!-- cogito:start -->"));
        assert!(content.contains("<!-- cogito:end -->"));
    }

    #[test]
    fn test_agents_md_install_twice_is_byte_identical() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, "Pre-existing content.\n").unwrap();

        install_agents_md_block(dir.path(), false).unwrap();
        let first = std::fs::read(&path).unwrap();

        install_agents_md_block(dir.path(), false).unwrap();
        let second = std::fs::read(&path).unwrap();

        assert_eq!(first, second, "installing twice must be byte-identical");
    }

    #[test]
    fn test_agents_md_replace_stale_block_preserves_rest() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(
            &path,
            "Before text.\n<!-- cogito:start -->Old content<!-- cogito:end -->\nAfter text.\n",
        )
        .unwrap();

        install_agents_md_block(dir.path(), false).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Before text."));
        assert!(content.contains("After text."));
        assert!(content.contains("<!-- cogito:start -->"));
        assert!(content.contains("<!-- cogito:end -->"));
        // Old content replaced
        assert!(!content.contains("Old content"));
        // New content present
        assert!(content.contains("project_map"));
    }

    #[test]
    fn test_agents_md_remove_restores_original() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(
            &path,
            "Before text.\n<!-- cogito:start -->Old content<!-- cogito:end -->\nAfter text.\n",
        )
        .unwrap();

        remove_agents_md_block(dir.path()).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Before text."));
        assert!(content.contains("After text."));
        assert!(!content.contains("<!-- cogito:start -->"));
        assert!(!content.contains("<!-- cogito:end -->"));
        assert!(!content.contains("Old content"));
    }

    #[test]
    fn test_agents_md_remove_no_block_is_noop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        std::fs::write(&path, "No block here.\n").unwrap();

        let result = remove_agents_md_block(dir.path()).unwrap();
        assert!(matches!(result, Change::AlreadyCorrect));

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "No block here.\n");
    }

    #[test]
    fn test_agents_md_remove_nonexistent_file_is_noop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");
        assert!(!path.exists());

        let result = remove_agents_md_block(dir.path()).unwrap();
        assert!(matches!(result, Change::AlreadyCorrect));
        assert!(!path.exists(), "must not create the file");
    }

    #[test]
    fn test_agents_md_dry_run_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("AGENTS.md");

        let result = install_agents_md_block(dir.path(), true).unwrap();
        assert!(matches!(result, Change::AlreadyCorrect));
        assert!(!path.exists(), "dry run must not create the file");
    }

    // ============================================================
    // opencode plugin
    // ============================================================

    #[test]
    fn test_opencode_plugin_loaded_from_file() {
        // Verify the plugin constant is loaded from the real file.
        let plugin = OPENCODE_PLUGIN;
        assert!(!plugin.is_empty(), "plugin must be loaded from asset file");
        assert!(plugin.contains("CogitoPlugin"));
    }

    #[test]
    fn test_opencode_plugin_has_session_scan() {
        let plugin = OPENCODE_PLUGIN;
        assert!(
            plugin.contains("session.created") || plugin.contains("session.start"),
            "plugin must handle session lifecycle events"
        );
        assert!(plugin.contains("cogito scan"));
    }

    #[test]
    fn test_opencode_plugin_has_card_injection() {
        let plugin = OPENCODE_PLUGIN;
        assert!(
            plugin.contains("experimental.chat.system.transform"),
            "plugin must have card injection hook"
        );
        assert!(plugin.contains("cogito map --card") || plugin.contains("map --card"));
    }

    #[test]
    fn test_opencode_plugin_has_read_guardrail() {
        let plugin = OPENCODE_PLUGIN;
        assert!(
            plugin.contains("tool.execute.before"),
            "plugin must have guardrail hook"
        );
        assert!(plugin.contains("cogito outline") || plugin.contains("outline"));
    }

    #[test]
    fn test_opencode_plugin_has_grep_guardrail() {
        let plugin = OPENCODE_PLUGIN;
        assert!(plugin.contains("cogito search") || plugin.contains("search"));
        // Should check for plain name patterns
        assert!(plugin.contains("isPlainName") || plugin.contains("is_plain_name"));
    }

    #[test]
    fn test_opencode_plugin_repeat_passes_through() {
        let plugin = OPENCODE_PLUGIN;
        // Should have per-session bookkeeping for "already nudged"
        assert!(
            plugin.contains("firstTime") || plugin.contains("first_time"),
            "plugin must track already-nudged state per session"
        );
    }

    #[test]
    fn test_reads_agents_md() {
        assert!(reads_agents_md(Agent::Codex));
        assert!(reads_agents_md(Agent::Opencode));
        assert!(!reads_agents_md(Agent::ClaudeCode));
        assert!(!reads_agents_md(Agent::GentleShell));
        assert!(!reads_agents_md(Agent::Gemini));
        assert!(!reads_agents_md(Agent::Crush));
    }
}
