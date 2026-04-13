/**
 * sqzBridge.ts — Bridge to the sqz CLI binary.
 *
 * Calls the `sqz` CLI via child_process.execSync. This is the implementation
 * path; the production N-API native module approach is documented in the spec
 * design but the CLI bridge satisfies all requirements (the CLI wraps sqz_engine).
 *
 * Requirements: 6.1, 6.2, 6.3
 */

import { execSync, ExecSyncOptionsWithStringEncoding } from "child_process";
import * as path from "path";
import * as fs from "fs";
import * as os from "os";

export interface CompressResult {
  compressed: string;
  tokensOriginal: number;
  tokensCompressed: number;
  compressionRatio: number;
  language: string;
  usedAst: boolean;
}

export interface BudgetStatus {
  consumed: number;
  windowSize: number;
  percentUsed: number;
  available: number;
}

export interface CostReport {
  totalTokens: number;
  totalUsd: number;
  cacheSavingsUsd: number;
  compressionSavingsUsd: number;
}

/** Languages supported by the sqz AST parser (tree-sitter + regex extractors). */
export const AST_SUPPORTED_LANGUAGES = new Set([
  "rust",
  "python",
  "javascript",
  "typescript",
  "go",
  "java",
  "c",
  "cpp",
  "ruby",
  "bash",
  "json",
  "html",
  "css",
  "csharp",
  "kotlin",
  "swift",
  "toml",
  "yaml",
]);

/** Map VS Code language IDs to sqz language identifiers. */
export function vscodeLanguageToSqz(vscodeId: string): string | null {
  const map: Record<string, string> = {
    rust: "rust",
    python: "python",
    javascript: "javascript",
    javascriptreact: "javascript",
    typescript: "typescript",
    typescriptreact: "typescript",
    go: "go",
    java: "java",
    c: "c",
    cpp: "cpp",
    ruby: "ruby",
    shellscript: "bash",
    json: "json",
    jsonc: "json",
    html: "html",
    css: "css",
    csharp: "csharp",
    kotlin: "kotlin",
    swift: "swift",
    toml: "toml",
    yaml: "yaml",
  };
  return map[vscodeId] ?? null;
}

/** Returns true if the VS Code language ID is supported by the AST parser. */
export function isAstSupported(vscodeLanguageId: string): boolean {
  const sqzLang = vscodeLanguageToSqz(vscodeLanguageId);
  return sqzLang !== null && AST_SUPPORTED_LANGUAGES.has(sqzLang);
}

/**
 * Line-based compression fallback for unsupported languages.
 * Removes blank lines and trims trailing whitespace.
 * Requirement 6.5: fallback to line-based compression.
 */
export function lineFallbackCompress(content: string): CompressResult {
  const lines = content.split("\n");
  const compressed = lines
    .map((l) => l.trimEnd())
    .filter((l) => l.length > 0)
    .join("\n");

  const tokensOriginal = Math.ceil(content.length / 4);
  const tokensCompressed = Math.ceil(compressed.length / 4);

  return {
    compressed,
    tokensOriginal,
    tokensCompressed,
    compressionRatio:
      tokensOriginal > 0 ? tokensCompressed / tokensOriginal : 1.0,
    language: "unknown",
    usedAst: false,
  };
}

export class SqzBridge {
  private binaryPath: string;

  constructor(binaryPath = "sqz") {
    this.binaryPath = binaryPath;
  }

  private exec(args: string[], input?: string): string {
    const opts: ExecSyncOptionsWithStringEncoding = {
      encoding: "utf8",
      timeout: 10_000,
      input,
    };
    try {
      return execSync(`${this.binaryPath} ${args.join(" ")}`, opts).trim();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      throw new Error(`sqz CLI error: ${msg}`);
    }
  }

  /** Check whether the sqz binary is available. */
  isAvailable(): boolean {
    try {
      this.exec(["--version"]);
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Compress content using the sqz CLI.
   * Falls back to line-based compression if the CLI is unavailable or the
   * language is not supported by the AST parser.
   *
   * Requirements: 6.2, 6.3, 6.5
   */
  compress(
    content: string,
    vscodeLanguageId: string,
    filePath?: string
  ): CompressResult {
    const sqzLang = vscodeLanguageToSqz(vscodeLanguageId);
    const astSupported = sqzLang !== null && AST_SUPPORTED_LANGUAGES.has(sqzLang);

    // If language is not AST-supported, use line-based fallback (Req 6.5)
    if (!astSupported) {
      return lineFallbackCompress(content);
    }

    // Try CLI-based compression
    if (!this.isAvailable()) {
      // CLI not available — fall back to line-based
      console.warn("[sqz] CLI not available, falling back to line-based");
      return lineFallbackCompress(content);
    }

    try {
      // Pipe content via stdin to sqz compress
      const args = ["compress"];
      const output = this.exec(args, content);
      console.log("[sqz] CLI compress returned", output.length, "chars");

      // Strip the stats line (e.g. "[sqz] 6/9 tokens (33% reduction)")
      const lines = output.split("\n");
      const compressed = lines.filter(l => !l.startsWith("[sqz]")).join("\n");

      const tokensOriginal = Math.ceil(content.length / 4);
      const tokensCompressed = Math.ceil(compressed.length / 4);

      return {
        compressed,
        tokensOriginal,
        tokensCompressed,
        compressionRatio:
          tokensOriginal > 0 ? tokensCompressed / tokensOriginal : 1.0,
        language: sqzLang ?? vscodeLanguageId,
        usedAst: true,
      };
    } catch (err: unknown) {
      // CLI failed — fall back to line-based
      const msg = err instanceof Error ? err.message : String(err);
      console.warn("[sqz] CLI compress failed:", msg);
      return lineFallbackCompress(content);
    }
  }

  /**
   * Get the current budget status for a session.
   * Requirement 6.4: display token budget usage in status bar.
   */
  getBudgetStatus(sessionId = "default"): BudgetStatus {
    if (!this.isAvailable()) {
      return { consumed: 0, windowSize: 200_000, percentUsed: 0, available: 200_000 };
    }

    try {
      const output = this.exec(["status", "--session", sessionId, "--json"]);
      const data = JSON.parse(output) as Partial<BudgetStatus>;
      const consumed = data.consumed ?? 0;
      const windowSize = data.windowSize ?? 200_000;
      return {
        consumed,
        windowSize,
        percentUsed: windowSize > 0 ? (consumed / windowSize) * 100 : 0,
        available: windowSize - consumed,
      };
    } catch {
      return { consumed: 0, windowSize: 200_000, percentUsed: 0, available: 200_000 };
    }
  }

  /**
   * Get the cost report for a session.
   * Requirement 6.4 / 22.x
   */
  getCostReport(sessionId = "default"): CostReport {
    if (!this.isAvailable()) {
      return { totalTokens: 0, totalUsd: 0, cacheSavingsUsd: 0, compressionSavingsUsd: 0 };
    }

    try {
      const output = this.exec(["cost", "--session", sessionId, "--json"]);
      const data = JSON.parse(output) as Partial<CostReport>;
      return {
        totalTokens: data.totalTokens ?? 0,
        totalUsd: data.totalUsd ?? 0,
        cacheSavingsUsd: data.cacheSavingsUsd ?? 0,
        compressionSavingsUsd: data.compressionSavingsUsd ?? 0,
      };
    } catch {
      return { totalTokens: 0, totalUsd: 0, cacheSavingsUsd: 0, compressionSavingsUsd: 0 };
    }
  }

  /**
   * Export the current session to a .ctx file.
   * Requirement 7.1
   */
  exportCtx(sessionId = "default"): string {
    return this.exec(["export", "--session", sessionId]);
  }

  /**
   * Import a .ctx file.
   * Requirement 7.3
   */
  importCtx(ctxContent: string): void {
    const tmpFile = path.join(
      os.tmpdir(),
      `sqz_import_${Date.now()}.ctx`
    );
    fs.writeFileSync(tmpFile, ctxContent, "utf8");
    try {
      this.exec(["import", tmpFile]);
    } finally {
      try {
        fs.unlinkSync(tmpFile);
      } catch {
        // ignore
      }
    }
  }
}
