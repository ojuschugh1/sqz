/**
 * statusBar.ts — Status bar widget showing token budget usage.
 *
 * Requirement 6.4: display current token budget usage in the editor status bar.
 */

import * as vscode from "vscode";
import { SqzBridge } from "./sqzBridge";

export class SqzStatusBar {
  private item: vscode.StatusBarItem;
  private bridge: SqzBridge;
  private sessionId: string;
  private timer: ReturnType<typeof setInterval> | undefined;

  constructor(bridge: SqzBridge, sessionId: string) {
    this.bridge = bridge;
    this.sessionId = sessionId;

    this.item = vscode.window.createStatusBarItem(
      vscode.StatusBarAlignment.Right,
      100
    );
    this.item.command = "sqz.status";
    this.item.tooltip = "sqz token budget — click for details";
    this.item.text = "sqz: 0%";
    this.item.show();
  }

  /** Update the status bar with the latest budget figures. */
  update(): void {
    try {
      const status = this.bridge.getBudgetStatus(this.sessionId);
      const pct = Math.round(status.percentUsed);
      const icon = pct >= 85 ? "$(warning)" : pct >= 70 ? "$(info)" : "$(pulse)";
      this.item.text = `${icon} sqz: ${pct}%`;
      this.item.tooltip = [
        `sqz token budget`,
        `Consumed: ${status.consumed.toLocaleString()} / ${status.windowSize.toLocaleString()} tokens`,
        `Available: ${status.available.toLocaleString()} tokens`,
        `Usage: ${pct}%`,
      ].join("\n");
    } catch {
      this.item.text = "sqz: --";
    }
  }

  /** Notify the status bar that a compression happened (updates immediately). */
  onCompression(tokensOriginal: number, tokensCompressed: number): void {
    const saved = tokensOriginal - tokensCompressed;
    const pct = tokensOriginal > 0
      ? Math.round((saved / tokensOriginal) * 100)
      : 0;
    this.item.text = `$(check) sqz: saved ${pct}%`;
    // Refresh to real budget after a short delay
    setTimeout(() => this.update(), 1_500);
  }

  /** Start polling the budget every `intervalMs` milliseconds. */
  startPolling(intervalMs = 30_000): void {
    this.stopPolling();
    this.update();
    this.timer = setInterval(() => this.update(), intervalMs);
  }

  /** Stop the polling timer. */
  stopPolling(): void {
    if (this.timer !== undefined) {
      clearInterval(this.timer);
      this.timer = undefined;
    }
  }

  /** Update the session ID (e.g. when the user changes it in settings). */
  setSessionId(sessionId: string): void {
    this.sessionId = sessionId;
    this.update();
  }

  dispose(): void {
    this.stopPolling();
    this.item.dispose();
  }
}
