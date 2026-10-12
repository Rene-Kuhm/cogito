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

/// Result of inspecting one agent.
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
        let configured = value
            .get("packages")
            .and_then(|p| p.as_array())
            .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(PI_PACKAGE)));
        return Ok(if configured {
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
    Ok(if present {
        Status::Configured
    } else {
        Status::NotConfigured
    })
}

/// Configure `agent` to use the `cogito` binary at `binary`.
pub fn install(agent: Agent, home: &Path, binary: &Path) -> Result<Change> {
    if status(agent, home)? == Status::Configured {
        return Ok(Change::AlreadyCorrect);
    }

    let path = agent.config_path(home);
    let backup = backup(&path)?;

    if agent.uses_pi_package() {
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
        return Ok(Change::Written { backup });
    }

    let bin = binary.display().to_string();

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
    Ok(Change::Written { backup })
}

/// Remove Cogito's configuration from `agent`.
pub fn uninstall(agent: Agent, home: &Path) -> Result<Change> {
    let path = agent.config_path(home);
    if !path.exists() {
        return Ok(Change::AlreadyCorrect);
    }

    let backup = backup(&path)?;

    if agent.uses_pi_package() {
        let mut value = read_json(&path)?;
        if let Some(arr) = value.get_mut("packages").and_then(|p| p.as_array_mut()) {
            arr.retain(|v| v.as_str() != Some(PI_PACKAGE));
        }
        write_json(&path, &value)?;
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
    Ok(Change::Written { backup })
}

// ============================================================
// Helpers
// ============================================================

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
fn backup(path: &Path) -> Result<Option<PathBuf>> {
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
    fn test_gentle_shell_adds_package() {
        let home = TempDir::new().unwrap();
        std::fs::create_dir_all(home.path().join(".gentle-shell/agent")).unwrap();
        std::fs::write(
            home.path().join(".gentle-shell/agent/settings.json"),
            r#"{"tuiMode":"fullscreen","packages":["npm:other"]}"#,
        )
        .unwrap();

        install(Agent::GentleShell, home.path(), &bin()).unwrap();
        let value = read_json(&home.path().join(".gentle-shell/agent/settings.json")).unwrap();
        assert_eq!(value["tuiMode"], "fullscreen");
        let packages = value["packages"].as_array().unwrap();
        assert!(packages.iter().any(|v| v == "npm:other"));
        assert!(packages.iter().any(|v| v == PI_PACKAGE));

        uninstall(Agent::GentleShell, home.path()).unwrap();
        let value = read_json(&home.path().join(".gentle-shell/agent/settings.json")).unwrap();
        let packages = value["packages"].as_array().unwrap();
        assert!(packages.iter().any(|v| v == "npm:other"));
        assert!(!packages.iter().any(|v| v == PI_PACKAGE));
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
