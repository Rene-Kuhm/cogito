import { CogitoPlugin } from "./cogito.js";

const PROJECT_DIR = "/tmp/cogito-test-project";
const OUTLINE_MIN_LINES = 250;

const LARGE_FILE_LINES = new Array(OUTLINE_MIN_LINES + 50)
  .fill(0)
  .map((_, i) => "pub fn f" + i + "() {}")
  .join("\n");

const OUTLINE_OUTPUT = "src/lib.rs · 300 lines · module crate";

let shellCalls = [];

class FakeShell {
  constructor() { this._entries = {}; }
  cmd(strings, ...values) {
    let cmd = strings[0] || "";
    for (let i = 0; i < values.length; i++) {
      cmd += String(values[i]);
      cmd += strings[i + 1] || "";
    }
    shellCalls.push(cmd);
    console.error('CMD:', JSON.stringify(cmd));
    const entry = this._entries[cmd];
    return {
      stdout: entry?.stdout ?? '',
      exitCode: entry?.exitCode ?? 127,
      cwd() { return this; },
      quiet() { return this; },
      nothrow() { return this; }
    };
  }
  get $() { return this.cmd.bind(this); }
  mock(cmd, stdout, exitCode = 0) {
    console.error('MOCK:', JSON.stringify(cmd), 'exitCode:', exitCode);
    this._entries[cmd] = { stdout, exitCode };
  }
}

const $ = new FakeShell();
$.mock('test -e .git', '', 0);
$.mock('test -d .cogito', '', 0);
$.mock('cogito scan --quiet', '', 0);
$.mock('cogito map --card', '<cogito_project>test</cogito_project>', 0);

const filePath = PROJECT_DIR + '/src/lib.rs';
console.error('filePath:', JSON.stringify(filePath));
$.mock('cat "' + filePath + '"', LARGE_FILE_LINES, 0);
$.mock('cogito outline "src/lib.rs"', OUTLINE_OUTPUT, 0);
$.mock('cat "/tmp/cogito-test-project/.cogito/nudges/s2" 2>/dev/null', '', 1);
$.mock('mkdir -p "/tmp/cogito-test-project/.cogito/nudges"', '', 0);
$.mock('cat >> "/tmp/cogito-test-project/.cogito/nudges/s2" << \'EOF\'\nread:src/lib.rs\nEOF', '', 0);

const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
const hook = plugin['tool.execute.before'];

console.error('Calling hook...');
shellCalls = [];
try {
  hook(
    { tool: 'read', sessionID: 's2', callID: 'c1' },
    { args: { filePath: filePath } }
  );
  console.error('Did NOT throw');
} catch(e) {
  console.error('Threw:', e.message.slice(0, 200));
}
console.error('Shell calls:', JSON.stringify(shellCalls));
console.error('Mock entries:', JSON.stringify(Object.keys($._entries)));
