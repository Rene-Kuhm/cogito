//! End-to-end MCP server tests
//!
//! These tests spawn `cogito mcp` and verify the JSON-RPC protocol
//! implementation: initialize, list tools, call tools.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn mcp_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_cogito"))
}

/// Arguments that select the MCP subcommand.
fn mcp_args() -> [&'static str; 1] {
    ["mcp"]
}

fn send_jsonrpc(messages: Vec<&str>) -> String {
    let mut child = Command::new(mcp_bin())
        .args(mcp_args())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn cogito mcp");

    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    for msg in messages {
        writeln!(stdin, "{msg}").expect("Failed to write to stdin");
    }
    drop(stdin);

    let output = child
        .wait_with_output()
        .expect("Failed to read cogito mcp output");
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn test_mcp_initialize() {
    let messages = vec![
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#,
    ];

    let output = send_jsonrpc(messages);

    assert!(output.contains("\"id\":1"), "Should have response id 1");
    assert!(
        output.contains("\"protocolVersion\""),
        "Should include protocol version"
    );
    assert!(
        output.contains("Cogito"),
        "Should advertise Cogito server name"
    );
    assert!(
        output.contains("capabilities"),
        "Should include capabilities"
    );
}

/// The tools a `tools/list` response advertises, by name.
fn tool_names(output: &str) -> Vec<String> {
    let response = output
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["id"] == 2)
        .expect("a tools/list response");
    let mut names: Vec<String> = response["result"]["tools"]
        .as_array()
        .expect("a tool array")
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn test_mcp_lists_the_map_tools_only() {
    let messages = vec![
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    ];

    // The surface is the project map. Decision and note tools are absent on
    // purpose: they belong to the agent's memory, not to Cogito.
    assert_eq!(
        tool_names(&send_jsonrpc(messages)),
        ["add_tag", "project_map", "related_files", "search_code"]
    );
}

/// The text of the tool result answering request `id`.
fn tool_text(output: &str, id: u64) -> String {
    output
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v["id"] == id)
        .and_then(|v| {
            v["result"]["content"][0]["text"]
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_else(|| panic!("no result for request {id} in:\n{output}"))
}

#[test]
fn test_mcp_map_tools_on_a_workspace() {
    // A two-package workspace: web imports shared through the package name.
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();
    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write("package.json", r#"{"name":"mono","private":true}"#);
    write("packages/shared/package.json", r#"{"name":"@m/shared"}"#);
    write("packages/shared/src/index.ts", "export const x = 1;\n");
    write("apps/web/package.json", r#"{"name":"@m/web"}"#);
    write("apps/web/tsconfig.json", "{}");
    write(
        "apps/web/lib/session.ts",
        "import { x } from '@m/shared';\n",
    );
    write("apps/web/app/page.tsx", "import '@/lib/session';\n");

    let path = root.display().to_string();
    let call = |id: u64, name: &str, args: &str| {
        format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"{name}","arguments":{{"path":"{path}"{args}}}}}}}"#
        )
    };
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;
    let ready = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;

    let map = call(2, "project_map", "");
    let related = call(3, "related_files", r#","file":"lib/session.ts""#);
    let search = call(4, "search_code", r#","query":"session""#);
    let output = send_jsonrpc(vec![init, ready, &map, &related, &search]);

    let map = tool_text(&output, 2);
    assert!(map.contains("2 modules"), "{map}");
    assert!(map.contains("apps/web (@m/web)"), "{map}");
    assert!(map.contains("uses: packages/shared"), "{map}");
    assert!(
        map.contains("packages/shared (@m/shared) · 1 files · entry src/index.ts · used by 1"),
        "{map}"
    );

    let related = tool_text(&output, 3);
    assert!(
        related.contains("Imports (1):\n  packages/shared/src/index.ts"),
        "{related}"
    );
    assert!(
        related.contains("Imported by (1):\n  apps/web/app/page.tsx"),
        "{related}"
    );

    let search = tool_text(&output, 4);
    assert!(search.contains("apps/web/lib/session.ts"), "{search}");
}

#[test]
fn test_mcp_stdout_is_only_json_rpc() {
    // stdout is the protocol channel: a log line there is a parse error for a
    // strict client. The server logs at INFO on startup, so this fails if
    // logging is ever pointed back at stdout.
    let messages = vec![
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    ];

    let output = send_jsonrpc(messages);
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    assert!(!lines.is_empty(), "the server must answer");
    for line in lines {
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(line);
        assert!(
            parsed.is_ok_and(|v| v.get("jsonrpc").is_some()),
            "every stdout line must be a JSON-RPC message, got: {line}"
        );
    }
}
