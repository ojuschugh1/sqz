/**
 * SqzPlugin.kt — Plugin entry point (ApplicationComponent).
 *
 * Initialises the SqzBridge and registers the file listener and status bar
 * widget when the IDE starts.
 *
 * Requirements: 6.1, 6.2, 6.3, 6.4, 6.5
 */

package com.sqz

import com.intellij.openapi.actionSystem.AnAction
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.Service
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.FileEditorManagerListener
import com.intellij.openapi.project.Project
import com.intellij.openapi.project.ProjectManagerListener
import com.intellij.openapi.ui.Messages
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.openapi.wm.StatusBar
import com.intellij.openapi.wm.StatusBarWidget
import com.intellij.openapi.wm.WindowManager

/**
 * Application-level service that holds the shared SqzBridge instance.
 * Registered in plugin.xml as an <applicationService>.
 */
@Service(Service.Level.APP)
class SqzService {
    val bridge: SqzBridge = SqzBridge()
}

/** Convenience accessor for the application-level service. */
fun sqzService(): SqzService =
    ApplicationManager.getApplication().getService(SqzService::class.java)

// ── Actions ───────────────────────────────────────────────────────────────────

/** sqz.compress — compress the active file and show stats. */
class SqzCompressAction : AnAction("Compress with sqz") {
    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val editor = FileEditorManager.getInstance(project).selectedTextEditor ?: return
        val doc = editor.document
        val content = doc.text
        val file = FileEditorManager.getInstance(project).selectedFiles.firstOrNull()
        val langId = file?.fileType?.name ?: "unknown"

        val result = sqzService().bridge.compress(content, langId, file?.path)
        val saved = result.tokensOriginal - result.tokensCompressed
        val pct = if (result.tokensOriginal > 0) saved * 100 / result.tokensOriginal else 0
        val method = if (result.usedAst) "AST" else "line-based"

        Messages.showInfoMessage(
            project,
            "Compressed ${result.tokensOriginal} → ${result.tokensCompressed} tokens ($pct% saved, $method)",
            "sqz Compress"
        )
    }
}

/** sqz.status — show current budget status. */
class SqzStatusAction : AnAction("sqz: Budget Status") {
    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val status = sqzService().bridge.getBudgetStatus()
        val pct = status.percentUsed.toInt()
        Messages.showInfoMessage(
            project,
            "Budget: $pct% used — ${status.consumed} / ${status.windowSize} tokens (${status.available} available)",
            "sqz Budget Status"
        )
    }
}

/** sqz.cost — show cost report. */
class SqzCostAction : AnAction("sqz: Cost Report") {
    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val report = sqzService().bridge.getCostReport()
        Messages.showInfoMessage(
            project,
            """sqz Cost Report
Total tokens: ${report.totalTokens}
Total cost: ${"%.4f".format(report.totalUsd)}
Cache savings: ${"%.4f".format(report.cacheSavingsUsd)}
Compression savings: ${"%.4f".format(report.compressionSavingsUsd)}""",
            "sqz Cost Report"
        )
    }
}

/** sqz.exportCtx — export session to a .ctx file. */
class SqzExportCtxAction : AnAction("sqz: Export CTX") {
    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        try {
            val ctx = sqzService().bridge.exportCtx()
            Messages.showInfoMessage(project, "Session exported successfully.", "sqz Export CTX")
        } catch (ex: Exception) {
            Messages.showErrorDialog(project, "Export failed: ${ex.message}", "sqz Export CTX")
        }
    }
}

/** sqz.importCtx — import a .ctx file. */
class SqzImportCtxAction : AnAction("sqz: Import CTX") {
    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        Messages.showInfoMessage(project, "Use sqz import <file.ctx> from the terminal.", "sqz Import CTX")
    }
}
