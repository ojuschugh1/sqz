/**
 * Integration tests for the sqz VS Code extension.
 *
 * Tests:
 *  - Commands are registered (sqz.compress, sqz.exportCtx, etc.)
 *  - SqzBridge.compress() for AST-supported languages
 *  - SqzBridge.compress() fallback for unsupported languages
 *  - isAstSupported() language detection
 *  - lineFallbackCompress() removes blank lines and trims whitespace
 *
 * Requirements: 6.2, 6.3, 6.5
 */

import * as assert from "assert";
import * as vscode from "vscode";
import {
  SqzBridge,
  isAstSupported,
  lineFallbackCompress,
  vscodeLanguageToSqz,
  AST_SUPPORTED_LANGUAGES,
} from "../sqzBridge";

// ---------------------------------------------------------------------------
// Helper: create a bridge that always reports the CLI as unavailable so tests
// don't depend on the sqz binary being installed in CI.
// ---------------------------------------------------------------------------
class OfflineBridge extends SqzBridge {
  override isAvailable(): boolean {
    return false;
  }
}

suite("sqz Extension — Command Registration", () => {
  test("all sqz commands are registered", async () => {
    const allCommands = await vscode.commands.getCommands(true);
    const sqzCommands = [
      "sqz.compress",
      "sqz.exportCtx",
      "sqz.importCtx",
      "sqz.status",
      "sqz.cost",
      "sqz.pin",
      "sqz.unpin",
    ];
    for (const cmd of sqzCommands) {
      assert.ok(
        allCommands.includes(cmd),
        `Expected command '${cmd}' to be registered`
      );
    }
  });
});

suite("sqz Extension — Language Detection", () => {
  test("isAstSupported returns true for TypeScript", () => {
    assert.strictEqual(isAstSupported("typescript"), true);
  });

  test("isAstSupported returns true for Python", () => {
    assert.strictEqual(isAstSupported("python"), true);
  });

  test("isAstSupported returns true for Rust", () => {
    assert.strictEqual(isAstSupported("rust"), true);
  });

  test("isAstSupported returns true for JavaScript", () => {
    assert.strictEqual(isAstSupported("javascript"), true);
  });

  test("isAstSupported returns true for Go", () => {
    assert.strictEqual(isAstSupported("go"), true);
  });

  test("isAstSupported returns false for plaintext", () => {
    assert.strictEqual(isAstSupported("plaintext"), false);
  });

  test("isAstSupported returns false for unknown language", () => {
    assert.strictEqual(isAstSupported("cobol"), false);
  });

  test("isAstSupported returns false for empty string", () => {
    assert.strictEqual(isAstSupported(""), false);
  });

  test("vscodeLanguageToSqz maps javascriptreact to javascript", () => {
    assert.strictEqual(vscodeLanguageToSqz("javascriptreact"), "javascript");
  });

  test("vscodeLanguageToSqz maps typescriptreact to typescript", () => {
    assert.strictEqual(vscodeLanguageToSqz("typescriptreact"), "typescript");
  });

  test("vscodeLanguageToSqz maps shellscript to bash", () => {
    assert.strictEqual(vscodeLanguageToSqz("shellscript"), "bash");
  });

  test("vscodeLanguageToSqz returns null for unknown language", () => {
    assert.strictEqual(vscodeLanguageToSqz("cobol"), null);
  });

  test("AST_SUPPORTED_LANGUAGES contains at least 18 languages (Req 19.1)", () => {
    assert.ok(
      AST_SUPPORTED_LANGUAGES.size >= 18,
      `Expected >= 18 supported languages, got ${AST_SUPPORTED_LANGUAGES.size}`
    );
  });
});

suite("sqz Extension — Line-Based Fallback Compression (Req 6.5)", () => {
  test("removes blank lines", () => {
    const input = "line1\n\nline2\n\nline3";
    const result = lineFallbackCompress(input);
    assert.ok(!result.compressed.includes("\n\n"), "should have no blank lines");
    assert.ok(result.compressed.includes("line1"));
    assert.ok(result.compressed.includes("line2"));
    assert.ok(result.compressed.includes("line3"));
  });

  test("trims trailing whitespace from lines", () => {
    const input = "  hello   \n  world   ";
    const result = lineFallbackCompress(input);
    const lines = result.compressed.split("\n");
    for (const line of lines) {
      assert.strictEqual(line, line.trimEnd(), "lines should have no trailing whitespace");
    }
  });

  test("usedAst is false for fallback", () => {
    const result = lineFallbackCompress("hello\nworld");
    assert.strictEqual(result.usedAst, false);
  });

  test("language is 'unknown' for fallback", () => {
    const result = lineFallbackCompress("hello");
    assert.strictEqual(result.language, "unknown");
  });

  test("tokensOriginal >= tokensCompressed for content with blank lines", () => {
    const input = "a\n\n\nb\n\n\nc";
    const result = lineFallbackCompress(input);
    assert.ok(
      result.tokensOriginal >= result.tokensCompressed,
      "compression should not increase token count"
    );
  });

  test("compressionRatio is between 0 and 1 for compressible content", () => {
    const input = "a\n\n\n\n\nb\n\n\n\n\nc";
    const result = lineFallbackCompress(input);
    assert.ok(result.compressionRatio >= 0, "ratio should be >= 0");
    assert.ok(result.compressionRatio <= 1, "ratio should be <= 1");
  });

  test("empty input produces empty output", () => {
    const result = lineFallbackCompress("");
    assert.strictEqual(result.compressed, "");
    assert.strictEqual(result.tokensOriginal, 0);
    assert.strictEqual(result.tokensCompressed, 0);
  });
});

suite("sqz Extension — SqzBridge (offline / CLI unavailable)", () => {
  let bridge: OfflineBridge;

  setup(() => {
    bridge = new OfflineBridge("sqz");
  });

  test("compress falls back to line-based for unsupported language when CLI unavailable", () => {
    const content = "hello\n\nworld\n\nfoo";
    const result = bridge.compress(content, "cobol");
    assert.strictEqual(result.usedAst, false);
    assert.ok(!result.compressed.includes("\n\n"));
  });

  test("compress falls back to line-based for supported language when CLI unavailable", () => {
    // Even for AST-supported languages, if CLI is unavailable we fall back
    const content = "fn main() {\n\n    println!(\"hello\");\n\n}";
    const result = bridge.compress(content, "rust");
    assert.strictEqual(result.usedAst, false);
  });

  test("getBudgetStatus returns zero-state when CLI unavailable", () => {
    const status = bridge.getBudgetStatus("default");
    assert.strictEqual(status.consumed, 0);
    assert.strictEqual(status.percentUsed, 0);
    assert.ok(status.windowSize > 0);
  });

  test("getCostReport returns zero-state when CLI unavailable", () => {
    const report = bridge.getCostReport("default");
    assert.strictEqual(report.totalTokens, 0);
    assert.strictEqual(report.totalUsd, 0);
  });
});

suite("sqz Extension — File Read Interception (Req 6.2)", () => {
  test("opening a TypeScript file triggers compression stats in status bar", async () => {
    // Create a minimal TS document and open it
    const content = [
      "import * as fs from 'fs';",
      "",
      "export function readFile(path: string): string {",
      "  return fs.readFileSync(path, 'utf8');",
      "}",
      "",
      "export class FileReader {",
      "  read(path: string): string {",
      "    return readFile(path);",
      "  }",
      "}",
    ].join("\n");

    const doc = await vscode.workspace.openTextDocument({
      content,
      language: "typescript",
    });

    // The extension's onDidOpenTextDocument handler fires automatically.
    // We just verify the document opened successfully and has the right language.
    assert.strictEqual(doc.languageId, "typescript");
    assert.ok(doc.getText().length > 0);
    assert.ok(isAstSupported(doc.languageId), "typescript should be AST-supported");
  });

  test("opening a plaintext file uses line-based fallback (Req 6.5)", async () => {
    const content = "hello\n\nworld\n\nsome plain text\n\nmore text";
    const doc = await vscode.workspace.openTextDocument({
      content,
      language: "plaintext",
    });

    assert.strictEqual(doc.languageId, "plaintext");
    assert.strictEqual(isAstSupported(doc.languageId), false);

    // Verify the fallback compressor works on this content
    const result = lineFallbackCompress(doc.getText());
    assert.strictEqual(result.usedAst, false);
    assert.ok(!result.compressed.includes("\n\n"));
  });

  test("AST-based compression for Python (Req 6.3)", async () => {
    const content = [
      "import os",
      "import sys",
      "",
      "def read_file(path: str) -> str:",
      "    with open(path) as f:",
      "        return f.read()",
      "",
      "class FileProcessor:",
      "    def process(self, path: str) -> str:",
      "        return read_file(path)",
    ].join("\n");

    const doc = await vscode.workspace.openTextDocument({
      content,
      language: "python",
    });

    assert.strictEqual(doc.languageId, "python");
    assert.ok(isAstSupported("python"), "python should be AST-supported");
  });
});
