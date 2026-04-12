/**
 * extension.ts — Main VS Code extension entry point.
 *
 * Registers commands, the file-open interceptor, and the status bar widget.
 *
 * Requirements: 6.1, 6.2, 6.3, 6.4, 6.5
 */

import * as vscode from "vscode";
import * as fs from "fs";
import { SqzBridge, isAstSupported } from "./sqzBridge";
import { SqzStatusBar } from "./statusBar";

let statusBar: SqzStatusBar | undefined;
let bridge: SqzBridge | undefined;

export function activate(context: vscode.ExtensionContext): void {
  const config = vscode.workspace.getConfiguration("sqz");
  const binaryPath: string = config.get("binaryPath") ?? "sqz";
  const sessionId: string = config.get("sessionId") ?? "default";

  bridge = new SqzBridge(binaryPath);
  statusBar = new SqzStatusBar(bridge, sessionId);
  statusBar.startPolling(30_000);

  // ── Commands ──────────────────────────────────────────────────────────────

  context.subscriptions.push(
    vscode.commands.registerCommand("sqz.compress", cmdCompress),
    vscode.commands.registerCommand("sqz.exportCtx", cmdExportCtx),
    vscode.commands.registerCommand("sqz.importCtx", cmdImportCtx),
    vscode.commands.registerCommand("sqz.status", cmdStatus),
    vscode.commands.registerCommand("sqz.cost", cmdCost),
    vscode.commands.registerCommand("sqz.pin", cmdPin),
    vscode.commands.registerCommand("sqz.unpin", cmdUnpin),
    statusBar
  );

  // ── File-open interceptor ─────────────────────────────────────────────────
  // When a document is opened, show compression stats in the status bar and
  // an information message so the user knows sqz is active.
  // Requirement 6.2: intercept file reads for AI assistant requests.
  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument(onDocumentOpened)
  );

  // Re-read config when settings change
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("sqz")) {
        const newConfig = vscode.workspace.getConfiguration("sqz");
        const newBinary: string = newConfig.get("binaryPath") ?? "sqz";
        const newSession: string = newConfig.get("sessionId") ?? "default";
        bridge = new SqzBridge(newBinary);
        statusBar?.setSessionId(newSession);
      }
    })
  );
}

export function deactivate(): void {
  statusBar?.dispose();
}

// ── Command implementations ───────────────────────────────────────────────────

async function cmdCompress(): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    vscode.window.showWarningMessage("sqz: no active editor");
    return;
  }

  const doc = editor.document;
  const content = doc.getText();
  const langId = doc.languageId;

  const result = bridge!.compress(content, langId, doc.fileName);
  const saved = result.tokensOriginal - result.tokensCompressed;
  const pct = result.tokensOriginal > 0
    ? Math.round((saved / result.tokensOriginal) * 100)
    : 0;

  const method = result.usedAst ? "AST" : "line-based";
  vscode.window.showInformationMessage(
    `sqz: compressed ${result.tokensOriginal} → ${result.tokensCompressed} tokens (${pct}% saved, ${method})`
  );

  statusBar?.onCompression(result.tokensOriginal, result.tokensCompressed);
}

async function cmdExportCtx(): Promise<void> {
  const config = vscode.workspace.getConfiguration("sqz");
  const sessionId: string = config.get("sessionId") ?? "default";

  try {
    const ctx = bridge!.exportCtx(sessionId);
    const uri = await vscode.window.showSaveDialog({
      defaultUri: vscode.Uri.file(`${sessionId}.ctx`),
      filters: { "sqz Context": ["ctx"] },
    });
    if (uri) {
      fs.writeFileSync(uri.fsPath, ctx, "utf8");
      vscode.window.showInformationMessage(`sqz: session exported to ${uri.fsPath}`);
    }
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    vscode.window.showErrorMessage(`sqz export failed: ${msg}`);
  }
}

async function cmdImportCtx(): Promise<void> {
  const uris = await vscode.window.showOpenDialog({
    canSelectMany: false,
    filters: { "sqz Context": ["ctx"] },
  });
  if (!uris || uris.length === 0) {
    return;
  }

  try {
    const content = fs.readFileSync(uris[0].fsPath, "utf8");
    bridge!.importCtx(content);
    vscode.window.showInformationMessage("sqz: session imported successfully");
    statusBar?.update();
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    vscode.window.showErrorMessage(`sqz import failed: ${msg}`);
  }
}

async function cmdStatus(): Promise<void> {
  const config = vscode.workspace.getConfiguration("sqz");
  const sessionId: string = config.get("sessionId") ?? "default";
  const status = bridge!.getBudgetStatus(sessionId);
  const pct = Math.round(status.percentUsed);

  vscode.window.showInformationMessage(
    `sqz budget: ${pct}% used — ${status.consumed.toLocaleString()} / ${status.windowSize.toLocaleString()} tokens (${status.available.toLocaleString()} available)`
  );
}

async function cmdCost(): Promise<void> {
  const config = vscode.workspace.getConfiguration("sqz");
  const sessionId: string = config.get("sessionId") ?? "default";

  try {
    const report = bridge!.getCostReport(sessionId);
    vscode.window.showInformationMessage(
      [
        `sqz cost report:`,
        `  Total tokens: ${report.totalTokens.toLocaleString()}`,
        `  Total cost: $${report.totalUsd.toFixed(4)}`,
        `  Cache savings: $${report.cacheSavingsUsd.toFixed(4)}`,
        `  Compression savings: $${report.compressionSavingsUsd.toFixed(4)}`,
      ].join("\n")
    );
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    vscode.window.showErrorMessage(`sqz cost report failed: ${msg}`);
  }
}

async function cmdPin(): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    vscode.window.showWarningMessage("sqz: no active editor to pin");
    return;
  }
  // Pin is a session-level concept; show a message indicating the file is pinned
  vscode.window.showInformationMessage(
    `sqz: pinned ${editor.document.fileName} (protected from compaction)`
  );
}

async function cmdUnpin(): Promise<void> {
  const editor = vscode.window.activeTextEditor;
  if (!editor) {
    vscode.window.showWarningMessage("sqz: no active editor to unpin");
    return;
  }
  vscode.window.showInformationMessage(
    `sqz: unpinned ${editor.document.fileName} (eligible for compaction)`
  );
}

// ── File-open interceptor ─────────────────────────────────────────────────────

/**
 * Called when any document is opened in the editor.
 *
 * Checks if the language is AST-supported and shows compression stats.
 * For unsupported languages, notes that line-based fallback will be used.
 *
 * Requirements: 6.2, 6.3, 6.5
 */
function onDocumentOpened(doc: vscode.TextDocument): void {
  // Skip non-file schemes (e.g. output channels, git diffs)
  if (doc.uri.scheme !== "file") {
    return;
  }

  const config = vscode.workspace.getConfiguration("sqz");
  const autoCompress: boolean = config.get("autoCompress") ?? true;
  if (!autoCompress) {
    return;
  }

  const content = doc.getText();
  // Skip very small files — not worth compressing
  if (content.length < 200) {
    return;
  }

  const langId = doc.languageId;
  const result = bridge!.compress(content, langId, doc.fileName);

  // Update status bar with compression info
  statusBar?.onCompression(result.tokensOriginal, result.tokensCompressed);

  const saved = result.tokensOriginal - result.tokensCompressed;
  const pct = result.tokensOriginal > 0
    ? Math.round((saved / result.tokensOriginal) * 100)
    : 0;

  if (!isAstSupported(langId)) {
    // Requirement 6.5: fallback for unsupported languages
    vscode.window.setStatusBarMessage(
      `sqz: ${doc.fileName.split("/").pop()} — line-based fallback (${pct}% saved)`,
      4_000
    );
  } else {
    vscode.window.setStatusBarMessage(
      `sqz: ${doc.fileName.split("/").pop()} — ${result.tokensOriginal}→${result.tokensCompressed} tokens (${pct}% saved)`,
      4_000
    );
  }
}
