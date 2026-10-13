// Check the patched file
import { readFileSync } from 'fs';
const patched = readFileSync('/tmp/cogito_patched.js', 'utf8');
const keyLines = patched.split('\n').filter(l => l.includes('const key = `grep:'));
console.error('Key lines in patched file:', keyLines);
console.error('Count:', keyLines.length);
