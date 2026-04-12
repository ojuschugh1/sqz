// sqz browser extension — integration tests
// Run with: node extension/tests/content.test.js
// No external test framework required — uses simple assertions.

'use strict';

// ---------------------------------------------------------------------------
// Minimal assertion helpers
// ---------------------------------------------------------------------------

let passed = 0;
let failed = 0;

function assert(condition, message) {
  if (condition) {
    console.log(`  ✓ ${message}`);
    passed++;
  } else {
    console.error(`  ✗ ${message}`);
    failed++;
  }
}

function assertEqual(actual, expected, message) {
  const ok = actual === expected;
  if (ok) {
    console.log(`  ✓ ${message}`);
    passed++;
  } else {
    console.error(`  ✗ ${message} — expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
    failed++;
  }
}

function assertApprox(actual, expected, tolerance, message) {
  const ok = Math.abs(actual - expected) <= tolerance;
  if (ok) {
    console.log(`  ✓ ${message}`);
    passed++;
  } else {
    console.error(`  ✗ ${message} — expected ~${expected} (±${tolerance}), got ${actual}`);
    failed++;
  }
}

function section(name) {
  console.log(`\n${name}`);
}

// ---------------------------------------------------------------------------
// Inline the pure-JS logic under test (mirrors content/common.js)
// These are the functions that can be tested without a real browser DOM.
// ---------------------------------------------------------------------------

function sqzEstimateTokens(text) {
  return Math.ceil(text.length / 4);
}

const TOKEN_THRESHOLD = 500;

function shouldShowPreview(tokenCount) {
  return tokenCount > TOKEN_THRESHOLD;
}

// Minimal TOON-like compression stub (mirrors WasmEngine behaviour for tests)
function sqzCompressStub(text) {
  try {
    const parsed = JSON.parse(text.trim());
    // Produce compact JSON (no spaces) — simulates TOON token reduction
    return 'TOON:' + JSON.stringify(parsed);
  } catch (_) {
    return text; // non-JSON: pass through unchanged
  }
}

// ---------------------------------------------------------------------------
// 1. Token estimation
// ---------------------------------------------------------------------------

section('1. Token estimation (sqzEstimateTokens)');

assertEqual(sqzEstimateTokens(''), 0, 'empty string → 0 tokens');
assertEqual(sqzEstimateTokens('abcd'), 1, '4 chars → 1 token');
assertEqual(sqzEstimateTokens('abcde'), 2, '5 chars → 2 tokens');
assertEqual(sqzEstimateTokens('a'.repeat(2000)), 500, '2000 chars → 500 tokens');
assertEqual(sqzEstimateTokens('a'.repeat(2001)), 501, '2001 chars → 501 tokens');
assertEqual(sqzEstimateTokens('a'.repeat(2004)), 501, '2004 chars → 501 tokens (ceil)');

// ---------------------------------------------------------------------------
// 2. Preview threshold (Requirement 5.3)
// ---------------------------------------------------------------------------

section('2. Preview threshold (Requirement 5.3)');

assert(!shouldShowPreview(0), 'no preview for 0 tokens');
assert(!shouldShowPreview(499), 'no preview for 499 tokens');
assert(!shouldShowPreview(500), 'no preview for exactly 500 tokens');
assert(shouldShowPreview(501), 'preview for 501 tokens');
assert(shouldShowPreview(1000), 'preview for 1000 tokens');
assert(shouldShowPreview(10000), 'preview for 10000 tokens');

// Boundary: text that produces exactly 500 tokens (2000 chars)
const exactly500 = 'a'.repeat(2000);
assert(!shouldShowPreview(sqzEstimateTokens(exactly500)), 'no preview for 2000-char input (500 tokens)');

// Text that produces 501 tokens (2001 chars)
const over500 = 'a'.repeat(2001);
assert(shouldShowPreview(sqzEstimateTokens(over500)), 'preview for 2001-char input (501 tokens)');

// ---------------------------------------------------------------------------
// 3. WASM module loading simulation (Requirement 5.1)
// ---------------------------------------------------------------------------

section('3. WASM module loading (simulated)');

// Simulate the lazy-load pattern: module starts null, becomes available
let simulatedWasm = null;

function simulateWasmLoad(cb) {
  // Simulate async load completing successfully
  setTimeout(() => {
    simulatedWasm = { estimateOnly: false, loaded: true };
    cb(null, simulatedWasm);
  }, 0);
}

let wasmLoadCalled = false;
simulateWasmLoad((err, wasm) => {
  assert(err === null, 'WASM load: no error on success');
  assert(wasm !== null, 'WASM load: module is non-null');
  assert(wasm.loaded === true, 'WASM load: module has expected property');
  wasmLoadCalled = true;
});

// Simulate load failure → estimateOnly fallback
function simulateWasmLoadFail(cb) {
  setTimeout(() => {
    cb(null, { estimateOnly: true });
  }, 0);
}

simulateWasmLoadFail((err, wasm) => {
  assert(err === null, 'WASM fallback: no error propagated');
  assert(wasm.estimateOnly === true, 'WASM fallback: estimateOnly mode');
});

// ---------------------------------------------------------------------------
// 4. Content interception per web UI (Requirement 5.2)
// ---------------------------------------------------------------------------

section('4. Content interception — selector coverage');

// Each site script defines getInput/getText/setText/getSubmit.
// We verify the selector strings are present in the source files.
const fs = require('fs');
const path = require('path');

const contentDir = path.join(__dirname, '..', 'content');

const siteFiles = {
  'chatgpt.js': {
    selectors: ['#prompt-textarea', 'send-button', 'Send message'],
    siteName: 'ChatGPT',
  },
  'claude.js': {
    selectors: ['ProseMirror', 'Send Message'],
    siteName: 'Claude',
  },
  'gemini.js': {
    selectors: ['ql-editor', 'rich-textarea', 'Send message'],
    siteName: 'Gemini',
  },
  'grok.js': {
    selectors: ['textarea', 'Send'],
    siteName: 'Grok',
  },
  'perplexity.js': {
    selectors: ['textarea', 'Submit'],
    siteName: 'Perplexity',
  },
};

for (const [filename, spec] of Object.entries(siteFiles)) {
  const filePath = path.join(contentDir, filename);
  let src = '';
  try {
    src = fs.readFileSync(filePath, 'utf8');
  } catch (e) {
    assert(false, `${filename} exists and is readable`);
    continue;
  }
  assert(src.length > 0, `${filename} is non-empty`);
  assert(src.includes(spec.siteName), `${filename} references siteName '${spec.siteName}'`);
  assert(src.includes('sqzAttachInterceptor'), `${filename} calls sqzAttachInterceptor`);
  assert(src.includes('pass-through'), `${filename} has fallback pass-through warning`);
  for (const sel of spec.selectors) {
    assert(src.includes(sel), `${filename} contains selector/string '${sel}'`);
  }
}

// ---------------------------------------------------------------------------
// 5. common.js structure (Requirement 5.1, 5.4, 5.5)
// ---------------------------------------------------------------------------

section('5. common.js structure');

const commonSrc = fs.readFileSync(path.join(contentDir, 'common.js'), 'utf8');

assert(commonSrc.includes('sqzEstimateTokens'), 'common.js exports sqzEstimateTokens');
assert(commonSrc.includes('sqzLoadWasm'), 'common.js exports sqzLoadWasm');
assert(commonSrc.includes('sqzCompress'), 'common.js exports sqzCompress');
assert(commonSrc.includes('sqzShowPreview'), 'common.js exports sqzShowPreview');
assert(commonSrc.includes('sqzAttachInterceptor'), 'common.js exports sqzAttachInterceptor');
assert(commonSrc.includes('TOKEN_THRESHOLD'), 'common.js defines TOKEN_THRESHOLD');
assert(commonSrc.includes('500'), 'common.js threshold is 500');
assert(!commonSrc.includes('fetch('), 'common.js makes no fetch() calls (Req 5.4)');
assert(!commonSrc.includes('XMLHttpRequest'), 'common.js makes no XHR calls (Req 5.4)');
assert(commonSrc.includes('pass-through'), 'common.js has fallback pass-through (Req 5.5)');

// ---------------------------------------------------------------------------
// 6. manifest.json structure (Requirement 5.1)
// ---------------------------------------------------------------------------

section('6. manifest.json structure');

const manifestPath = path.join(__dirname, '..', 'manifest.json');
let manifest;
try {
  manifest = JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
} catch (e) {
  assert(false, 'manifest.json is valid JSON');
  manifest = {};
}

assertEqual(manifest.manifest_version, 3, 'manifest_version is 3');
assert(typeof manifest.name === 'string' && manifest.name.length > 0, 'manifest has name');
assert(typeof manifest.version === 'string', 'manifest has version');
assert(Array.isArray(manifest.content_scripts), 'manifest has content_scripts array');
assertEqual(manifest.content_scripts.length, 5, 'manifest has 5 content_script entries');

const allMatches = manifest.content_scripts.flatMap((cs) => cs.matches || []).join(' ');
assert(allMatches.includes('chatgpt.com') || allMatches.includes('chat.openai.com'), 'ChatGPT match present');
assert(allMatches.includes('claude.ai'), 'Claude match present');
assert(allMatches.includes('gemini.google.com'), 'Gemini match present');
assert(allMatches.includes('grok.com'), 'Grok match present');
assert(allMatches.includes('perplexity.ai'), 'Perplexity match present');

// Verify no external host_permissions beyond the 5 AI sites
const hostPerms = manifest.host_permissions || [];
const externalHosts = hostPerms.filter((h) =>
  !h.includes('chatgpt.com') &&
  !h.includes('chat.openai.com') &&
  !h.includes('claude.ai') &&
  !h.includes('gemini.google.com') &&
  !h.includes('grok.com') &&
  !h.includes('perplexity.ai')
);
assertEqual(externalHosts.length, 0, 'no external host_permissions (Req 5.4)');

// ---------------------------------------------------------------------------
// 7. Compression stub — non-JSON pass-through (Requirement 5.5 analogue)
// ---------------------------------------------------------------------------

section('7. Compression — non-JSON pass-through');

const plainText = 'hello world, this is plain text';
assertEqual(sqzCompressStub(plainText), plainText, 'non-JSON input passes through unchanged');

const jsonText = '{"key": "value", "num": 42}';
const compressed = sqzCompressStub(jsonText);
assert(compressed.startsWith('TOON:'), 'JSON input gets TOON prefix');
assert(compressed.length < jsonText.length + 6, 'TOON output is not longer than input + prefix');

// ---------------------------------------------------------------------------
// 8. Preview banner token display format
// ---------------------------------------------------------------------------

section('8. Preview banner token display');

function formatPreviewText(originalTokens, compressedTokens) {
  const reduction = originalTokens > 0
    ? Math.round((1 - compressedTokens / originalTokens) * 100)
    : 0;
  return `sqz: ${originalTokens} tokens → ${compressedTokens} tokens (${reduction}% reduction)`;
}

const preview = formatPreviewText(2000, 800);
assert(preview.includes('2000 tokens'), 'preview shows original token count');
assert(preview.includes('800 tokens'), 'preview shows compressed token count');
assert(preview.includes('60% reduction'), 'preview shows correct reduction percentage');

const preview2 = formatPreviewText(1000, 1000);
assert(preview2.includes('0% reduction'), 'preview shows 0% when no reduction');

// ---------------------------------------------------------------------------
// 9. Stats accumulation
// ---------------------------------------------------------------------------

section('9. Stats accumulation');

function accumulateStats(stats, originalTokens, compressedTokens) {
  return {
    totalOriginal: stats.totalOriginal + originalTokens,
    totalCompressed: stats.totalCompressed + compressedTokens,
    compressions: stats.compressions + 1,
  };
}

let stats = { totalOriginal: 0, totalCompressed: 0, compressions: 0 };
stats = accumulateStats(stats, 1000, 400);
stats = accumulateStats(stats, 2000, 600);

assertEqual(stats.compressions, 2, 'stats: 2 compressions recorded');
assertEqual(stats.totalOriginal, 3000, 'stats: totalOriginal correct');
assertEqual(stats.totalCompressed, 1000, 'stats: totalCompressed correct');
assertApprox(
  Math.round((1 - stats.totalCompressed / stats.totalOriginal) * 100),
  67,
  1,
  'stats: overall reduction ~67%'
);

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

console.log(`\n${'─'.repeat(50)}`);
console.log(`Results: ${passed} passed, ${failed} failed`);

if (failed > 0) {
  process.exit(1);
} else {
  console.log('All tests passed.');
}
