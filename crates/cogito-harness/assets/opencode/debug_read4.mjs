// Simulate the exact test code for read test
import { CogitoPlugin } from "./cogito.js";

const PROJECT_DIR = "/tmp/cogito-test-project";
const OUTLINE_MIN_LINES = 250;
const WHOLE_READ_LIMIT = 400;
const LARGE_FILE_LINES = new Array(OUTLINE_MIN_LINES + 50).fill(0).map((_, i) => "pub fn f" + i + "() {}").join("\n");
const OUTLINE_OUTPUT = "src/lib.rs · 300 lines";

let failures = 0;

function assert(condition, message) {
  if (!condition) { console.error("FAIL:", message); failures++; }
}

async function assertThrows(fn, expectedSubstring) {
  try {
    await fn();
    assert(false, "Expected function to throw, but it did not.");
  } catch (e) {
    if (expectedSubstring !== null && !e.message.includes(expectedSubstring)) {
      assert(false, "Throw message should contain \"" + expectedSubstring + "\", got: " + e.message);
    }
  }
}

class FakeShell {
  constructor(entries = []) { this._entries = entries; }
  cmd(strings, ...values) {
    let cmd = strings[0] || "";
    for (let i = 0; i < values.length; i++) { cmd += String(values[i]); cmd += strings[i + 1] || ""; }
    const entry = this._entries.find((e) => e.cmd === cmd);
    return new FakeResult(entry ? entry.stdout : "", entry ? entry.exitCode : 127);
  }
  get $() { return this.cmd.bind(this); }
  mock(cmd, stdout, exitCode = 0) { const existing = this._entries.findIndex((e) => e.cmd === cmd); const entry = { cmd, stdout, exitCode }; if (existing >= 0) { this._entries[existing] = entry; } else { this._entries.push(entry); } }
}

class FakeResult {
  constructor(stdout, exitCode) { this.stdout = stdout; this.exitCode = exitCode; this._cwd = null; this._quiet = false; this._nothrow = false; }
  cwd(dir) { this._cwd = dir; return this; }
  quiet() { this._quiet = true; return this; }
  nothrow() { this._nothrow = true; return this; }
}

function buildShell(isProject = true) {
  const $ = new FakeShell();
  $.mock("test -e .git", isProject ? "" : "", isProject ? 0 : 1);
  $.mock("test -d .cogito", isProject ? "" : "", isProject ? 0 : 1);
  $.mock("cogito scan --quiet", "", 0);
  $.mock("cogito map --card", "<cogito_project>test</cogito_project>", 0);
  return $;
}

const $ = buildShell();
const largeFilePath = PROJECT_DIR + "/src/lib.rs";
console.error('largeFilePath:', largeFilePath);
$.mock("cat \"" + largeFilePath + "\"", LARGE_FILE_LINES, 0);
$.mock("cogito outline \"src/lib.rs\"", OUTLINE_OUTPUT, 0);
$.mock("cat \"/tmp/cogito-test-project/.cogito/nudges/s2\" 2>/dev/null", "", 1);
$.mock("mkdir -p \"/tmp/cogito-test-project/.cogito/nudges\"", "", 0);
$.mock("cat >> \"/tmp/cogito-test-project/.cogito/nudges/s2\" << 'EOF'\nread:src/lib.rs\nEOF", "", 0);

const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
const hook = plugin["tool.execute.before"];

console.error('About to call assertThrows...');
await assertThrows(
  () => hook({ tool: "read", sessionID: "s2", callID: "c1" }, { args: { filePath: largeFilePath } }),
  "Cogito's outline"
);
console.error('assertThrows returned, failures:', failures);
