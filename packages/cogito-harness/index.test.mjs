// Executable test for the Pi Cogito extension.
// Run with: node --test packages/cogito-harness/index.test.mjs
//
// The extension delegates every guardrail decision to `cogito guard` and
// keeps no state of its own, so these tests drive the `tool_call` handler with
// a stub `cogito` and cover the delegation contract:
// - a block is transmitted with its reason, and the call carries the tool,
//   input, cwd and session id the guard needs
// - block=false lets the call through
// - a non-zero exit, unparseable output, or a missing binary lets it through
// - non-read/grep tools and nested tool calls are never sent to the guard

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync, chmodSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const projectDir = mkdtempSync(join(tmpdir(), "cogito-pi-test-"));
const stubPath = join(projectDir, "cogito-stub.sh");
const capturePath = join(projectDir, "captured-stdin.json");

const STUB = `#!/bin/sh
cat > "${capturePath}"
case "$STUB_MODE" in
  block) printf '%s' '{"block":true,"reason":"stub blocked the read"}' ;;
  allow) printf '%s' '{"block":false}' ;;
  junk) printf '%s' 'not json at all' ;;
  fail) exit 3 ;;
  *) printf '%s' '{"block":false}' ;;
esac
`;

let handler;

/** Capture the `tool_call` handler from a minimal fake Pi API. */
function fakePi() {
  const handlers = {};
  return {
    handlers,
    on(event, fn) {
      handlers[event] = fn;
      return () => {};
    },
    registerCommand() {},
  };
}

const ctx = {
  cwd: projectDir,
  sessionManager: { getSessionId: () => "s-test" },
};

before(async () => {
  // `isProject` only checks that `.git` exists; a directory is enough.
  mkdirSync(join(projectDir, ".git"), { recursive: true });
  writeFileSync(stubPath, STUB);
  chmodSync(stubPath, 0o755);
  process.env.COGITO_BIN = stubPath;

  const { default: registerCogito } = await import("./index.ts");
  const pi = fakePi();
  registerCogito(pi);
  handler = pi.handlers["tool_call"];
  assert.equal(typeof handler, "function", "extension registered a tool_call handler");
});

after(() => {
  rmSync(projectDir, { recursive: true, force: true });
});

test("a block is transmitted with its reason and the guard payload", async () => {
  process.env.STUB_MODE = "block";
  const result = await handler(
    { toolName: "read", input: { path: "big.ts" } },
    ctx,
  );
  assert.deepEqual(result, { block: true, reason: "stub blocked the read" });

  const sent = JSON.parse(readFileSync(capturePath, "utf8"));
  assert.deepEqual(sent, {
    tool: "read",
    input: { path: "big.ts" },
    cwd: projectDir,
    session: "s-test",
  });
});

test("block=false lets the call through", async () => {
  process.env.STUB_MODE = "allow";
  const result = await handler(
    { toolName: "grep", input: { pattern: "handle_request" } },
    ctx,
  );
  assert.equal(result, undefined);
});

test("a non-zero exit lets the call through", async () => {
  process.env.STUB_MODE = "fail";
  const result = await handler(
    { toolName: "read", input: { path: "big.ts" } },
    ctx,
  );
  assert.equal(result, undefined);
});

test("unparseable output lets the call through", async () => {
  process.env.STUB_MODE = "junk";
  const result = await handler(
    { toolName: "read", input: { path: "big.ts" } },
    ctx,
  );
  assert.equal(result, undefined);
});

test("a missing binary lets the call through", async () => {
  process.env.STUB_MODE = "block";
  rmSync(stubPath, { force: true });
  const result = await handler(
    { toolName: "read", input: { path: "big.ts" } },
    ctx,
  );
  assert.equal(result, undefined);
});

test("tools other than read and grep are never sent to the guard", async () => {
  // The stub is gone after the previous test; the handler must return before
  // reaching it.
  const result = await handler(
    { toolName: "write", input: { path: "big.ts" } },
    ctx,
  );
  assert.equal(result, undefined);
});

test("nested tool calls are skipped", async () => {
  const result = await handler(
    { toolName: "read", input: { path: "big.ts" }, parentToolCallId: "p1" },
    ctx,
  );
  assert.equal(result, undefined);
});
