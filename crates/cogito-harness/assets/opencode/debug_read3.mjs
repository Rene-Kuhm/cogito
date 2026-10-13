import { CogitoPlugin } from "./cogito.js";

const PROJECT_DIR = "/tmp/cogito-test-project";
const OUTLINE_MIN_LINES = 250;
const LARGE_FILE_LINES = new Array(OUTLINE_MIN_LINES + 50)
  .fill(0).map((_, i) => "pub fn f" + i + "() {}").join("\n");
const OUTLINE_OUTPUT = "src/lib.rs · 300 lines\n  1-10 fn main()";

let shellCalls = [];

class FakeShell {
  constructor() { this._entries = {}; }
  cmd(strings, ...values) {
    let cmd = strings[0] || "";
    for (let i = 0; i < values.length; i++) { cmd += String(values[i]); cmd += strings[i + 1] || ""; }
    shellCalls.push(cmd);
    console.error('CMD:', JSON.stringify(cmd));
    return { stdout: this._entries[cmd]?.stdout ?? '', exitCode: this._entries[cmd]?.exitCode ?? 127, cwd() { return this; }, quiet() { return this; }, nothrow() { return this; } };
  }
  get $() { return this.cmd.bind(this); }
  mock(cmd, stdout, exitCode = 0) { console.error('MOCK:', JSON.stringify(cmd)); this._entries[cmd] = { stdout, exitCode }; }
}

const $ = new FakeShell();
$.mock('test -e .git', '', 0);
$.mock('test -d .cogito', '', 0);
$.mock('cogito scan --quiet', '', 0);
$.mock('cogito map --card', '<cogito_project>test</cogito_project>', 0);

const largeFilePath = PROJECT_DIR + '/src/lib.rs';
console.error('largeFilePath:', JSON.stringify(largeFilePath));
$.mock('cat "' + largeFilePath + '"', LARGE_FILE_LINES, 0);
$.mock('cogito outline "src/lib.rs"', OUTLINE_OUTPUT, 0);
$.mock('cat "/tmp/cogito-test-project/.cogito/nudges/s2" 2>/dev/null', '', 1);
$.mock('mkdir -p "/tmp/cogito-test-project/.cogito/nudges"', '', 0);
const readKey = 'read:src/lib.rs';
const nudgeHeredocCmd = 'cat >> "/tmp/cogito-test-project/.cogito/nudges/s2" << \'EOF\'\n' + readKey + '\nEOF';
console.error('nudgeHeredocCmd:', JSON.stringify(nudgeHeredocCmd));
$.mock(nudgeHeredocCmd, '', 0);

const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
const hook = plugin['tool.execute.before'];

shellCalls = [];
try {
  await hook({ tool: 'read', sessionID: 's2', callID: 'c1' }, { args: { filePath: largeFilePath } });
  console.error('Did NOT throw');
} catch(e) {
  console.error('Threw:', e.message.slice(0, 100));
}
console.error('Shell calls:', shellCalls.length);
