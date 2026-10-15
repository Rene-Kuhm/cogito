/**
 * Cogito — Pi extension adapter.
 *
 * What this adds to Pi:
 *
 * 1. On `session_start` it loads the project card (rescanning only if `HEAD`
 *    moved) and puts it in the system prompt of every turn, so the model
 *    starts each answer already knowing the layout and how to look code up.
 * 2. Guardrails: whole-file reads and plain-name greps are handed to
 *    `cogito guard`, the same entry point Claude Code and opencode call, and
 *    its decision (show Cogito's answer instead, or let the call run) is
 *    returned. The rule lives in one place; this extension keeps no copy of
 *    it and no state of its own.
 * 3. It sets a status line showing what Cogito knows about the project.
 * 4. It registers `/cogito` with `status`, `map`, `scan`, `search`, `stats`
 *    and `doctor`, so the map is usable without leaving the session.
 *
 * Persistence is owned by the `cogito` binary. This extension does **not**
 * register the MCP server: Pi reads MCP servers from `mcp.json`, which
 * `cogito setup` writes. Nothing here spawns a second process tree.
 */

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

/**
 * The slice of Pi's session context this extension uses.
 *
 * Declared locally rather than imported so the extension keeps working across
 * Pi releases that add fields to the host context. The handler parameter stays
 * `any` and is narrowed here.
 */
interface CogitoContext {
  cwd?: string;
  hasUI?: boolean;
  ui?: {
    notify?: (message: string, severity?: string) => void;
    setStatus?: (key: string, text: string | undefined) => void;
  };
  /** Pi's session manager; `getSessionId()` keys the guardrail state. */
  sessionManager?: { getSessionId?: () => string };
}

/** Overridable for development builds. */
const COGITO_BIN = process.env.COGITO_BIN?.trim() || "cogito";

/** Never let a scan block the session for long. */
const SCAN_TIMEOUT_MS = 30_000;
const QUERY_TIMEOUT_MS = 10_000;

interface RunResult {
  ok: boolean;
  stdout: string;
  stderr: string;
}

/**
 * Run `cogito …` in `cwd`, capturing output.
 *
 * When `stdin` is given it is written to the child and the stream closed;
 * otherwise the child gets no stdin at all. Rejects are converted to a failed
 * `RunResult` so callers never need to handle a spawn error: a missing binary
 * is reported, not thrown.
 */
function run(
  args: string[],
  cwd: string,
  timeoutMs: number,
  stdin?: string,
): Promise<RunResult> {
  return new Promise((resolve) => {
    let stdout = "";
    let stderr = "";
    let settled = false;

    const finish = (result: RunResult) => {
      if (settled) return;
      settled = true;
      resolve(result);
    };

    let child;
    try {
      child = spawn(COGITO_BIN, args, {
        cwd,
        stdio: [stdin === undefined ? "ignore" : "pipe", "pipe", "pipe"],
        env: process.env,
      });
    } catch (error) {
      finish({ ok: false, stdout: "", stderr: String(error) });
      return;
    }

    if (stdin !== undefined && child.stdin) {
      // The child may close stdin early; that is not a guardrail failure.
      child.stdin.on("error", () => {});
      child.stdin.end(stdin);
    }

    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      finish({ ok: false, stdout, stderr: `${stderr}\ntimed out after ${timeoutMs}ms` });
    }, timeoutMs);

    child.stdout?.on("data", (chunk) => {
      stdout += String(chunk);
    });
    child.stderr?.on("data", (chunk) => {
      stderr += String(chunk);
    });
    child.on("error", (error) => {
      clearTimeout(timer);
      finish({ ok: false, stdout, stderr: String(error) });
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      finish({ ok: code === 0, stdout, stderr });
    });
  });
}

/** Whether the project already has a Cogito database. */
function hasState(cwd: string): boolean {
  return existsSync(join(cwd, ".cogito", "state.db"));
}

/**
 * Whether `cwd` looks like a project Cogito should touch.
 *
 * `cogito scan` creates `.cogito/` and walks the tree. Running it in `$HOME`,
 * or in any unrelated directory an agent happens to start in, would leave a
 * stray database behind and scan the whole tree to build it. Only directories
 * that are already a git repository or already have a `.cogito/` qualify.
 */
function isProject(cwd: string): boolean {
  // `existsSync` covers both cases: `.git` is a directory in a normal
  // checkout and a file (`gitdir: …`) in a worktree or submodule.
  return existsSync(join(cwd, ".git")) || existsSync(join(cwd, ".cogito"));
}

/** Collapse noisy stderr into a single line for the UI. */
function firstLine(text: string): string {
  return text.split("\n").map((l) => l.trim()).find(Boolean) ?? "";
}

/**
 * The project card for each working directory, computed once per session.
 *
 * It goes into the system prompt of every turn, so it must not change between
 * turns: an identical prefix is what lets the provider serve it from its
 * prompt cache. A new session is what refreshes it.
 */
const cards = new Map<string, Promise<string | undefined>>();

/** Longest a turn waits for the card before going ahead without it. */
const CARD_WAIT_MS = 5_000;

function loadCard(cwd: string): Promise<string | undefined> {
  let card = cards.get(cwd);
  if (!card) {
    // `map --card` reuses the stored index and rescans only if HEAD moved.
    card = run(["map", "--card"], cwd, SCAN_TIMEOUT_MS).then((r) =>
      r.ok && r.stdout.trim() ? r.stdout.trim() : undefined,
    );
    cards.set(cwd, card);
  }
  return card;
}

/**
 * Ask `cogito guard` about one read or grep call.
 *
 * The decision (outline a first whole read, turn away an unchanged repeat,
 * nudge a plain-name grep) lives in the guard, the same rule Claude Code and
 * opencode use, so this extension keeps no copy of it and no state. Any
 * failure — a missing binary, a non-zero exit, unparseable output — returns
 * undefined, because a guardrail must never break a tool call.
 */
async function guardrail(
  toolName: string,
  input: any,
  cwd: string,
  session: string,
): Promise<string | undefined> {
  if (!input || typeof input !== "object") return undefined;
  if (toolName !== "read" && toolName !== "grep") return undefined;
  const call = JSON.stringify({ tool: toolName, input, cwd, session });
  const result = await run(["guard"], cwd, QUERY_TIMEOUT_MS, call);
  if (!result.ok) return undefined;
  try {
    const answer = JSON.parse(result.stdout);
    if (answer?.block === true && typeof answer.reason === "string" && answer.reason) {
      return answer.reason;
    }
  } catch {
    /* A guardrail must never break a tool call. */
  }
  return undefined;
}

export default function registerCogito(pi: ExtensionAPI) {
  pi.on("tool_call", async (event: any, raw: any) => {
    const ctx = raw as CogitoContext;
    const cwd = ctx?.cwd;
    if (!cwd || !isProject(cwd) || event?.parentToolCallId) return;
    // Pi's session id keys the guard's state. The process id is the fallback
    // when an older host does not expose the session manager.
    const session = ctx?.sessionManager?.getSessionId?.() ?? `pi-${process.pid}`;
    try {
      const reason = await guardrail(String(event?.toolName ?? ""), event?.input, cwd, session);
      if (reason) return { block: true, reason };
    } catch {
      /* A guardrail must never break a tool call. */
    }
    return undefined;
  });

  // Put the project card in the system prompt, so the model knows the
  // project's layout and how to look things up without spending a tool call
  // on orientation.
  pi.on("before_agent_start", async (event: any, raw: any) => {
    const cwd = (raw as CogitoContext)?.cwd;
    if (!cwd || !isProject(cwd) || !event?.systemPromptOptions) return;
    const card = await Promise.race([
      loadCard(cwd),
      new Promise<undefined>((resolve) => setTimeout(() => resolve(undefined), CARD_WAIT_MS)),
    ]);
    if (!card) return;
    const options = event.systemPromptOptions;
    options.sections = { ...(options.sections ?? {}), cogito_project: card };
  });

  pi.on("session_start", (event: unknown, raw: any) => {
    void event;
    const ctx = raw as CogitoContext;
    const cwd = ctx.cwd;
    if (!cwd) return;

    // Refresh the stored fingerprint in the background. A scan can take a
    // moment on a large repo, and nothing in the session depends on it, so do
    // not await.
    if (!isProject(cwd)) {
      try {
        ctx.ui?.setStatus?.("cogito", "cogito: not a project");
      } catch {
        /* ignore */
      }
      return;
    }

    // A new session starts from a fresh card. The guard's per-session state
    // lives under `.cogito/nudges/` and is keyed by the session id.
    cards.delete(cwd);
    void (async () => {
      const card = await loadCard(cwd);
      try {
        ctx.ui?.setStatus?.("cogito", card ? "cogito: ready" : "cogito: unavailable");
      } catch {
        /* A disposed UI must not break the session. */
      }
    })();

    // Cheap synchronous hint so the status line is populated immediately.
    try {
      ctx.ui?.setStatus?.("cogito", hasState(cwd) ? "cogito: ready" : "cogito: scanning");
    } catch {
      /* ignore */
    }
  });

  pi.registerCommand("cogito", {
    description: "Cogito context harness: status, map, scan, search, stats, doctor",
    handler: async (args: string, raw: any) => {
      const ctx = raw as CogitoContext;
      const cwd = ctx.cwd;
      if (!cwd) return;
      const parts = (args ?? "").trim().split(/\s+/).filter(Boolean);
      const sub = parts[0] ?? "status";
      const rest = parts.slice(1);

      const report = (text: string) => {
        try {
          if (ctx.hasUI) ctx.ui?.notify?.(text, "info");
          else process.stdout.write(`${text}\n`);
        } catch {
          /* ignore */
        }
      };

      switch (sub) {
        case "scan": {
          if (!isProject(cwd)) {
            report(
              `${cwd} is not a project (no .git or .cogito). Refusing to create a fingerprint here.`,
            );
            return;
          }
          report("cogito: scanning…");
          const result = await run(["scan"], cwd, SCAN_TIMEOUT_MS);
          report(result.ok ? result.stdout.trim() : `cogito scan failed: ${firstLine(result.stderr)}`);
          return;
        }
        case "map": {
          if (!isProject(cwd)) {
            report(`${cwd} is not a project (no .git or .cogito).`);
            return;
          }
          const result = await run(["map"], cwd, SCAN_TIMEOUT_MS);
          report(result.ok ? result.stdout.trim() : `cogito map failed: ${firstLine(result.stderr)}`);
          return;
        }
        case "stats": {
          const result = await run(["stats"], cwd, QUERY_TIMEOUT_MS);
          report(result.ok ? result.stdout.trim() : `cogito stats failed: ${firstLine(result.stderr)}`);
          return;
        }
        case "search": {
          if (rest.length === 0) {
            report("usage: /cogito search <text>");
            return;
          }
          const result = await run(["search", rest.join(" ")], cwd, QUERY_TIMEOUT_MS);
          report(result.ok ? result.stdout.trim() : `cogito search failed: ${firstLine(result.stderr)}`);
          return;
        }
        case "doctor": {
          const result = await run(["doctor"], cwd, QUERY_TIMEOUT_MS);
          report(result.ok ? result.stdout.trim() : `cogito doctor failed: ${firstLine(result.stderr)}`);
          return;
        }
        case "status": {
          const result = await run(["doctor"], cwd, QUERY_TIMEOUT_MS);
          const state = hasState(cwd) ? "state.db present" : "no state.db yet";
          report(result.ok ? `${state}\n\n${result.stdout.trim()}` : `${state} — cogito unreachable: ${firstLine(result.stderr)}`);
          return;
        }
        default:
          report(
            [
              "Cogito commands:",
              "  /cogito status          what Cogito sees in this project",
              "  /cogito map             the project map the agent reads",
              "  /cogito scan            regenerate the code fingerprint",
              "  /cogito search <text>  find symbols, files and modules by words",
              "  /cogito stats           what the index holds",
              "  /cogito doctor          full diagnostics incl. agent integrations",
            ].join("\n"),
          );
      }
    },
  });
}
