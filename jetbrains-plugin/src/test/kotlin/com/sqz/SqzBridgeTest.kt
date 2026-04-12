/**
 * SqzBridgeTest.kt — Unit tests for SqzBridge.
 *
 * Tests:
 *  - lineFallbackCompress() for unsupported languages
 *  - isAstSupported() language detection
 *  - Bridge returns zero-state when CLI is unavailable
 *
 * Requirements: 6.2, 6.3, 6.5
 */

package com.sqz

import org.junit.jupiter.api.Assertions.*
import org.junit.jupiter.api.Test

class SqzBridgeTest {

    // ── lineFallbackCompress ──────────────────────────────────────────────────

    @Test
    fun `lineFallbackCompress removes blank lines`() {
        val input = "line one\n\nline two\n\n\nline three\n"
        val result = lineFallbackCompress(input)
        assertFalse(result.compressed.contains("\n\n"), "Should have no consecutive blank lines")
        assertTrue(result.compressed.contains("line one"))
        assertTrue(result.compressed.contains("line two"))
        assertTrue(result.compressed.contains("line three"))
    }

    @Test
    fun `lineFallbackCompress trims trailing whitespace`() {
        val input = "hello   \nworld  \n"
        val result = lineFallbackCompress(input)
        for (line in result.compressed.lines()) {
            assertEquals(line.trimEnd(), line, "Each line should have no trailing whitespace")
        }
    }

    @Test
    fun `lineFallbackCompress sets usedAst to false`() {
        val result = lineFallbackCompress("some content")
        assertFalse(result.usedAst)
    }

    @Test
    fun `lineFallbackCompress sets language to unknown`() {
        val result = lineFallbackCompress("some content")
        assertEquals("unknown", result.language)
    }

    @Test
    fun `lineFallbackCompress token counts are non-negative`() {
        val result = lineFallbackCompress("hello world")
        assertTrue(result.tokensOriginal >= 0)
        assertTrue(result.tokensCompressed >= 0)
    }

    @Test
    fun `lineFallbackCompress compressionRatio is at most 1 for non-empty input`() {
        val input = "line one\n\nline two\n\n\nline three\n"
        val result = lineFallbackCompress(input)
        assertTrue(result.compressionRatio <= 1.0, "Removing blank lines should not increase token count")
    }

    @Test
    fun `lineFallbackCompress handles empty string`() {
        val result = lineFallbackCompress("")
        assertEquals("", result.compressed)
        assertEquals(0, result.tokensOriginal)
        assertEquals(0, result.tokensCompressed)
        assertEquals(1.0, result.compressionRatio, 0.001)
    }

    @Test
    fun `lineFallbackCompress handles content with only blank lines`() {
        val result = lineFallbackCompress("\n\n\n")
        assertEquals("", result.compressed)
    }

    // ── isAstSupported ────────────────────────────────────────────────────────

    @Test
    fun `isAstSupported returns true for Kotlin`() {
        assertTrue(isAstSupported("Kotlin"))
    }

    @Test
    fun `isAstSupported returns true for JAVA`() {
        assertTrue(isAstSupported("JAVA"))
    }

    @Test
    fun `isAstSupported returns true for Python`() {
        assertTrue(isAstSupported("Python"))
    }

    @Test
    fun `isAstSupported returns true for Rust`() {
        assertTrue(isAstSupported("Rust"))
    }

    @Test
    fun `isAstSupported returns true for TypeScript`() {
        assertTrue(isAstSupported("TypeScript"))
    }

    @Test
    fun `isAstSupported returns true for JSON`() {
        assertTrue(isAstSupported("JSON"))
    }

    @Test
    fun `isAstSupported returns false for unknown language`() {
        assertFalse(isAstSupported("COBOL"))
    }

    @Test
    fun `isAstSupported returns false for empty string`() {
        assertFalse(isAstSupported(""))
    }

    @Test
    fun `isAstSupported returns false for Lua`() {
        assertFalse(isAstSupported("Lua"))
    }

    // ── Zero-state when CLI unavailable ───────────────────────────────────────

    @Test
    fun `getBudgetStatus returns zero-state when CLI unavailable`() {
        val bridge = SqzBridge(binaryPath = "sqz-nonexistent-binary-xyz")
        val status = bridge.getBudgetStatus()
        assertEquals(0, status.consumed)
        assertEquals(200_000, status.windowSize)
        assertEquals(0.0, status.percentUsed, 0.001)
        assertEquals(200_000, status.available)
    }

    @Test
    fun `getCostReport returns zero-state when CLI unavailable`() {
        val bridge = SqzBridge(binaryPath = "sqz-nonexistent-binary-xyz")
        val report = bridge.getCostReport()
        assertEquals(0, report.totalTokens)
        assertEquals(0.0, report.totalUsd, 0.001)
        assertEquals(0.0, report.cacheSavingsUsd, 0.001)
        assertEquals(0.0, report.compressionSavingsUsd, 0.001)
    }

    @Test
    fun `compress falls back to line-based when CLI unavailable and language is supported`() {
        val bridge = SqzBridge(binaryPath = "sqz-nonexistent-binary-xyz")
        val content = "fun main() {\n\n    println(\"hello\")\n\n}\n"
        // Kotlin is AST-supported, but CLI is unavailable → line-based fallback
        val result = bridge.compress(content, "Kotlin")
        assertFalse(result.usedAst, "Should fall back to line-based when CLI unavailable")
        assertEquals("unknown", result.language)
    }

    @Test
    fun `compress uses line-based fallback for unsupported language regardless of CLI`() {
        val bridge = SqzBridge(binaryPath = "sqz-nonexistent-binary-xyz")
        val content = "PROGRAM HELLO.\n   DISPLAY 'Hello'.\n   STOP RUN.\n"
        val result = bridge.compress(content, "COBOL")
        assertFalse(result.usedAst)
        assertEquals("unknown", result.language)
    }

    @Test
    fun `isAvailable returns false when binary does not exist`() {
        val bridge = SqzBridge(binaryPath = "sqz-nonexistent-binary-xyz")
        assertFalse(bridge.isAvailable())
    }
}
