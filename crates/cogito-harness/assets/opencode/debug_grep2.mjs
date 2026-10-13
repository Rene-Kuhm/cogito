// Test what the grep key looks like
const pattern = 'handle_request';
const path = undefined;
const glob = undefined;
const key = `grep:${pattern}:${path || ""}:${glob || ""}`;
console.error('key:', JSON.stringify(key));
console.error('key includes :?', key.includes(':'));
console.error('key.split(\':\').length:', key.split(':').length);

// Test what the heredoc mock looks like
const nudgeFile = '/tmp/cogito-test-project/.cogito/nudges/s4';
const mockCmd = `cat >> "${nudgeFile}" << 'EOF'
${key}
EOF`;
console.error('mockCmd:', JSON.stringify(mockCmd));
console.error('mockCmd lines:', mockCmd.split('\n'));

// And what the actual plugin command looks like (simulate)
const actualCmd = `cat >> "${nudgeFile}" << 'EOF'
${key}
EOF`;
console.error('actualCmd:', JSON.stringify(actualCmd));
console.error('Match:', mockCmd === actualCmd);
