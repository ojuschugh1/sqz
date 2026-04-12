/**
 * SqzStatusBarWidget.kt — Status bar widget showing token budget usage.
 *
 * Requirement 6.4: display current token budget usage in the editor status bar.
 */

package com.sqz

import com.intellij.openapi.project.Project
import com.intellij.openapi.wm.StatusBar
import com.intellij.openapi.wm.StatusBarWidget
import com.intellij.openapi.wm.StatusBarWidgetFactory
import com.intellij.util.Consumer
import java.awt.event.MouseEvent
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledFuture
import java.util.concurrent.TimeUnit

class SqzStatusBarWidget(private val project: Project) :
    StatusBarWidget, StatusBarWidget.TextPresentation {

    companion object {
        const val ID = "SqzStatusBarWidget"
    }

    private var statusBar: StatusBar? = null
    private var text: String = "sqz: 0%"
    private var tooltip: String = "sqz token budget"

    private val scheduler = Executors.newSingleThreadScheduledExecutor { r ->
        Thread(r, "sqz-status-bar-poller").also { it.isDaemon = true }
    }
    private var pollFuture: ScheduledFuture<*>? = null

    override fun ID(): String = ID

    override fun getPresentation(): StatusBarWidget.WidgetPresentation = this

    override fun install(statusBar: StatusBar) {
        this.statusBar = statusBar
        startPolling(30_000)
    }

    override fun dispose() {
        stopPolling()
        scheduler.shutdownNow()
    }

    // ── TextPresentation ──────────────────────────────────────────────────────

    override fun getText(): String = text

    override fun getTooltipText(): String = tooltip

    override fun getClickConsumer(): Consumer<MouseEvent> = Consumer {
        val status = sqzService().bridge.getBudgetStatus()
        val pct = status.percentUsed.toInt()
        com.intellij.openapi.ui.Messages.showInfoMessage(
            project,
            "Budget: $pct% used — ${status.consumed} / ${status.windowSize} tokens (${status.available} available)",
            "sqz Budget Status"
        )
    }

    override fun getAlignment(): Float = 0f

    // ── Polling ───────────────────────────────────────────────────────────────

    private fun startPolling(intervalMs: Long) {
        stopPolling()
        update()
        pollFuture = scheduler.scheduleAtFixedRate(::update, intervalMs, intervalMs, TimeUnit.MILLISECONDS)
    }

    private fun stopPolling() {
        pollFuture?.cancel(false)
        pollFuture = null
    }

    fun update() {
        try {
            val status = sqzService().bridge.getBudgetStatus()
            val pct = status.percentUsed.toInt()
            val icon = when {
                pct >= 85 -> "⚠ "
                pct >= 70 -> "ℹ "
                else -> ""
            }
            text = "${icon}sqz: $pct%"
            tooltip = buildString {
                appendLine("sqz token budget")
                appendLine("Consumed: ${status.consumed} / ${status.windowSize} tokens")
                appendLine("Available: ${status.available} tokens")
                append("Usage: $pct%")
            }
        } catch (_: Exception) {
            text = "sqz: --"
        }
        statusBar?.updateWidget(ID)
    }

    /** Called by SqzFileListener after a compression event. */
    fun onCompression(tokensOriginal: Int, tokensCompressed: Int) {
        val saved = tokensOriginal - tokensCompressed
        val pct = if (tokensOriginal > 0) saved * 100 / tokensOriginal else 0
        text = "✓ sqz: saved $pct%"
        statusBar?.updateWidget(ID)
        // Refresh to real budget after a short delay
        scheduler.schedule(::update, 1_500, TimeUnit.MILLISECONDS)
    }
}

/** Factory registered in plugin.xml to create the widget per project. */
class SqzStatusBarWidgetFactory : StatusBarWidgetFactory {
    override fun getId(): String = SqzStatusBarWidget.ID
    override fun getDisplayName(): String = "sqz Token Budget"
    override fun isAvailable(project: Project): Boolean = true
    override fun createWidget(project: Project): StatusBarWidget = SqzStatusBarWidget(project)
    override fun disposeWidget(widget: StatusBarWidget) = widget.dispose()
    override fun canBeEnabledOn(statusBar: StatusBar): Boolean = true
}
