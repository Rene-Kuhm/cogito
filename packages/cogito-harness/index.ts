/**
 * Cogito — Pi extension adapter.
 *
 * What this adds to Pi:
 *
 * 1. On `session_start` it loads the project card (rescanning only if `HEAD`
 *    moved) and puts it in the system prompt of every turn, so the model
 *    starts each answer already knowing the layout and how to look code up.
 * 2. Guardrails: the first time the agent reads a large file whole, or greps
 *    for a plain name, it gets Cogito's outline or search result instead.
 *    Repeating the same call goes through, so the agent is never stuck.
 * 3. It sets a status line showing what Cogito knows about the project.
 * 4. It registers `/cogito` with `status`, `map`, `scan`, `search`, `stats`
 *    and `doctor`, so the map is usable without leaving the session.
 *
 * Persistence is owned by the `cogito` binary. This extension does **not**
 * register the MCP server: Pi reads MCP servers from `mcp.json`, which
 * `cogito setup` writes. Nothing here spawns a second process tree.
 */

import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { extname, isAbsolute, join, relative } from "node:path";
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
 * Rejects are converted to a failed `RunResult` so callers never need to
 * handle a spawn error: a missing binary is reported, not thrown.
 */
function run(args: string[], cwd: string, timeoutMs: number): Promise<RunResult> {
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
        stdio: ["ignore", "pipe", "pipe"],
        env: process.env,
      });
    } catch (error) {
      finish({ ok: false, stdout: "", stderr: String(error) });
      return;
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
 * Guardrails: the expensive ways to find your way around a project (reading a
 * whole large file, grepping for a name) get Cogito's answer instead, once.
 *
 * Each nudge fires at most once per call shape and session: repeating the
 * exact same call goes through. The agent is never blocked from doing what it
 * needs, it is only shown the cheap answer first.
 */
const nudged = new Set<string>();

/** Files longer than this are outlined before they are read whole. */
const OUTLINE_MIN_LINES = 250;

/** Extensions Cogito can outline (code declarations, Markdown sections). */
const OUTLINED = new Set([
  ".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".rs", ".py", ".go",
  ".java", ".kt", ".kts", ".cs", ".swift", ".php", ".rb", ".md", ".mdx", ".markdown",
]);

/** A grep or find pattern that is a plain name, not a regex over content. */
function isPlainName(pattern: unknown): pattern is string {
  return typeof pattern === "string" && /^[\w.\-/ ]{2,80}$/.test(pattern.trim());
}

/** Count lines without holding more than the file in memory. */
function lineCount(file: string): number {
  try {
    const text = readFileSync(file, "utf8");
    let lines = 1;
    for (let i = 0; i < text.length; i++) if (text.charCodeAt(i) === 10) lines++;
    return lines;
  } catch {
    return 0;
  }
}

/**
 * Cogito's answer to show instead of the call, or undefined to let the call
 * run untouched.
 */
async function guardrail(toolName: string, input: any, cwd: string): Promise<string | undefined> {
  if (!input || typeof input !== "object") return undefined;

  if (toolName === "read") {
    const wholeFile = input.offset === undefined && (input.limit === undefined || input.limit > 400);
    if (!wholeFile || typeof input.path !== "string") return undefined;
    const file = isAbsolute(input.path) ? input.path : join(cwd, input.path);
    const rel = relative(cwd, file);
    if (rel.startsWith("..") || !OUTLINED.has(extname(file).toLowerCase())) return undefined;
    const key = `read:${rel}`;
    if (nudged.has(key)) return undefined;
    const lines = lineCount(file);
    if (lines < OUTLINE_MIN_LINES) return undefined;
    const result = await run(["outline", rel], cwd, QUERY_TIMEOUT_MS);
    if (!result.ok || !result.stdout.trim()) return undefined;
    nudged.add(key);
    return (
      `${rel} has ${lines} lines. Cogito's outline instead of the whole file:\n\n` +
      `${result.stdout.trim()}\n\n` +
      `Read the range you need with offset/limit. Reading the whole file again is allowed if you really need it.`
    );
  }

  if (toolName === "grep" || toolName === "find") {
    const pattern = input.pattern ?? input.name;
    if (!isPlainName(pattern)) return undefined;
    const key = `${toolName}:${pattern}:${input.path ?? ""}:${input.glob ?? ""}`;
    if (nudged.has(key)) return undefined;
    const result = await run(["search", pattern], cwd, QUERY_TIMEOUT_MS);
    if (!result.ok || !result.stdout.trim() || result.stdout.startsWith("Nothing matches")) {
      return undefined;
    }
    nudged.add(key);
    return (
      `Cogito's index for "${pattern}" (symbols and doc sections with file:line, files, modules):\n\n` +
      `${result.stdout.trim()}\n\n` +
      `Use file_outline on a result and read only its range. If you need every textual match, ` +
      `repeat the same ${toolName} call and it will run.`
    );
  }

  return undefined;
}

export default function registerCogito(pi: ExtensionAPI) {
  pi.on("tool_call", async (event: any, raw: any) => {
    const cwd = (raw as CogitoContext)?.cwd;
    if (!cwd || !isProject(cwd) || event?.parentToolCallId) return;
    try {
      const reason = await guardrail(String(event?.toolName ?? ""), event?.input, cwd);
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

    // A new session starts from a fresh card and fresh nudges.
    cards.delete(cwd);
    nudged.clear();
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
