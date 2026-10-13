// Test the exact command string
const nudgeKey = 'grep:handle_request:::';
const nudgeHeredocCmd = `cat >> "/tmp/cogito-test-project/.cogito/nudges/s4" << 'EOF'
${nudgeKey}
EOF`;
console.error('nudgeHeredocCmd length:', nudgeHeredocCmd.length);
console.error('nudgeHeredocCmd charCodes:', [...nudgeHeredocCmd.slice(-20)].map(c => c.charCodeAt(0)));

// Split by lines
const lines = nudgeHeredocCmd.split('\n');
console.error('Lines:', lines.map(l => JSON.stringify(l)));

// Now test what the plugin would produce
const pattern = 'handle_request';
const args = {};
const key = `grep:${pattern}:${args.path || ""}:${args.glob || ""}`;
console.error('key:', JSON.stringify(key), 'length:', key.length, 'colons:', (key.match(/:/g) || []).length);

// Plugin heredoc
const nudgeFile = '/tmp/cogito-test-project/.cogito/nudges/s4';
const pluginHeredoc = `cat >> "${nudgeFile}" << 'EOF'
${key}
EOF`;
console.error('pluginHeredoc:', JSON.stringify(pluginHeredoc));
const pluginLines = pluginHeredoc.split('\n');
console.error('pluginLines:', pluginLines.map(l => JSON.stringify(l)));
console.error('Match:', nudgeHeredocCmd === pluginHeredoc);
