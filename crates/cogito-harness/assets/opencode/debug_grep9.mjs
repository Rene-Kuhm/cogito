// Directly patch the grepNudge function to log args
import { readFileSync, writeFileSync } from 'fs';
const src = readFileSync('/home/tecnodespegue/proyectos/cogito/crates/cogito-harness/assets/opencode/cogito.js', 'utf8');
// Insert logging before the key line
const patched = src.replace(
  'const key = `grep:${pattern}:${args.path || ""}:${args.glob || ""}`;',
  'console.error("ARGS:", JSON.stringify({pattern, path: args.path, glob: args.glob})); const key = `grep:${pattern}:${args.path || ""}:${args.glob || ""}`; console.error("KEY:", JSON.stringify(key));'
);
writeFileSync('/tmp/cogito_patched2.js', patched);
const { CogitoPlugin } = await import('/tmp/cogito_patched2.js?t=' + Date.now());

const PROJECT_DIR = "/tmp/cogito-test-project";
class FakeShell {
  constructor() { this._entries = {}; }
  cmd(strings, ...values) {
    let cmd = strings[0] || "";
    for (let i = 0; i < values.length; i++) { cmd += String(values[i]); cmd += strings[i + 1] || ""; }
    return { stdout: this._entries[cmd]?.stdout ?? '', exitCode: this._entries[cmd]?.exitCode ?? 127, cwd() { return this; }, quiet() { return this; }, nothrow() { return this; } };
  }
  get $() { return this.cmd.bind(this); }
  mock(cmd, stdout, exitCode = 0) { this._entries[cmd] = { stdout, exitCode }; }
}

const $ = new FakeShell();
$.mock('test -e .git', '', 0);
$.mock('test -d .cogito', '', 0);
$.mock('cogito scan --quiet', '', 0);
$.mock('cogito map --card', '<cogito_project>test</cogito_project>', 0);
$.mock('cogito search "handle_request"', 'result', 0);
$.mock('cat "/tmp/cogito-test-project/.cogito/nudges/s4" 2>/dev/null', '', 1);
$.mock('mkdir -p "/tmp/cogito-test-project/.cogito/nudges"', '', 0);
$.mock('cat >> "/tmp/cogito-test-project/.cogito/nudges/s4" << \'EOF\'\ngrep:handle_request:::\nEOF', '', 0);

const plugin = await CogitoPlugin({ directory: PROJECT_DIR, $: $.$ });
const hook = plugin['tool.execute.before'];
await hook({ tool: 'grep', sessionID: 's4', callID: 'c1' }, { args: { pattern: 'handle_request' } });
