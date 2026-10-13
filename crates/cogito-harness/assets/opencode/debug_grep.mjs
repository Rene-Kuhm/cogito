import { CogitoPlugin } from "./cogito.js";

const PROJECT_DIR = "/tmp/cogito-test-project";
const SEARCH_OUTPUT = "src/handlers.rs:10  async fn handle_request()\nsrc/handlers.rs:45  fn process_data()";

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
    return {
      stdout: this._entries[cmd]?.stdout ?? '',
      exitCode: this._entries[cmd]?.exitCode ?? 127,
      cwd() { return this; },
      quiet() { return this; },
      nothrow() { return this; }
    };
  }
  get $() { return this.cmd.bind(this); }
  mock(cmd, stdout, exitCode = 0) { this._entries[cmd] = { stdout, exitCode }; }
}

const $ = new FakeShell();
$.mock('test -e .git', '', 0);
$.mock('test -d .cogito', '', 0);
$.mock('cogito scan --quiet', '', 0);
$.mock('cogito map --card', '<cogito_project>test</cogito_project>', 0);
$.mock('cogito search "handle_request"', SEARCH_OUTPUT, 0);
$.mock('cat "/tmp/cogito-test-project/.cogito/nudges/s4" 2>/dev/null', '', 1);
$.mock('mkdir -p "/tmp/cogito-test-project/.cogito/nudges"', '', 0);
const nudgeCmd = 'cat >> "/tmp/cogito-test-project/.cogito/nudges/s4" << \'EOF\'\ngrep:handle_request::::0\nEOF';
console.error('nudgeCmd:', JSON.stringify(nudgeCmd));
$.mock(nudgeCmd, '', 0);

const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
const hook = plugin['tool.execute.before'];

// First call - should throw
shellCalls = [];
try {
  await hook(
    { tool: 'grep', sessionID: 's4', callID: 'c1' },
    { args: { pattern: 'handle_request' } }
  );
  console.error('First call: did NOT throw');
} catch(e) {
  console.error('First call: threw');
}
console.error('Shell calls after first:', JSON.stringify(shellCalls));

// Update mock to simulate nudge file having the key
const nudgeFileContent = 'grep:handle_request::::0\n';
$.mock('cat "/tmp/cogito-test-project/.cogito/nudges/s4" 2>/dev/null', nudgeFileContent, 0);
console.error('Updated nudge file content:', JSON.stringify(nudgeFileContent));

// Second call - should NOT throw
shellCalls = [];
try {
  await hook(
    { tool: 'grep', sessionID: 's4', callID: 'c2' },
    { args: { pattern: 'handle_request' } }
  );
  console.error('Second call: did NOT throw');
} catch(e) {
  console.error('Second call: threw');
}
console.error('Shell calls after second:', JSON.stringify(shellCalls));
