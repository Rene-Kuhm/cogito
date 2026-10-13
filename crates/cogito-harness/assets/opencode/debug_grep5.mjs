// Test: what does the plugin actually produce for the grep key?
import { readFileSync } from 'fs';
const src = readFileSync('/home/tecnodespegue/proyectos/cogito/crates/cogito-harness/assets/opencode/cogito.js', 'utf8');

// Find the grepNudge function's key line
const match = src.match(/const key = `grep:.*?`/);
console.error('grepNudge key line:', match ? match[0] : 'NOT FOUND');

// Also check what args are used in the key
const keyMatch = src.match(/const key = `grep:\$\{pattern\}:\$\{.*?\}/);
console.error('grepNudge keyMatch:', keyMatch ? keyMatch[0] : 'NOT FOUND');

// Test what the key actually evaluates to
const pattern = 'handle_request';
const args = {};
const key = `grep:${pattern}:${args.path || ""}:${args.glob || ""}`;
console.error('Evaluated key:', JSON.stringify(key), 'colons:', (key.match(/:/g)||[]).length);

// Also check what path/glob values the plugin receives
// args = output.args = { pattern: 'handle_request' }
// args.path = undefined
// args.glob = undefined
// So key = `grep:${pattern}:${undefined || ""}:${undefined || ""}` = `grep:${pattern}::`
console.error('args.path || "" =', JSON.stringify(args.path || ""));
console.error('args.glob || "" =', JSON.stringify(args.glob || ""));
