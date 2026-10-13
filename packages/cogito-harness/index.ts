/**
 * Cogito — Pi extension adapter.
 *
 * What this adds to Pi:
 *
 * 1. On `session_start` it scans the project in the background, refreshing
 *    the fingerprint Cogito stores for it. The scan is bounded and never
 *    blocks the session.
 * 2. It sets a status line showing what Cogito knows about the project.
 * 3. It registers `/cogito` with `status`, `scan`, `query`, `stats` and
 *    `doctor`, so the knowledge base is usable without leaving the session.
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

export default function registerCogito(pi: ExtensionAPI) {
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

    void (async () => {
      const result = await run(["scan"], cwd, SCAN_TIMEOUT_MS);
      try {
        if (!result.ok) {
          const reason = firstLine(result.stderr) || "unavailable";
          ctx.ui?.setStatus?.("cogito", `cogito: ${reason}`);
          return;
        }
        ctx.ui?.setStatus?.("cogito", `cogito: ${hasState(cwd) ? "ready" : "scanned"}`);
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
    description: "Cogito context harness: status, scan, query, stats, doctor",
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
        case "stats": {
          const result = await run(["stats"], cwd, QUERY_TIMEOUT_MS);
          report(result.ok ? result.stdout.trim() : `cogito stats failed: ${firstLine(result.stderr)}`);
          return;
        }
        case "query": {
          if (rest.length === 0) {
            report("usage: /cogito query <text>");
            return;
          }
          const result = await run(["query", rest.join(" ")], cwd, QUERY_TIMEOUT_MS);
          report(result.ok ? result.stdout.trim() : `cogito query failed: ${firstLine(result.stderr)}`);
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
              "  /cogito scan            regenerate the code fingerprint",
              "  /cogito query <text>    search tags and architectural decisions",
              "  /cogito stats           stored tag/decision counts",
              "  /cogito doctor          full diagnostics incl. agent integrations",
            ].join("\n"),
          );
      }
    },
  });
}
