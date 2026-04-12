/**
 * SqzFileListener.kt — VirtualFileListener that fires when files are opened.
 *
 * Intercepts file-open events, runs the content through SqzBridge, and
 * updates the status bar widget with compression stats.
 *
 * Requirements: 6.2, 6.3, 6.5
 */

package com.sqz

import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.FileEditorManagerListener
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.openapi.wm.WindowManager

/**
 * Listens for file-open events in the editor.
 * Registered as a project-level message bus listener in plugin.xml.
 */
class SqzFileListener(private val project: Project) : FileEditorManagerListener {

    override fun fileOpened(source: FileEditorManager, file: VirtualFile) {
        // Skip non-local files (e.g. remote, jar entries)
        if (!file.isInLocalFileSystem) return

        val content = try {
            String(file.contentsToByteArray(), Charsets.UTF_8)
        } catch (_: Exception) {
            return
        }

        // Skip very small files
        if (content.length < 200) return

        val bridge = sqzService().bridge
        val langId = file.fileType.name
        val result = bridge.compress(content, langId, file.path)

        // Update status bar widget
        val statusBar = WindowManager.getInstance().getStatusBar(project)
        val widget = statusBar?.getWidget(SqzStatusBarWidget.ID) as? SqzStatusBarWidget
        widget?.onCompression(result.tokensOriginal, result.tokensCompressed)

        val saved = result.tokensOriginal - result.tokensCompressed
        val pct = if (result.tokensOriginal > 0) saved * 100 / result.tokensOriginal else 0
        val method = if (result.usedAst) "AST" else "line-based"

        // Log to IDE status bar message (transient)
        statusBar?.info = "sqz: ${file.name} — ${result.tokensOriginal}→${result.tokensCompressed} tokens ($pct% saved, $method)"
    }
}
