/**
 * SqzBridge.kt — CLI bridge to the sqz binary via ProcessBuilder.
 *
 * Mirrors the VS Code sqzBridge.ts approach: calls `sqz` as a subprocess
 * rather than using JNI. This satisfies all requirements since the CLI
 * wraps sqz_engine internally.
 *
 * Requirements: 6.1, 6.2, 6.3, 6.5
 */

package com.sqz

import java.io.File
import java.nio.file.Files

data class CompressResult(
    val compressed: String,
    val tokensOriginal: Int,
    val tokensCompressed: Int,
    val compressionRatio: Double,
    val language: String,
    val usedAst: Boolean,
)

data class BudgetStatus(
    val consumed: Int,
    val windowSize: Int,
    val percentUsed: Double,
    val available: Int,
)

data class CostReport(
    val totalTokens: Int,
    val totalUsd: Double,
    val cacheSavingsUsd: Double,
    val compressionSavingsUsd: Double,
)

/** Languages supported by the sqz AST parser (tree-sitter + regex extractors). */
val AST_SUPPORTED_LANGUAGES: Set<String> = setOf(
    "rust", "python", "javascript", "typescript", "go", "java",
    "c", "cpp", "ruby", "bash", "json", "html", "css", "csharp",
    "kotlin", "swift", "toml", "yaml",
)

/** Map JetBrains language IDs to sqz language identifiers. */
fun jetbrainsLanguageToSqz(languageId: String): String? {
    val map = mapOf(
        "Rust" to "rust",
        "Python" to "python",
        "JavaScript" to "javascript",
        "TypeScript" to "typescript",
        "Go" to "go",
        "JAVA" to "java",
        "C" to "c",
        "C++" to "cpp",
        "Ruby" to "ruby",
        "Shell Script" to "bash",
        "JSON" to "json",
        "HTML" to "html",
        "CSS" to "css",
        "C#" to "csharp",
        "Kotlin" to "kotlin",
        "Swift" to "swift",
        "TOML" to "toml",
        "YAML" to "yaml",
    )
    return map[languageId]
}

/** Returns true if the JetBrains language ID is supported by the AST parser. */
fun isAstSupported(languageId: String): Boolean {
    val sqzLang = jetbrainsLanguageToSqz(languageId) ?: return false
    return sqzLang in AST_SUPPORTED_LANGUAGES
}

/**
 * Line-based compression fallback for unsupported languages.
 * Removes blank lines and trims trailing whitespace.
 * Requirement 6.5: fallback to line-based compression.
 */
fun lineFallbackCompress(content: String): CompressResult {
    val compressed = content.lines()
        .map { it.trimEnd() }
        .filter { it.isNotEmpty() }
        .joinToString("\n")

    val tokensOriginal = (content.length + 3) / 4
    val tokensCompressed = (compressed.length + 3) / 4

    return CompressResult(
        compressed = compressed,
        tokensOriginal = tokensOriginal,
        tokensCompressed = tokensCompressed,
        compressionRatio = if (tokensOriginal > 0) tokensCompressed.toDouble() / tokensOriginal else 1.0,
        language = "unknown",
        usedAst = false,
    )
}

class SqzBridge(private val binaryPath: String = "sqz") {

    private fun exec(args: List<String>, input: String? = null): String {
        val cmd = listOf(binaryPath) + args
        val pb = ProcessBuilder(cmd)
            .redirectErrorStream(true)

        val process = pb.start()

        if (input != null) {
            process.outputStream.bufferedWriter().use { it.write(input) }
        }

        val output = process.inputStream.bufferedReader().readText()
        val exitCode = process.waitFor()

        if (exitCode != 0) {
            throw RuntimeException("sqz CLI error (exit $exitCode): $output")
        }
        return output.trim()
    }

    /** Check whether the sqz binary is available. */
    fun isAvailable(): Boolean = try {
        exec(listOf("--version"))
        true
    } catch (_: Exception) {
        false
    }

    /**
     * Compress content using the sqz CLI.
     * Falls back to line-based compression if the CLI is unavailable or the
     * language is not supported by the AST parser.
     *
     * Requirements: 6.2, 6.3, 6.5
     */
    fun compress(content: String, languageId: String, filePath: String? = null): CompressResult {
        val sqzLang = jetbrainsLanguageToSqz(languageId)
        val astSupported = sqzLang != null && sqzLang in AST_SUPPORTED_LANGUAGES

        // If language is not AST-supported, use line-based fallback (Req 6.5)
        if (!astSupported) {
            return lineFallbackCompress(content)
        }

        // Try CLI-based compression
        if (!isAvailable()) {
            return lineFallbackCompress(content)
        }

        return try {
            val tmpFile = Files.createTempFile("sqz_jb_", ".tmp").toFile()
            try {
                tmpFile.writeText(content)
                val args = mutableListOf("compress", tmpFile.absolutePath)
                if (sqzLang != null) {
                    args += listOf("--language", sqzLang)
                }
                val output = exec(args)
                val tokensOriginal = (content.length + 3) / 4
                val tokensCompressed = (output.length + 3) / 4
                CompressResult(
                    compressed = output,
                    tokensOriginal = tokensOriginal,
                    tokensCompressed = tokensCompressed,
                    compressionRatio = if (tokensOriginal > 0) tokensCompressed.toDouble() / tokensOriginal else 1.0,
                    language = sqzLang ?: languageId,
                    usedAst = true,
                )
            } finally {
                tmpFile.delete()
            }
        } catch (_: Exception) {
            lineFallbackCompress(content)
        }
    }

    /**
     * Get the current budget status for a session.
     * Requirement 6.4: display token budget usage in status bar.
     */
    fun getBudgetStatus(sessionId: String = "default"): BudgetStatus {
        if (!isAvailable()) {
            return BudgetStatus(consumed = 0, windowSize = 200_000, percentUsed = 0.0, available = 200_000)
        }
        return try {
            val output = exec(listOf("status", "--session", sessionId, "--json"))
            // Minimal JSON parse — avoid pulling in a full JSON library
            val consumed = parseJsonInt(output, "consumed") ?: 0
            val windowSize = parseJsonInt(output, "windowSize") ?: 200_000
            BudgetStatus(
                consumed = consumed,
                windowSize = windowSize,
                percentUsed = if (windowSize > 0) consumed.toDouble() / windowSize * 100 else 0.0,
                available = windowSize - consumed,
            )
        } catch (_: Exception) {
            BudgetStatus(consumed = 0, windowSize = 200_000, percentUsed = 0.0, available = 200_000)
        }
    }

    /**
     * Get the cost report for a session.
     * Requirement 6.4 / 22.x
     */
    fun getCostReport(sessionId: String = "default"): CostReport {
        if (!isAvailable()) {
            return CostReport(0, 0.0, 0.0, 0.0)
        }
        return try {
            val output = exec(listOf("cost", "--session", sessionId, "--json"))
            CostReport(
                totalTokens = parseJsonInt(output, "totalTokens") ?: 0,
                totalUsd = parseJsonDouble(output, "totalUsd") ?: 0.0,
                cacheSavingsUsd = parseJsonDouble(output, "cacheSavingsUsd") ?: 0.0,
                compressionSavingsUsd = parseJsonDouble(output, "compressionSavingsUsd") ?: 0.0,
            )
        } catch (_: Exception) {
            CostReport(0, 0.0, 0.0, 0.0)
        }
    }

    /** Export the current session to a .ctx string. Requirement 7.1 */
    fun exportCtx(sessionId: String = "default"): String =
        exec(listOf("export", "--session", sessionId))

    /** Import a .ctx string. Requirement 7.3 */
    fun importCtx(ctxContent: String) {
        val tmpFile = Files.createTempFile("sqz_import_", ".ctx").toFile()
        try {
            tmpFile.writeText(ctxContent)
            exec(listOf("import", tmpFile.absolutePath))
        } finally {
            tmpFile.delete()
        }
    }

    // ── Minimal JSON field extractors (no external dependency) ───────────────

    private fun parseJsonInt(json: String, key: String): Int? =
        Regex(""""$key"\s*:\s*(\d+)""").find(json)?.groupValues?.get(1)?.toIntOrNull()

    private fun parseJsonDouble(json: String, key: String): Double? =
        Regex(""""$key"\s*:\s*([\d.]+)""").find(json)?.groupValues?.get(1)?.toDoubleOrNull()
}
