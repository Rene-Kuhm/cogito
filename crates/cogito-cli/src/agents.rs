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
//! | Gentle Shell / pi | `~/.gentle-shell/agent/settings.json` | `packages: ["npm:pi-cogito"]` |
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
pub const PI_PACKAGE: &str = "npm:pi-cogito";

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
# Warms the project fingerprint so the first query in the session is fast.
#
# This script always exits 0. A missing `cogito`, a directory that is not a
# project, or a failed scan are all silent no-ops, so it can never block or
# fail a session.

command -v cogito >/dev/null 2>&1 || exit 0

# Only touch directories that already look like a project. Scanning
# unconditionally would create a `.cogito/` in $HOME, or in any unrelated tree
# an agent happens to start in, and walk the whole thing.
[ -d .git ] || [ -d .cogito ] || exit 0

# Bounded, so a huge repository cannot leave a scan running forever.
# `nohup` survives the SIGHUP this shell gets when it exits.
if command -v timeout >/dev/null 2>&1; then
  nohup timeout 60 cogito scan --quiet >/dev/null 2>&1 &
else
  nohup cogito scan --quiet >/dev/null 2>&1 &
fi

exit 0
"#;

/// The opencode plugin.
const OPENCODE_PLUGIN: &str = r#"// Cogito plugin for opencode.
//
// Warms the project fingerprint when a session starts so the first query is
// fast, and exposes `/cogito` through the plugin's command surface.
//
// The MCP tools come from the `mcp.cogito` entry in opencode.json; this
// plugin only reacts to session lifecycle events.

export const CogitoPlugin = async ({ directory, $ }) => {
  const scan = async () => {
    try {
      // Only touch real projects. Scanning unconditionally would create a
      // `.cogito/` in $HOME or in any tree opencode happens to start in.
      const isProject =
        (await $`test -d .git`.cwd(directory).quiet().nothrow()).exitCode === 0 ||
        (await $`test -d .cogito`.cwd(directory).quiet().nothrow()).exitCode === 0;
      if (!isProject) return;

      await $`cogito scan --quiet`.cwd(directory).quiet().nothrow();
    } catch {
      // Cogito missing or the scan failed: never block the session.
    }
  };

  return {
    event: async ({ event }) => {
      if (event?.type === "session.created" || event?.type === "session.start") {
        // Fire and forget.
        void scan();
      }
    },
  };
};

export default CogitoPlugin;
"#;

impl Agent {
    /// Files Cogito writes besides the agent's main configuration file.
    pub fn extra_files(self, home: &Path, _binary: &Path) -> Vec<ExtraFile> {
        match self {
            Agent::ClaudeCode => vec![ExtraFile {
                path: self
                    .hook_script_path(home)
                    .expect("ClaudeCode has a hook script path"),
                content: CLAUDE_HOOK_SCRIPT.to_string(),
                executable: true,
            }],
            Agent::Opencode => vec![ExtraFile {
                path: home.join(".config/opencode/plugins/cogito.js"),
                content: OPENCODE_PLUGIN.to_string(),
                executable: false,
            }],
            _ => Vec::new(),
        }
    }

    /// Whether this agent supports a session-start hook in its config file.
    fn has_session_hook(self) -> bool {
        matches!(self, Agent::ClaudeCode)
    }

    /// Canonical path of this agent's session-hook script.
    ///
    /// Independent of the installed binary so that detecting, writing and
    /// removing the hook entry always agree on the same file.
    pub fn hook_script_path(self, home: &Path) -> Option<PathBuf> {
        match self {
            Agent::ClaudeCode => Some(home.join(".claude/hooks/cogito-session-start.sh")),
            _ => None,
        }
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
    /// Cogito is configured for this agent.
    Configured,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::AgentMissing => "not installed",
            Status::NotConfigured => "installed, cogito missing",
            Status::Configured => "cogito configured",
        }
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

/// Inspect whether Cogito is configured for `agent`.
pub fn status(agent: Agent, home: &Path) -> Result<Status> {
    if !agent.agent_present(home) {
        return Ok(Status::AgentMissing);
    }
    let path = agent.config_path(home);
    if !path.exists() {
        return Ok(Status::NotConfigured);
    }

    if agent.uses_pi_package() {
        let value = read_json(&path)?;
        let package_ok = value
            .get("packages")
            .and_then(|p| p.as_array())
            .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(PI_PACKAGE)));

        let mcp_ok = match agent.mcp_config_path(home) {
            Some(mcp_path) if mcp_path.exists() => read_json(&mcp_path)?
                .get("mcpServers")
                .and_then(|m| m.get(SERVER_KEY))
                .is_some(),
            _ => false,
        };

        return Ok(if package_ok && mcp_ok {
            Status::Configured
        } else {
            Status::NotConfigured
        });
    }

    if agent == Agent::Codex {
        let text = std::fs::read_to_string(&path)?;
        let doc: toml_edit::DocumentMut = text.parse()?;
        let present = doc
            .get("mcp_servers")
            .and_then(|t| t.get(SERVER_KEY))
            .is_some();
        return Ok(if present {
            Status::Configured
        } else {
            Status::NotConfigured
        });
    }

    let value = read_json(&path)?;
    let present = match agent {
        Agent::Opencode | Agent::Crush => {
            value.get("mcp").and_then(|m| m.get(SERVER_KEY)).is_some()
        }
        _ => value
            .get("mcpServers")
            .and_then(|m| m.get(SERVER_KEY))
            .is_some(),
    };
    Ok(if !present {
        Status::NotConfigured
    } else if !agent
        .extra_files(home, Path::new("cogito"))
        .iter()
        .all(|f| f.path.exists())
    {
        // Companion artifacts (hook script, plugin) are part of the install;
        // without them the harness half would silently be missing.
        Status::NotConfigured
    } else {
        Status::Configured
    })
}

/// Configure `agent` to use the `cogito` binary at `binary`.
pub fn install(agent: Agent, home: &Path, binary: &Path) -> Result<Change> {
    if status(agent, home)? == Status::Configured {
        return Ok(Change::AlreadyCorrect);
    }

    let path = agent.config_path(home);
    let backup = backup_file(&path)?;
    let bin = binary.display().to_string();

    if agent.uses_pi_package() {
        // (1) Declare the pi package that provides the session adapter.
        let mut value = read_json_or_default(&path);
        let packages = value
            .as_object_mut()
            .context("settings.json is not an object")?
            .entry("packages")
            .or_insert_with(|| json!([]));
        let arr = packages
            .as_array_mut()
            .context("`packages` is not an array")?;
        if !arr.iter().any(|v| v.as_str() == Some(PI_PACKAGE)) {
            arr.push(json!(PI_PACKAGE));
        }
        write_json(&path, &value)?;

        // (2) Register the MCP server. Pi reads MCP servers from its own file.
        if let Some(mcp_path) = agent.mcp_config_path(home) {
            let _ = backup_file(&mcp_path)?;
            let mut mcp = read_json_or_default(&mcp_path);
            let obj = mcp.as_object_mut().context("mcp.json is not an object")?;
            obj.entry("mcpServers")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .context("`mcpServers` is not an object")?
                .insert(
                    SERVER_KEY.to_string(),
                    json!({
                        "command": bin,
                        "args": ["mcp"],
                        // Pi defaults every MCP server to `codemode` exposure.
                        // Gentle Shell ships with `-builtin:codemode`, so under
                        // the default the model could never reach these tools.
                        // `direct` declares them like built-in tools, which is
                        // what the Pi docs recommend for a small, frequently
                        // used tool set.
                        "exposure": "direct",
                        "description": SERVER_DESCRIPTION,
                    }),
                );
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

        let mut server = toml_edit::Table::new();
        server["command"] = toml_edit::value(bin);
        server["args"] = toml_edit::value(toml_edit::Array::from_iter(["mcp"]));
        server["enabled"] = toml_edit::value(true);
        servers.insert(SERVER_KEY, toml_edit::Item::Table(server));

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, doc.to_string())?;
        return Ok(Change::Written { backup });
    }

    let mut value = read_json_or_default(&path);
    let (section, entry) = match agent {
        Agent::Opencode => (
            "mcp",
            json!({ "type": "local", "command": [bin, "mcp"], "enabled": true }),
        ),
        Agent::Crush => (
            "mcp",
            json!({ "type": "stdio", "command": bin, "args": ["mcp"] }),
        ),
        Agent::Gemini => ("mcpServers", json!({ "command": bin, "args": ["mcp"] })),
        // Claude Code wants an explicit transport type.
        _ => (
            "mcpServers",
            json!({ "type": "stdio", "command": bin, "args": ["mcp"], "env": {} }),
        ),
    };

    let obj = value.as_object_mut().context("config is not an object")?;
    obj.entry(section)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .with_context(|| format!("`{section}` is not an object"))?
        .insert(SERVER_KEY.to_string(), entry);

    write_json(&path, &value)?;

    // Companion artifacts: hook scripts and plugins that run without the model
    // asking. Written once the MCP entry is in place so a failure here leaves a
    // working (if less automatic) setup.
    write_extra_files(agent, home, binary)?;

    if agent.has_session_hook()
        && let Some(hook_path) = hook_config_path(agent, home)
        && let Some(hook_script) = agent.hook_script_path(home)
    {
        let _ = backup_file(&hook_path)?;
        let mut settings = read_json_or_default(&hook_path);
        upsert_session_hook(&mut settings, &hook_script)?;
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
        && let Some(hook_script) = agent.hook_script_path(home)
        && hook_path.exists()
    {
        let _ = backup_file(&hook_path)?;
        let mut settings = read_json(&hook_path)?;
        remove_session_hook(&mut settings, &hook_script);
        write_json(&hook_path, &settings)?;
    }

    remove_extra_files(agent, home, &PathBuf::from("cogito"))?;
    Ok(Change::Written { backup })
}

// ============================================================
// Helpers
// ============================================================

/// Insert a `SessionStart` hook into a Claude-Code-style settings object.
///
/// Idempotent: an entry whose command already mentions Cogito is left alone.
fn upsert_session_hook(value: &mut Value, script: &Path) -> Result<()> {
    let obj = value.as_object_mut().context("settings is not an object")?;
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("`hooks` is not an object")?;
    let session_start = hooks
        .entry("SessionStart")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .context("`hooks.SessionStart` is not an array")?;

    // Match on the exact script path. A substring match on "cogito" would
    // also claim an unrelated hook that merely mentions the word, for example
    // a user script living under a checkout at ~/proyectos/cogito/.
    if !session_start
        .iter()
        .any(|entry| entry_owns_script(entry, script))
    {
        session_start.push(json!({
            "matcher": "^(startup|resume|clear|compact|fork)$",
            "hooks": [{
                "type": "command",
                "command": format!("sh '{}'", script.display()),
                "timeout": 10,
                "async": true
            }]
        }));
    }
    Ok(())
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

/// Remove Cogito-owned entries from a Claude-Code-style `SessionStart` hook list.
fn remove_session_hook(value: &mut Value, script: &Path) {
    if let Some(arr) = value
        .get_mut("hooks")
        .and_then(|h| h.get_mut("SessionStart"))
        .and_then(|s| s.as_array_mut())
    {
        arr.retain(|entry| !entry_owns_script(entry, script));
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
            status(Agent::ClaudeCode, home.path()).unwrap(),
            Status::NotConfigured
        );

        let change = install(Agent::ClaudeCode, home.path(), &bin()).unwrap();
        assert!(matches!(change, Change::Written { backup: Some(_) }));
        assert_eq!(
            status(Agent::ClaudeCode, home.path()).unwrap(),
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
            install(Agent::ClaudeCode, home.path(), &bin()).unwrap(),
            Change::AlreadyCorrect
        ));

        uninstall(Agent::ClaudeCode, home.path()).unwrap();
        assert_eq!(
            status(Agent::ClaudeCode, home.path()).unwrap(),
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

        install(Agent::Codex, home.path(), &bin()).unwrap();
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
            status(Agent::Codex, home.path()).unwrap(),
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
            status(Agent::GentleShell, home.path()).unwrap(),
            Status::NotConfigured
        );

        install(Agent::GentleShell, home.path(), &bin()).unwrap();

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
            status(Agent::GentleShell, home.path()).unwrap(),
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

            install(agent, home.path(), &bin()).unwrap();
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

        install(Agent::Gemini, home.path(), &bin()).unwrap();
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
            status(Agent::ClaudeCode, home.path()).unwrap(),
            Status::NotConfigured
        );

        install(Agent::ClaudeCode, home.path(), &bin()).unwrap();

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
            status(Agent::ClaudeCode, home.path()).unwrap(),
            Status::Configured
        );

        // Idempotent: installing again must not duplicate the hook
        install(Agent::ClaudeCode, home.path(), &bin()).unwrap();
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

        install(Agent::Opencode, home.path(), &bin()).unwrap();

        let plugin = home.path().join(".config/opencode/plugins/cogito.js");
        assert!(plugin.exists(), "opencode plugin written");
        let source = std::fs::read_to_string(&plugin).unwrap();
        assert!(source.contains("CogitoPlugin"), "{source}");
        assert!(source.contains("cogito scan"), "{source}");

        assert_eq!(
            status(Agent::Opencode, home.path()).unwrap(),
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
            status(Agent::Opencode, home.path()).unwrap(),
            Status::AgentMissing
        );
        assert_eq!(
            status(Agent::ClaudeCode, home.path()).unwrap(),
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
        install(Agent::ClaudeCode, home.path(), &bin()).unwrap();
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

        install(Agent::GentleShell, home.path(), &bin()).unwrap();

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

        install(Agent::ClaudeCode, home.path(), &bin()).unwrap();

        let text = std::fs::read_to_string(home.path().join(".claude.json")).unwrap();
        let zeta = text.find("\"zeta\"").expect("zeta present");
        let alpha = text.find("\"alpha\"").expect("alpha present");
        assert!(zeta < alpha, "key order must be preserved:\n{text}");

        let b = text.find("\"b\"").expect("b present");
        let a = text.find("\"a\"").expect("a present");
        assert!(b < a, "nested key order must be preserved:\n{text}");
    }

    #[test]
    fn test_missing_agent_reports_not_installed() {
        let home = TempDir::new().unwrap();
        assert_eq!(
            status(Agent::Crush, home.path()).unwrap(),
            Status::AgentMissing
        );
    }

    #[test]
    fn test_install_creates_missing_parent_dirs() {
        let home = TempDir::new().unwrap();
        // No .config/crush at all; agent_present() is false but install must still work
        let change = install(Agent::Crush, home.path(), &bin()).unwrap();
        assert!(matches!(change, Change::Written { .. }));
        assert!(home.path().join(".config/crush/crush.json").exists());
    }
}
