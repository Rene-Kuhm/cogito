// Executable test for the opencode Cogito plugin.
// Run with: node crates/cogito-harness/assets/opencode/cogito.test.mjs
//
// The plugin delegates every guardrail decision to `cogito guard`, so these
// tests cover the delegation contract:
// - experimental.chat.system.transform pushes the card into output.system exactly once
// - a block from `cogito guard` is transmitted with its reason
// - a non-block answer lets the call through
// - an error from `cogito` (non-zero exit, bad JSON, missing binary) lets the call through
// - a read outside the project never reaches `cogito guard`
// - reads and greps are forwarded with the fields the guard contract expects

import { CogitoPlugin } from "./cogito.js";

// ─── Fake `$` shell ────────────────────────────────────────────────────────

/**
 * Fake shell that returns canned output per command.
 * Implements the BunShell-like interface the plugin uses.
 *
 * Each entry is `{ cmd, stdout, exitCode }`.  The first matching entry wins.
 * Commands that are not registered return exitCode 127.
 *
 * The plugin calls $ as a tagged template: $`cmd`.  In JavaScript a tagged
 * template `tag`literal`...`` calls `tag(["literal", ...], val, ...)`.
 */
class FakeShell {
  constructor(entries = []) {
    this._entries = entries; // [{ cmd: string, stdout: string, exitCode: number }]
  }

  /**
   * Tagged-template entry point: $`cmd`.
   *
   * Reconstructs the command exactly as Bun does by interleaving strings and
   * interpolated values.
   */
  cmd(strings, ...values) {
    let cmd = strings[0] || "";
    for (let i = 0; i < values.length; i++) {
      cmd += String(values[i]);
      cmd += strings[i + 1] || "";
    }
    const entry = this._entries.find((e) => e.cmd === cmd);
    return new FakeResult(entry ? entry.stdout : "", entry ? entry.exitCode : 127);
  }

  /** Also expose a `.bind(this)` alias as `$` so the plugin can destructure it. */
  get $() { return this.cmd.bind(this); }

  /** Add or replace a canned response for `cmd`. */
  mock(cmd, stdout, exitCode = 0) {
    const existing = this._entries.findIndex((e) => e.cmd === cmd);
    const entry = { cmd, stdout, exitCode };
    if (existing >= 0) {
      this._entries[existing] = entry;
    } else {
      this._entries.push(entry);
    }
  }
}

// ─── Chainable fake shell result ─────────────────────────────────────────────

/** Chainable fake shell result. */
class FakeResult {
  constructor(stdout, exitCode) {
    this.stdout = stdout;
    this.exitCode = exitCode;
    this._cwd = null;
    this._quiet = false;
    this._nothrow = false;
  }

  cwd(dir) {
    this._cwd = dir;
    return this;
  }
  quiet() {
    this._quiet = true;
    return this;
  }
  nothrow() {
    this._nothrow = true;
    return this;
  }
}

// ─── Tiny assertion helper ───────────────────────────────────────────────────

let failures = 0;

function assert(condition, message) {
  if (!condition) {
    console.error("FAIL:", message);
    failures++;
  }
}

async function assertThrows(fn, expectedSubstring) {
  try {
    await fn();
    assert(false, "Expected function to throw, but it did not.");
  } catch (e) {
    if (expectedSubstring !== null && !e.message.includes(expectedSubstring)) {
      assert(
        false,
        "Throw message should contain \"" + expectedSubstring + "\", got: " + e.message
      );
    }
  }
}

async function assertResolves(fn) {
  try {
    await fn();
  } catch (e) {
    assert(false, "Should not throw, but did: " + e.message);
  }
}

// ─── Test helpers ────────────────────────────────────────────────────────────

const PROJECT_DIR = "/tmp/cogito-test-project";

const CARD_OUTPUT = "<cogito_project>\ncode files · indexed at abc1234\nBusiest files: src/lib.rs · src/main.rs · src/handlers.rs\nTools: search_code · file_outline · read · grep · search\n</cogito_project>";

const OUTLINE_REASON = "src/lib.rs has 300 lines. Cogito's outline instead of the whole file:\n\nsrc/lib.rs · 300 lines\n  1-10  fn main() {}";
const SEARCH_REASON = "Cogito's index for \"handle_request\" (symbols and doc sections with file:line, files, modules):\n\nsrc/handlers.rs:10  async fn handle_request()";

function buildShell() {
  const $ = new FakeShell();

  // project detection
  $.mock("test -e .git", "", 0);
  $.mock("test -d .cogito", "", 0);

  // scan
  $.mock("cogito scan --quiet", "", 0);

  // card
  $.mock("cogito map --card", CARD_OUTPUT, 0);

  return $;
}

/** The exact shell command the plugin uses to ask `cogito guard`. */
function guardCommand(call) {
  return `printf '%s' ${JSON.stringify(call)} | cogito guard`;
}

// ─── Tests ───────────────────────────────────────────────────────────────────

async function runTests() {
  console.log("\n=== Test: experimental.chat.system.transform pushes card once ===");

  {
    const $ = buildShell();
    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
    const hook = plugin["experimental.chat.system.transform"];

    const output = { system: [] };
    await hook({ sessionID: "s1", model: {} }, output);
    assert(
      output.system.length === 1,
      "Expected 1 card, got " + output.system.length
    );
    assert(
      output.system[0].includes("<cogito_project>"),
      "Card should contain marker: " + output.system[0]
    );

    // Second call: must not push again (marker guard)
    await hook({ sessionID: "s1", model: {} }, output);
    assert(
      output.system.length === 1,
      "Expected still 1 card after second call, got " + output.system.length
    );
  }

  console.log("\n=== Test: a block from cogito guard is transmitted with its reason ===");

  {
    const $ = buildShell();
    const filePath = PROJECT_DIR + "/src/lib.rs";
    const call = {
      tool: "read",
      input: { path: filePath },
      cwd: PROJECT_DIR,
      session: "s2",
    };
    $.mock(
      guardCommand(call),
      JSON.stringify({ block: true, reason: OUTLINE_REASON }),
      0
    );

    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
    const hook = plugin["tool.execute.before"];

    await assertThrows(
      () =>
        hook(
          { tool: "read", sessionID: "s2", callID: "c1" },
          { args: { filePath } }
        ),
      "Cogito's outline"
    );
  }

  console.log("\n=== Test: block=false lets the whole-file read through ===");

  {
    const $ = buildShell();
    const filePath = PROJECT_DIR + "/src/lib.rs";
    const call = {
      tool: "read",
      input: { path: filePath },
      cwd: PROJECT_DIR,
      session: "s3",
    };
    $.mock(guardCommand(call), JSON.stringify({ block: false }), 0);

    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
    const hook = plugin["tool.execute.before"];

    await assertResolves(() =>
      hook(
        { tool: "read", sessionID: "s3", callID: "c1" },
        { args: { filePath } }
      )
    );
  }

  console.log("\n=== Test: a plain-name grep block carries the search reason ===");

  {
    const $ = buildShell();
    const call = {
      tool: "grep",
      input: { pattern: "handle_request" },
      cwd: PROJECT_DIR,
      session: "s4",
    };
    $.mock(
      guardCommand(call),
      JSON.stringify({ block: true, reason: SEARCH_REASON }),
      0
    );

    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
    const hook = plugin["tool.execute.before"];

    await assertThrows(
      () =>
        hook(
          { tool: "grep", sessionID: "s4", callID: "c1" },
          { args: { pattern: "handle_request" } }
        ),
      "Cogito's index"
    );
  }

  console.log("\n=== Test: reads and greps forward offset/limit and path/glob ===");

  {
    const $ = buildShell();
    const filePath = PROJECT_DIR + "/src/lib.rs";
    const readCall = {
      tool: "read",
      input: { path: filePath, offset: 10, limit: 20 },
      cwd: PROJECT_DIR,
      session: "s5",
    };
    $.mock(guardCommand(readCall), JSON.stringify({ block: false }), 0);

    const grepCall = {
      tool: "grep",
      input: { pattern: "handle_request", path: "src", glob: "*.rs" },
      cwd: PROJECT_DIR,
      session: "s5",
    };
    $.mock(guardCommand(grepCall), JSON.stringify({ block: false }), 0);

    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
    const hook = plugin["tool.execute.before"];

    await assertResolves(() =>
      hook(
        { tool: "read", sessionID: "s5", callID: "c1" },
        { args: { filePath, offset: 10, limit: 20 } }
      )
    );
    await assertResolves(() =>
      hook(
        { tool: "grep", sessionID: "s5", callID: "c2" },
        { args: { pattern: "handle_request", path: "src", glob: "*.rs" } }
      )
    );
  }

  console.log("\n=== Test: a cogito error lets the call through ===");

  {
    const $ = buildShell();
    const filePath = PROJECT_DIR + "/src/lib.rs";
    const call = {
      tool: "read",
      input: { path: filePath },
      cwd: PROJECT_DIR,
      session: "s6",
    };
    // Non-zero exit with empty stdout, as a missing/broken binary gives.
    $.mock(guardCommand(call), "", 127);

    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
    const hook = plugin["tool.execute.before"];

    await assertResolves(() =>
      hook(
        { tool: "read", sessionID: "s6", callID: "c1" },
        { args: { filePath } }
      )
    );
  }

  console.log("\n=== Test: unparseable cogito output lets the call through ===");

  {
    const $ = buildShell();
    const filePath = PROJECT_DIR + "/src/lib.rs";
    const call = {
      tool: "read",
      input: { path: filePath },
      cwd: PROJECT_DIR,
      session: "s7",
    };
    $.mock(guardCommand(call), "not json", 0);

    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
    const hook = plugin["tool.execute.before"];

    await assertResolves(() =>
      hook(
        { tool: "read", sessionID: "s7", callID: "c1" },
        { args: { filePath } }
      )
    );
  }

  console.log("\n=== Test: a read outside the project never reaches cogito guard ===");

  {
    const $ = buildShell();
    // If the plugin forwarded this, the answer would block it.
    const outsideCall = {
      tool: "read",
      input: { path: "/tmp/elsewhere/lib.rs" },
      cwd: PROJECT_DIR,
      session: "s8",
    };
    $.mock(
      guardCommand(outsideCall),
      JSON.stringify({ block: true, reason: OUTLINE_REASON }),
      0
    );

    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
    const hook = plugin["tool.execute.before"];

    await assertResolves(() =>
      hook(
        { tool: "read", sessionID: "s8", callID: "c1" },
        { args: { filePath: "/tmp/elsewhere/lib.rs" } }
      )
    );
  }

  console.log("\n=== Test: hooks do not reject when cogito is unavailable ===");

  {
    // $ that always returns non-zero and empty stdout
    const $ = new FakeShell();
    $.mock("test -e .git", "", 1); // not a project
    $.mock("test -d .cogito", "", 1);

    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });

    // event hook
    const eventHook = plugin.event;
    await assertResolves(() => eventHook({ event: { type: "session.start" } }));

    // system transform
    const sysHook = plugin["experimental.chat.system.transform"];
    await assertResolves(() => sysHook({ sessionID: "s9", model: {} }, { system: [] }));

    // tool.execute.before
    const toolHook = plugin["tool.execute.before"];
    await assertResolves(() =>
      toolHook(
        { tool: "read", sessionID: "s9", callID: "c1" },
        { args: { filePath: PROJECT_DIR + "/src/lib.rs" } }
      )
    );
    await assertResolves(() =>
      toolHook(
        { tool: "grep", sessionID: "s9", callID: "c2" },
        { args: { pattern: "something" } }
      )
    );
  }

  // ─── Summary ────────────────────────────────────────────────────────────────
  console.log("\n" + "-".repeat(60));
  if (failures === 0) {
    console.log("All tests passed.");
    process.exit(0);
  } else {
    console.error(failures + " test(s) FAILED.");
    process.exit(1);
  }
}

runTests().catch((e) => {
  console.error("Test runner threw:", e);
  process.exit(1);
});
