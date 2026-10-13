import { CogitoPlugin } from "./cogito.js";
const PROJECT_DIR = "/tmp/cogito-test-project";
const SEARCH_OUTPUT = "src/handlers.rs:10  async fn handle_request()";

class FakeShell {
  constructor() { this._entries = {}; }
  cmd(strings, ...values) {
    let cmd = strings[0] || "";
    for (let i = 0; i < values.length; i++) {
      cmd += String(values[i]);
      cmd += strings[i + 1] || "";
    }
    console.error('ACTUAL_CMD:', cmd.split('\n').map(l => l.length + ':' + l).join(' | '));
    return {
      stdout: this._entries[cmd]?.stdout ?? '',
      exitCode: this._entries[cmd]?.exitCode ?? 127,
      cwd() { return this; }, quiet() { return this; }, nothrow() { return this; }
    };
  }
  get $() { return this.cmd.bind(this); }
  mock(cmd, stdout, exitCode = 0) {
    console.error('MOCK_CMD:', cmd.split('\n').map(l => l.length + ':' + l).join(' | '));
    this._entries[cmd] = { stdout, exitCode };
  }
}

const $ = new FakeShell();
$.mock('test -e .git', '', 0);
$.mock('test -d .cogito', '', 0);
$.mock('cogito scan --quiet', '', 0);
$.mock('cogito map --card', '<cogito_project>test</cogito_project>', 0);
$.mock('cogito search "handle_request"', SEARCH_OUTPUT, 0);
$.mock('cat "/tmp/cogito-test-project/.cogito/nudges/s4" 2>/dev/null', '', 1);
$.mock('mkdir -p "/tmp/cogito-test-project/.cogito/nudges"', '', 0);

// Test the exact mock format from the test file
const nudgeKey = 'grep:handle_request:::';
const nudgeHeredocCmd = `cat >> "/tmp/cogito-test-project/.cogito/nudges/s4" << 'EOF'
${nudgeKey}
EOF`;
console.error('MOCK_CMD built:', JSON.stringify(nudgeHeredocCmd));
console.error('MOCK_CMD lines:', nudgeHeredocCmd.split('\n'));
$.mock(nudgeHeredocCmd, '', 0);

const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
const hook = plugin['tool.execute.before'];
await hook({ tool: 'grep', sessionID: 's4', callID: 'c1' }, { args: { pattern: 'handle_request' } });
console.error('Done');
