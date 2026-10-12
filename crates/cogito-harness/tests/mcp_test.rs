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

#[test]
fn test_mcp_lists_five_tools() {
    let messages = vec![
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
    ];

    let output = send_jsonrpc(messages);

    // All 5 tools should be advertised
    assert!(output.contains("scan_project"), "Should list scan_project");
    assert!(
        output.contains("compress_context"),
        "Should list compress_context"
    );
    assert!(
        output.contains("search_knowledge"),
        "Should list search_knowledge"
    );
    assert!(output.contains("add_decision"), "Should list add_decision");
    assert!(output.contains("add_tag"), "Should list add_tag");
}

#[test]
fn test_mcp_compress_tool() {
    let messages = vec![
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"compress_context","arguments":{"content":"// big comment\nfn main() {\n    // another\n    let x = 5;\n    println!(\"{}\", x);\n}","max_tokens":1000}}}"#,
    ];

    let output = send_jsonrpc(messages);

    assert!(output.contains("\"id\":2"), "Should respond to call");
    assert!(
        output.contains("Original tokens"),
        "Should show original tokens"
    );
    assert!(
        output.contains("Compressed tokens"),
        "Should show compressed tokens"
    );
    assert!(output.contains("Savings"), "Should show savings");
}
