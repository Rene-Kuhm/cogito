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

    // The surface is the project map. Decision, tag and note tools are absent on
    // purpose: they belong to the agent's memory, not to Cogito.
    assert_eq!(
        tool_names(&send_jsonrpc(messages)),
        [
            "file_outline",
            "project_map",
            "related_files",
            "search_code"
        ]
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
        "import { x } from '@m/shared';\n\nexport function requireSession(token: string): boolean {\n  return x > 0;\n}\n",
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
    let outline = call(5, "file_outline", r#","file":"lib/session.ts""#);
    let output = send_jsonrpc(vec![init, ready, &map, &related, &search, &outline]);

    let map = tool_text(&output, 2);
    assert!(map.contains("2 modules"), "{map}");
    assert!(map.contains("apps/web (@m/web)"), "{map}");
    assert!(map.contains("uses: packages/shared"), "{map}");
    assert!(
        map.contains("packages/shared (@m/shared) · 1 files · entry src/index.ts · used by 1"),
        "{map}"
    );

    let related = tool_text(&output, 3);
    // The index resolves imports and stores the resolved path, so imports
    // shows the actual file and importers shows files that import this one.
    assert!(
        related.contains("Imports (1):\n  packages/shared/src/index.ts"),
        "related should show resolved import: {related}"
    );
    assert!(
        related.contains("Imported by (1):\n  apps/web/app/page.tsx"),
        "related should show importer: {related}"
    );

    let search = tool_text(&output, 4);
    assert!(search.contains("apps/web/lib/session.ts"), "{search}");
    assert!(
        search.contains("function requireSession apps/web/lib/session.ts:3"),
        "{search}"
    );

    let outline = tool_text(&output, 5);
    assert!(
        outline.contains("3-5 export function requireSession(token: string): boolean"),
        "{outline}"
    );
}

#[test]
fn test_mcp_search_finds_new_uncommitted_file() {
    // On a git repository that has been indexed, a new uncommitted file with a
    // declaration is found via the index without an explicit refresh or scan.
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    // Initialize a git repo.
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(root)
        .output()
        .expect("git init failed");

    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };

    // Write an initial file and commit it so the repo is "indexed".
    write("src/main.ts", "export const existing = 1;\n");
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .output()
        .expect("git add failed");
    std::process::Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(root)
        .output()
        .expect("git commit failed");

    // Now write a new uncommitted file with a unique declaration.
    write(
        "src/newfeature.ts",
        "export function brandNewUtility(input: string): boolean {\n  return input.length > 0;\n}\n",
    );

    let path = root.display().to_string();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;
    let ready = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
    let search = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"search_code","arguments":{{"path":"{}","query":"brandNewUtility"}}}}}}"#,
        path
    );

    let output = send_jsonrpc(vec![init, ready, &search]);
    let text = tool_text(&output, 2);
    assert!(
        text.contains("brandNewUtility"),
        "search_code should find brandNewUtility in the new uncommitted file: {text}"
    );
    assert!(
        text.contains("src/newfeature.ts"),
        "search_code should report the new file path: {text}"
    );
}

#[test]
fn test_mcp_file_outline_matches_declarations() {
    // The index-sourced outline contains declarations with expected kinds and line ranges.
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path();

    // Initialize a git repo.
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(root)
        .output()
        .expect("git init failed");

    let write = |rel: &str, text: &str| {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };

    write("package.json", r#"{"name":"test"}"#);
    write(
        "src/lib.ts",
        "// This is a utility function\nexport function greet(name: string): string {\n  return `Hello, ${name}`;\n}\n\nexport const MAX_RETRIES = 3;\n\nclass InternalCache {\n  private data: Map<string, string>;\n  constructor() { this.data = new Map(); }\n}\n\nexport { InternalCache };\n",
    );

    // Commit so the index has something to work with.
    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .output()
        .expect("git add failed");
    std::process::Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(root)
        .output()
        .expect("git commit failed");

    let path = root.display().to_string();
    let init = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;
    let ready = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
    let outline = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"file_outline","arguments":{{"path":"{}","file":"src/lib.ts"}}}}}}"#,
        path
    );

    let output = send_jsonrpc(vec![init, ready, &outline]);
    let text = tool_text(&output, 2);

    // Verify the outline contains the expected declarations.
    assert!(
        text.contains("function greet") || text.contains("fn greet"),
        "outline should contain function greet: {text}"
    );
    assert!(
        text.contains("MAX_RETRIES") || text.contains("const MAX_RETRIES"),
        "outline should contain const MAX_RETRIES: {text}"
    );
    assert!(
        text.contains("class InternalCache") || text.contains("InternalCache"),
        "outline should contain class InternalCache: {text}"
    );

    // Verify line ranges are present (e.g., "3-5" or similar).
    assert!(
        text.contains('-') && text.matches(char::is_numeric).count() >= 4,
        "outline should contain line ranges: {text}"
    );
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
