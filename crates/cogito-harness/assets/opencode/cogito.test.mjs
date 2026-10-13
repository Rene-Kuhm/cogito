// Executable test for the opencode Cogito plugin.
// Run with: node crates/cogito-harness/assets/opencode/cogito.test.mjs
//
// Verifies:
// - experimental.chat.system.transform pushes the card into output.system exactly once
// - tool.execute.before with a whole-file read (> OUTLINE_MIN_LINES lines) throws with outline
// - repeating that identical read passes without throwing
// - a read with a small explicit limit passes without throwing
// - tool.execute.before with a plain-name grep throws with search result
// - repeating that grep passes; a regex-pattern grep passes
// - no hook rejects when cogito is unavailable (non-zero exit, empty stdout)

import { CogitoPlugin } from "./cogito.js";

const OUTLINE_MIN_LINES = 250; // must match the plugin
const WHOLE_READ_LIMIT = 400; // must match the plugin

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
 * Because cogito commands never contain `${...}` substitutions, all
 * expressions resolve to empty strings and the full command is the sole
 * array element.  We expose `cmd` (singular) as the tagged-template entry
 * point so the plugin's $`...` calls work unchanged.
 */
class FakeShell {
  constructor(entries = []) {
    this._entries = entries; // [{ cmd: string, stdout: string, exitCode: number }]
  }

  /**
   * Tagged-template entry point: $`cmd`.
   *
   * Bun's $ template tag does proper interpolation: `${expr}` is replaced by the
   * string value of `expr`.  We replicate that by joining the strings array and
   * substituting `${...}` placeholders with the corresponding values.
   *
   * The arguments are (strings: string[], ...values: any[]).
   */
  cmd(strings, ...values) {
    // Reconstruct the command exactly as Bun does: interleave strings and values.
    // Bun replaces ${expr} in the template with the value of expr.  When a
    // tagged template has N ${...} substitutions, strings has N+1 elements and
    // values has N elements.  The result is:
    //   strings[0] + values[0] + strings[1] + values[1] + ... + strings[N]
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

const LARGE_FILE_LINES = new Array(OUTLINE_MIN_LINES + 50)
  .fill(0)
  .map((_, i) => "pub fn f" + i + "() {}")
  .join("\n");

const OUTLINE_OUTPUT = "src/lib.rs · 300 lines · module crate\n  1-10  mod handlers  (src/handlers.rs)\n  11-20 mod utils     (src/utils.rs)\n  25-99 fn main() {}";

const SEARCH_OUTPUT = "src/handlers.rs:10  async fn handle_request()\nsrc/handlers.rs:45  fn process_data()";

function buildShell(isProject = true) {
  const $ = new FakeShell();

  // project detection
  $.mock("test -e .git", isProject ? "" : "", isProject ? 0 : 1);
  $.mock("test -d .cogito", isProject ? "" : "", isProject ? 0 : 1);

  // scan
  $.mock("cogito scan --quiet", "", 0);

  // card
  $.mock("cogito map --card", CARD_OUTPUT, 0);

  return $;
}

// ─── Tests ───────────────────────────────────────────────────────────────────

async function runTests() {
  console.log("\n=== Test: experimental.chat.system.transform pushes card once ===");

  {
    const $ = buildShell();
    // JavaScript does not re-invoke getters during destructuring; pass $.$ so the
    // plugin receives the bound tagged-template function, not the shell object.
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

  console.log("\n=== Test: tool.execute.before — whole-file read throws with outline ===");

  {
    const $ = buildShell();
    const largeFilePath = PROJECT_DIR + "/src/lib.rs";
    $.mock("cat \"" + largeFilePath + "\"", LARGE_FILE_LINES, 0);
    $.mock("cogito outline \"src/lib.rs\"", OUTLINE_OUTPUT, 0);
    // firstTime: mkdir succeeds, nudge file write succeeds
    $.mock(
      "cat \"/tmp/cogito-test-project/.cogito/nudges/s2\" 2>/dev/null",
      "",
      1
    );
    $.mock("mkdir -p \"/tmp/cogito-test-project/.cogito/nudges\"", "", 0);
    $.mock(
      "cat >> \"/tmp/cogito-test-project/.cogito/nudges/s2\" << 'EOF'\nread:src/lib.rs\nEOF",
      "",
      0
    );

    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
    const hook = plugin["tool.execute.before"];

    // Whole-file read (no offset, no limit)
    await assertThrows(
      () =>
        hook(
          { tool: "read", sessionID: "s2", callID: "c1" },
          { args: { filePath: PROJECT_DIR + "/src/lib.rs" } }
        ),
      "Cogito's outline"
    );

    // Nudge file now contains the key, so a repeat should pass
    $.mock(
      "cat \"/tmp/cogito-test-project/.cogito/nudges/s2\" 2>/dev/null",
      "read:src/lib.rs\n",
      0
    );

    await assertResolves(() =>
      hook(
        { tool: "read", sessionID: "s2", callID: "c2" },
        { args: { filePath: PROJECT_DIR + "/src/lib.rs" } }
      )
    );
  }

  console.log("\n=== Test: tool.execute.before — read with small explicit limit passes ===");

  {
    const $ = buildShell();
    // No need to mock cat — the guard should return early before reaching it
    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
    const hook = plugin["tool.execute.before"];

    await assertResolves(() =>
      hook(
        { tool: "read", sessionID: "s3", callID: "c1" },
        { args: { filePath: PROJECT_DIR + "/src/lib.rs", offset: 10, limit: 20 } }
      )
    );
  }

  console.log(
    "\n=== Test: tool.execute.before — plain-name grep throws with search result ==="
  );

  {
    const $ = buildShell();
    $.mock("cogito search \"handle_request\"", SEARCH_OUTPUT, 0);
    $.mock(
      "cat \"/tmp/cogito-test-project/.cogito/nudges/s4\" 2>/dev/null",
      "",
      1
    );
    $.mock("mkdir -p \"/tmp/cogito-test-project/.cogito/nudges\"", "", 0);
    $.mock(
      "cat >> \"/tmp/cogito-test-project/.cogito/nudges/s4\" << 'EOF'\ngrep:handle_request::\nEOF",
      "",
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

    // Repeat: nudge file now has the key
    $.mock(
      "cat \"/tmp/cogito-test-project/.cogito/nudges/s4\" 2>/dev/null",
      "grep:handle_request::\n",
      0
    );

    await assertResolves(() =>
      hook(
        { tool: "grep", sessionID: "s4", callID: "c2" },
        { args: { pattern: "handle_request" } }
      )
    );
  }

  console.log(
    "\n=== Test: tool.execute.before — regex-pattern grep passes without throwing ==="
  );

  {
    const $ = buildShell();
    const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
    const hook = plugin["tool.execute.before"];

    await assertResolves(() =>
      hook(
        { tool: "grep", sessionID: "s5", callID: "c1" },
        { args: { pattern: "fn\\s+\\w+" } }
      )
    );
  }

  console.log(
    "\n=== Test: hooks do not reject when cogito is unavailable ==="
  );

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
    await assertResolves(() => sysHook({ sessionID: "s6", model: {} }, { system: [] }));

    // tool.execute.before
    const toolHook = plugin["tool.execute.before"];
    await assertResolves(() =>
      toolHook(
        { tool: "read", sessionID: "s6", callID: "c1" },
        { args: { filePath: PROJECT_DIR + "/src/lib.rs" } }
      )
    );
    await assertResolves(() =>
      toolHook(
        { tool: "grep", sessionID: "s6", callID: "c2" },
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
