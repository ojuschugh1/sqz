// sqz content script — shared utilities
// All processing is local; zero external network requests.
//
// Compression flow:
//   1. Content script sends text to background via message passing
//   2. Background script compresses via WASM (no CSP restrictions)
//   3. If background WASM fails, content script falls back to pure JS

'use strict';

// Cross-browser API shim (Firefox uses browser.*, Chrome uses chrome.*)
const sqzApi = typeof browser !== 'undefined' ? browser : chrome;

console.log('[sqz] Common content script loaded');

// ---------------------------------------------------------------------------
// Token estimation (mirrors WasmEngine::estimate_tokens: chars / 4, ceil)
// ---------------------------------------------------------------------------

function sqzEstimateTokens(text) {
  return Math.ceil(text.length / 4);
}

// ---------------------------------------------------------------------------
// Background WASM compression via message passing
// ---------------------------------------------------------------------------

function sqzCompressViaBackground(text) {
  return new Promise((resolve) => {
    try {
      const result = sqzApi.runtime.sendMessage({ type: 'COMPRESS', text: text });
      // Firefox returns a Promise; Chrome uses callback
      if (result && typeof result.then === 'function') {
        result.then((response) => {
          resolve(response && response.compressed ? response.compressed : null);
        }).catch(() => resolve(null));
      } else {
        // Chrome callback style — handled by the third arg
        resolve(null);
      }
    } catch (e) {
      resolve(null);
    }
  });
}

// Chrome callback fallback — re-register with callback style
if (typeof browser === 'undefined') {
  // Chrome: override with callback-based version
  sqzCompressViaBackground = function(text) {
    return new Promise((resolve) => {
      try {
        chrome.runtime.sendMessage({ type: 'COMPRESS', text: text }, (response) => {
          if (chrome.runtime.lastError || !response) {
            resolve(null);
            return;
          }
          resolve(response.compressed || null);
        });
      } catch (e) {
        resolve(null);
      }
    });
  };
}

// ---------------------------------------------------------------------------


// ---------------------------------------------------------------------------
// sqz Squeeze Engine — adaptive multi-pass compression
//
// A content-aware compression pipeline that routes input through
// specialized passes based on detected content type. Each pass is
// independent and stateless — the output of one feeds the next.
//
// Supported content types: json, code, log, markdown, prose
// ---------------------------------------------------------------------------

// --- Scoring helpers ---

var SQZ_COMMON_WORDS = 'the a an is are was were be been being have has had do does did will would could should may might shall can to of in for on with at by from as into through during before after above below between under again further then once and but or nor not so yet both either neither each every all any few more most other some such no only own same than too very just also now here there when where why how this that these those it its he she they them their his her we our you your i me my which who whom what';
var _sqzCommonSet = null;
function sqzCommonWords() {
  if (!_sqzCommonSet) _sqzCommonSet = new Set(SQZ_COMMON_WORDS.split(' '));
  return _sqzCommonSet;
}

// Information density: ratio of content words to total words
function sqzDensity(sentence) {
  var words = sentence.toLowerCase().replace(/[^a-z' ]/g, ' ').split(/\s+/).filter(function(w) { return w.length > 1; });
  if (words.length === 0) return 0;
  var common = sqzCommonWords(), content = 0;
  for (var i = 0; i < words.length; i++) { if (!common.has(words[i])) content++; }
  return content / words.length;
}

// Fast 32-bit fingerprint for near-duplicate detection
function sqzFingerprint(text) {
  var words = text.toLowerCase().split(/\s+/);
  var v = new Int32Array(32);
  for (var i = 0; i < words.length; i++) {
    var h = 0;
    for (var j = 0; j < words[i].length; j++) {
      h = ((h << 5) - h + words[i].charCodeAt(j)) | 0;
    }
    for (var b = 0; b < 32; b++) {
      v[b] += (h & (1 << b)) ? 1 : -1;
    }
  }
  var hash = 0;
  for (var b2 = 0; b2 < 32; b2++) {
    if (v[b2] > 0) hash |= (1 << b2);
  }
  return hash;
}

function sqzBitDiff(a, b) {
  var x = a ^ b, d = 0;
  while (x) { d += x & 1; x >>>= 1; }
  return d;
}

// --- Content classifier ---

function sqzClassify(text) {
  var t = text.trim();
  // JSON
  if ((t[0] === '{' || t[0] === '[') && (t[t.length-1] === '}' || t[t.length-1] === ']')) {
    try { JSON.parse(t); return 'json'; } catch(e) {}
  }
  // Code
  var codeHits = 0;
  if (/^(import |from |require\(|#include|using |package )/m.test(t)) codeHits += 3;
  if (/^(def |fn |func |function |class |interface |struct |enum |const |let |var )/m.test(t)) codeHits += 3;
  if (/[{};]\s*$/m.test(t)) codeHits += 2;
  if (/^\s*(if|for|while|return|switch|case)\s*[\s(]/m.test(t)) codeHits += 2;
  if (/\/\/|\/\*|\*\/|#\s|--\s/.test(t)) codeHits += 1;
  if (codeHits >= 4) return 'code';
  // Logs
  var lines = t.split('\n');
  var logLines = lines.filter(function(l) {
    return /^\d{4}[-/]\d{2}[-/]\d{2}|^\[?(INFO|WARN|ERROR|DEBUG|TRACE)\]?|^\d{2}:\d{2}:\d{2}/.test(l.trim());
  });
  if (logLines.length > lines.length * 0.3) return 'log';
  // Markdown
  if (/^#{1,6}\s/m.test(t) && /\n#{1,6}\s/m.test(t)) return 'markdown';
  return 'prose';
}

// --- Specialized compressors ---

function sqzSqueezeJSON(text) {
  try {
    var parsed = JSON.parse(text.trim());
    if (Array.isArray(parsed) && parsed.length > 5) {
      var first = parsed[0], mid = parsed[Math.floor(parsed.length / 2)];
      var shape = {};
      if (first && typeof first === 'object') {
        Object.keys(first).forEach(function(k) { shape[k] = typeof first[k]; });
      }
      var sampled = JSON.stringify({
        _sqz_sampled: true,
        count: parsed.length,
        shape: shape,
        examples: [first, mid]
      });
      if (sampled.length < text.length) return sampled;
    }
    var compact = JSON.stringify(parsed);
    return compact.length < text.length ? compact : text;
  } catch(e) { return text; }
}

function sqzSqueezeLogs(text) {
  var lines = text.split('\n'), out = [], prevKey = '', count = 0;
  for (var i = 0; i < lines.length; i++) {
    var key = lines[i].replace(/^\d{4}[-/]\d{2}[-/]\d{2}[T ]\d{2}:\d{2}:\d{2}[.\d]*\s*/, '')
                       .replace(/^\[?\d{2}:\d{2}:\d{2}[.\d]*\]?\s*/, '').trim();
    if (key === prevKey && key.length > 0) {
      count++;
    } else {
      if (count > 0) out[out.length - 1] += ' [x' + (count + 1) + ']';
      out.push(lines[i]);
      prevKey = key;
      count = 0;
    }
  }
  if (count > 0) out[out.length - 1] += ' [x' + (count + 1) + ']';
  return out.join('\n');
}

function sqzStripBlobs(text) {
  return text.replace(/(?:data:[a-z]+\/[a-z+.-]+;base64,)?[A-Za-z0-9+/]{100,}={0,2}/g, function(m) {
    return '[blob:' + m.length + 'ch]';
  });
}

function sqzShortenPaths(text) {
  var paths = text.match(/(?:\/[\w.-]+){3,}/g) || [];
  if (paths.length < 2) return text;
  var sorted = paths.slice().sort();
  var first = sorted[0], last = sorted[sorted.length - 1], prefix = '';
  for (var i = 0; i < first.length && i < last.length; i++) {
    if (first[i] === last[i]) prefix += first[i]; else break;
  }
  var cut = prefix.lastIndexOf('/');
  if (cut > 10) {
    prefix = prefix.substring(0, cut + 1);
    var r = text;
    while (r.indexOf(prefix) !== -1) r = r.replace(prefix, '~/');
    return r;
  }
  return text;
}

function sqzStripMarkdown(text) {
  var r = text;
  r = r.replace(/\*\*([^*]+)\*\*/g, '$1');
  r = r.replace(/\*([^*]+)\*/g, '$1');
  r = r.replace(/__([^_]+)__/g, '$1');
  r = r.replace(/_([^_]+)_/g, '$1');
  r = r.replace(/^#{1,6}\s+(.+)$/gm, '$1:');
  r = r.replace(/^[-*_]{3,}\s*$/gm, '');
  r = r.replace(/^(\s*)[-*+]\s+/gm, '$1- ');
  return r;
}

// --- Main squeeze function ---

function sqzCompressJS(input) {
  if (!input || input.length < 100) return input;

  var kind = sqzClassify(input);
  var r = input;

  // Pass 1: Whitespace normalization
  r = r.replace(/\r\n/g, '\n').replace(/[ \t]+/g, ' ').replace(/\n{3,}/g, '\n\n');
  r = r.split('\n').map(function(l) { return l.trim(); }).join('\n');

  // Pass 2: Blob detection — replace base64 and binary blobs with placeholders
  r = sqzStripBlobs(r);

  // Pass 3: Route to specialized compressor
  if (kind === 'json') return sqzSqueezeJSON(r);

  if (kind === 'log') {
    r = sqzSqueezeLogs(r);
    r = sqzShortenPaths(r);
    r = r.replace(/  +/g, ' ').replace(/\n{3,}/g, '\n\n').trimEnd();
    return (r.length < input.length) ? r : input;
  }

  if (kind === 'markdown') r = sqzStripMarkdown(r);

  // Pass 4: Path shortening
  r = sqzShortenPaths(r);

  // Pass 5: Fingerprint-based deduplication
  var sents = r.split(/(?<=[.!?])\s+/);
  if (sents.length > 2) {
    var kept = [], prints = [];
    for (var i = 0; i < sents.length; i++) {
      if (sents[i].length < 15) { kept.push(sents[i]); continue; }
      var fp = sqzFingerprint(sents[i]);
      var dup = false;
      for (var j = 0; j < prints.length; j++) {
        if (sqzBitDiff(fp, prints[j]) <= 3) { dup = true; break; }
      }
      if (!dup) { kept.push(sents[i]); prints.push(fp); }
    }
    if (kept.length < sents.length) r = kept.join(' ');
  }

  // Pass 6: Density-ranked sentence pruning (prose/markdown only)
  if (kind !== 'code') {
    sents = r.split(/(?<=[.!?])\s+/);
    if (sents.length > 8) {
      var scored = [];
      for (var si = 0; si < sents.length; si++) {
        scored.push({ idx: si, text: sents[si], score: sqzDensity(sents[si]), words: sents[si].split(/\s+/).length });
      }
      var ranked = scored.slice().sort(function(a, b) { return a.score - b.score; });
      var dropCount = Math.floor(sents.length * 0.15);
      var dropSet = new Set(), dropped = 0;
      for (var di = 0; di < ranked.length && dropped < dropCount; di++) {
        var s = ranked[di];
        if (s.idx === 0 || s.idx === sents.length - 1) continue;
        if (s.words <= 5) continue;
        if (/\d/.test(s.text)) continue;
        if (/\[[\d,\s]+\]|\(\d{4}\)|et al|Fig\.|Table|Equation|Section|Chapter/i.test(s.text)) continue;
        if (/[=<>{}()\[\]].*[=<>{}()\[\]]/.test(s.text)) continue;
        dropSet.add(s.idx);
        dropped++;
      }
      if (dropSet.size > 0) {
        var filtered = [];
        for (var fi = 0; fi < sents.length; fi++) {
          if (!dropSet.has(fi)) filtered.push(sents[fi]);
        }
        r = filtered.join(' ');
      }
    }
  }

  // Pass 7: Filler removal
  if (kind !== 'code') {
    r = r.replace(/\b(?:I think that|I believe that|I feel that|It seems that|It appears that)\s*/gi, '');
    r = r.replace(/\b(?:As you (?:may |might )?know|As we all know|As mentioned (?:earlier|above|before|previously))\s*,?\s*/gi, '');
    r = r.replace(/\b(?:It is (?:important|worth|interesting) to note that|It should be noted that)\s*/gi, '');
    r = r.replace(/\b(?:Needless to say|It goes without saying that|Obviously|Clearly|Of course)\s*,?\s*/gi, '');
    r = r.replace(/\b(?:In my opinion|From my perspective|In my view|Personally)\s*,?\s*/gi, '');
    r = r.replace(/\b(?:basically|essentially|fundamentally|literally|actually|really|quite|rather|somewhat|fairly)\s+/gi, '');
    r = r.replace(/\b(?:In conclusion|To summarize|To sum up|In summary|All in all|Overall)\s*,?\s*/gi, '');
    r = r.replace(/\b(?:As a matter of fact|The thing is|The point is|What I mean is)\s*,?\s*/gi, '');
  }

  // Pass 8: Phrase substitution
  if (kind !== 'code') {
    var phrases = [
      ['in spite of the fact that','although'],['despite the fact that','although'],
      ['due to the fact that','because'],['for the purpose of','to'],
      ['in the event that','if'],['with the exception of','except'],
      ['in the absence of','without'],['at this point in time','now'],
      ['in close proximity to','near'],['the vast majority of','most'],
      ['a large number of','many'],['has the ability to','can'],
      ['in accordance with','per'],['take into consideration','consider'],
      ['with respect to','about'],['with regard to','about'],
      ['in conjunction with','with'],['on the other hand','however'],
      ['in addition to','besides'],['a number of','several'],
      ['in order to','to'],['as a result of','from'],['in terms of','regarding'],
      ['pertaining to','about'],['subsequent to','after'],['prior to','before'],
      ['whether or not','whether'],['as well as','and'],
      ['in the field of','in'],['in the area of','in'],
      ['is able to','can'],['are able to','can'],['was able to','could'],
      ['such as','like'],['on a daily basis','daily'],['for the most part','mostly'],
      ['in the near future','soon'],['a wide range of','many'],
      ['a variety of','various'],['the majority of','most'],
      ['a significant number of','many'],['on the basis of','based on'],
      ['in the context of','in'],['make a decision','decide'],
      ['have an impact on','affect'],['plays a role in','affects'],
    ];
    for (var pi = 0; pi < phrases.length; pi++) {
      r = r.split(phrases[pi][0]).join(phrases[pi][1]);
      var cap = phrases[pi][0].charAt(0).toUpperCase() + phrases[pi][0].slice(1);
      var capR = phrases[pi][1].charAt(0).toUpperCase() + phrases[pi][1].slice(1);
      r = r.split(cap).join(capR);
    }
  }

  // Pass 9: Article stripping
  if (kind !== 'code') {
    r = r.replace(/\b(?:The|the|A|a|An|an)\s+(?=[a-z])/g, '');
  }

  // Pass 10: Redundant "that" removal
  if (kind !== 'code') {
    r = r.replace(/\b(believe|think|know|said|found|show|suggest|indicate|ensure|note|argue|claim|assume|expect|hope|feel|realize|understand|demonstrate|reveal|confirm|conclude|determine|observe|report|state|mention|explain|propose|recommend|consider|notice|remember|prove|verify|imply|mean|see)\s+that\b/gi, '$1');
  }

  // Pass 11: Verb simplification
  if (kind !== 'code') {
    r = r.replace(/\bis being\b/gi,'is').replace(/\bare being\b/gi,'are');
    r = r.replace(/\bhas been\b/gi,'was').replace(/\bhave been\b/gi,'were');
    r = r.replace(/\bwill be able to\b/gi,'can').replace(/\bwould be able to\b/gi,'could');
  }

  // Pass 12: Transition stripping
  if (kind !== 'code') {
    r = r.replace(/^(?:However|Therefore|Furthermore|Moreover|Additionally|Consequently|Nevertheless|Nonetheless|Meanwhile|Subsequently|Accordingly|Similarly|Likewise|Alternatively|Conversely|Specifically|Importantly|Interestingly|Notably|Significantly|Ultimately)\s*,?\s+/gm, '');
  }

  // Pass 13: Contractions
  if (kind !== 'code') {
    r = r.replace(/\bdo not\b/g,"don't").replace(/\bDo not\b/g,"Don't");
    r = r.replace(/\bcannot\b/g,"can't").replace(/\bwill not\b/g,"won't");
    r = r.replace(/\bshould not\b/g,"shouldn't").replace(/\bwould not\b/g,"wouldn't");
    r = r.replace(/\bcould not\b/g,"couldn't").replace(/\bdoes not\b/g,"doesn't");
    r = r.replace(/\bdid not\b/g,"didn't").replace(/\bis not\b/g,"isn't");
    r = r.replace(/\bare not\b/g,"aren't").replace(/\bwas not\b/g,"wasn't");
    r = r.replace(/\bwere not\b/g,"weren't").replace(/\bhas not\b/g,"hasn't");
    r = r.replace(/\bhave not\b/g,"haven't").replace(/\bit is\b/g,"it's");
    r = r.replace(/\bthat is\b/g,"that's").replace(/\bthere is\b/g,"there's");
    r = r.replace(/\bthey are\b/g,"they're").replace(/\bwe are\b/g,"we're");
    r = r.replace(/\byou are\b/g,"you're").replace(/\bI am\b/g,"I'm");
    r = r.replace(/\bI have\b/g,"I've").replace(/\bI would\b/g,"I'd").replace(/\bI will\b/g,"I'll");
  }

  // Pass 14: Adverb stripping
  if (kind !== 'code') {
    r = r.replace(/\b(?:very|extremely|incredibly|remarkably|particularly|especially|significantly|substantially|considerably|tremendously|enormously|immensely|vastly|greatly|highly|deeply|strongly|firmly|closely|widely|largely|mostly|nearly|almost|approximately|roughly|generally|typically|usually|often|frequently|commonly|increasingly|rapidly|gradually|slowly|carefully|properly|effectively|efficiently|successfully|completely|entirely|totally|fully|absolutely|perfectly|exactly|precisely|certainly|definitely|undoubtedly|obviously|apparently|seemingly|relatively|comparatively)\s+/gi, '');
  }

  // Pass 15: Code cleanup (code only)
  if (kind === 'code') {
    r = r.replace(/^\s*\/\/(?!.*https?:).*$/gm, '');
    r = r.replace(/^\s*#(?!!)(?!.*https?:).*$/gm, '');
    r = r.replace(/\n{2,}/g, '\n');
    r = r.replace(/[ \t]+$/gm, '');
  }

  // Pass 16: Final polish
  r = r.replace(/  +/g, ' ').replace(/\n /g, '\n').replace(/\n{3,}/g, '\n\n');
  r = r.replace(/ ([.,;:!?])/g, '$1');
  r = r.trimEnd();

  return (r.length < input.length) ? r : input;
}


// ---------------------------------------------------------------------------
// Unified compression: try WASM via background, fall back to JS
// ---------------------------------------------------------------------------

async function sqzCompress(text) {
  if (!text || text.length === 0) return text;

  // Try WASM compression via background script first
  const wasmResult = await sqzCompressViaBackground(text);
  if (wasmResult && wasmResult.length < text.length) {
    return wasmResult;
  }

  // Fall back to pure JS compression
  return sqzCompressJS(text);
}

// ---------------------------------------------------------------------------
// Preview banner
// ---------------------------------------------------------------------------

const PREVIEW_BANNER_ID = 'sqz-preview-banner';
const TOKEN_THRESHOLD = 500;

function sqzShowPreview(originalTokens, compressedTokens, onAccept, onDismiss) {
  sqzRemovePreview();

  const reduction = originalTokens > 0
    ? Math.round((1 - compressedTokens / originalTokens) * 100)
    : 0;

  const banner = document.createElement('div');
  banner.id = PREVIEW_BANNER_ID;
  banner.setAttribute('role', 'status');
  banner.setAttribute('aria-live', 'polite');
  banner.style.cssText = [
    'position:fixed',
    'bottom:80px',
    'left:50%',
    'transform:translateX(-50%)',
    'z-index:2147483647',
    'background:#1a1a2e',
    'color:#e0e0e0',
    'border:1px solid #4a4a8a',
    'border-radius:8px',
    'padding:10px 16px',
    'font-family:monospace',
    'font-size:13px',
    'display:flex',
    'align-items:center',
    'gap:12px',
    'box-shadow:0 4px 16px rgba(0,0,0,0.4)',
    'max-width:480px',
  ].join(';');

  const label = document.createElement('span');
  label.textContent = 'sqz: ' + originalTokens + ' → ' + compressedTokens + ' tokens (' + reduction + '% reduction)';

  const acceptBtn = document.createElement('button');
  acceptBtn.textContent = 'Compress';
  acceptBtn.style.cssText = 'background:#4a4a8a;color:#fff;border:none;border-radius:4px;padding:4px 10px;cursor:pointer;font-size:12px;';
  acceptBtn.addEventListener('click', () => { sqzRemovePreview(); onAccept(); });

  const dismissBtn = document.createElement('button');
  dismissBtn.textContent = 'Skip';
  dismissBtn.style.cssText = 'background:transparent;color:#aaa;border:1px solid #555;border-radius:4px;padding:4px 10px;cursor:pointer;font-size:12px;';
  dismissBtn.addEventListener('click', () => { sqzRemovePreview(); onDismiss(); });

  banner.appendChild(label);
  banner.appendChild(acceptBtn);
  banner.appendChild(dismissBtn);
  document.body.appendChild(banner);
}

function sqzRemovePreview() {
  const existing = document.getElementById(PREVIEW_BANNER_ID);
  if (existing) existing.remove();
}

// ---------------------------------------------------------------------------
// Stats tracking
// ---------------------------------------------------------------------------

function sqzRecordStats(originalTokens, compressedTokens) {
  sqzApi.storage.local.get(['sqzStats']).then((result) => {
    const stats = result.sqzStats || { totalOriginal: 0, totalCompressed: 0, compressions: 0 };
    stats.totalOriginal += originalTokens;
    stats.totalCompressed += compressedTokens;
    stats.compressions += 1;
    sqzApi.storage.local.set({ sqzStats: stats });
  }).catch(() => {});
}

// ---------------------------------------------------------------------------
// Core interception helper
// ---------------------------------------------------------------------------

function sqzAttachInterceptor(opts) {
  const { getInput, getText, setText, getSubmit, siteName } = opts;

  let pendingCompressed = null;
  let pendingOriginal = null;
  let lastCompressedText = null; // track what we last wrote to prevent re-trigger

  // Handle text grabbed directly from clipboard (before site converts to attachment)
  async function handlePastedText(el, clipText) {
    const originalTokens = sqzEstimateTokens(clipText);
    const compressed = await sqzCompress(clipText);
    const compressedTokens = sqzEstimateTokens(compressed);

    if (compressed === clipText) return; // no reduction

    pendingCompressed = compressed;
    pendingOriginal = clipText;

    sqzShowPreview(originalTokens, compressedTokens,
      () => {
        try {
          lastCompressedText = compressed;
          setText(el, compressed);
          sqzRecordStats(originalTokens, compressedTokens);
        } catch (e) {
          console.warn('[sqz][' + siteName + '] setText failed:', e);
        }
        pendingCompressed = null;
        pendingOriginal = null;
      },
      () => {
        pendingCompressed = null;
        pendingOriginal = null;
      }
    );
  }

  async function handleInput(el) {
    const text = getText(el);

    // Skip if this is the text we just wrote via compression
    if (lastCompressedText && text === lastCompressedText) return;
    // Also skip if text is a substring match (editor may add/strip trailing whitespace)
    if (lastCompressedText && text.trim() === lastCompressedText.trim()) return;
    // Clear the marker if user has typed new content
    lastCompressedText = null;

    const tokens = sqzEstimateTokens(text);
    if (tokens <= TOKEN_THRESHOLD) {
      pendingCompressed = null;
      pendingOriginal = null;
      sqzRemovePreview();
      return;
    }

    console.log('[sqz][' + siteName + '] Input exceeds threshold: ' + tokens + ' tokens (' + text.length + ' chars)');

    const compressed = await sqzCompress(text);
    const compressedTokens = sqzEstimateTokens(compressed);

    if (compressed === text) {
      // Already fully compressed — nothing more to do
      sqzRemovePreview();
      return;
    }

    pendingCompressed = compressed;
    pendingOriginal = text;

    sqzShowPreview(tokens, compressedTokens,
      () => {
        try {
          lastCompressedText = compressed;
          setText(el, compressed);
          sqzRecordStats(tokens, compressedTokens);
        } catch (e) {
          console.warn('[sqz][' + siteName + '] setText failed:', e);
        }
        pendingCompressed = null;
        pendingOriginal = null;
      },
      () => {
        pendingCompressed = null;
        pendingOriginal = null;
      }
    );
  }

  function handleSubmit() {
    if (!pendingCompressed) return;
    const el = getInput();
    if (!el) return;
    try {
      const originalTokens = sqzEstimateTokens(pendingOriginal || '');
      const compressedTokens = sqzEstimateTokens(pendingCompressed);
      lastCompressedText = pendingCompressed;
      setText(el, pendingCompressed);
      sqzRecordStats(originalTokens, compressedTokens);
    } catch (e) {
      console.warn('[sqz][' + siteName + '] submit setText failed:', e);
    }
    pendingCompressed = null;
    pendingOriginal = null;
    sqzRemovePreview();
  }

  // Observe DOM for the input element (it may not exist at script load time)
  function tryAttach() {
    const el = getInput();
    if (!el) {
      console.log('[sqz][' + siteName + '] Input element not found yet, waiting...');
      return false;
    }

    if (el._sqzAttached) return true;
    el._sqzAttached = true;
    console.log('[sqz][' + siteName + '] Attached to input element:', el.tagName, el.className);

    // Standard input event
    el.addEventListener('input', () => {
      handleInput(el).catch((e) => {
        console.warn('[sqz][' + siteName + '] input handler error:', e);
      });
    });

    // Paste event — intercept in CAPTURING phase to read clipboard data
    // before the site (e.g. Claude) converts long pastes into file attachments.
    el.addEventListener('paste', (evt) => {
      try {
        const cd = evt.clipboardData || window.clipboardData;
        const clipText = cd ? cd.getData('text') : '';
        if (clipText) {
          const tokens = sqzEstimateTokens(clipText);
          if (tokens > TOKEN_THRESHOLD) {
            console.log('[sqz][' + siteName + '] Paste intercepted: ' + clipText.length + ' chars, ' + tokens + ' tokens');
            handlePastedText(el, clipText).catch((e) => {
              console.warn('[sqz][' + siteName + '] paste compress error:', e);
            });
            return;
          }
        }
      } catch (e) { /* fall through */ }
      // Fallback: check editor content after paste settles
      setTimeout(() => {
        handleInput(el).catch((e) => {
          console.warn('[sqz][' + siteName + '] paste handler error:', e);
        });
      }, 200);
    }, true); // capturing phase

    // MutationObserver fallback for Tiptap/ProseMirror
    let debounceTimer = null;
    const editorObserver = new MutationObserver(() => {
      clearTimeout(debounceTimer);
      debounceTimer = setTimeout(() => {
        handleInput(el).catch((e) => {
          console.warn('[sqz][' + siteName + '] mutation handler error:', e);
        });
      }, 300);
    });
    editorObserver.observe(el, { childList: true, subtree: true, characterData: true });

    const submitEl = getSubmit ? getSubmit() : null;
    if (submitEl && !submitEl._sqzAttached) {
      submitEl._sqzAttached = true;
      submitEl.addEventListener('click', () => {
        try { handleSubmit(); } catch (e) {
          console.warn('[sqz][' + siteName + '] submit handler error:', e);
        }
      });
    }

    el.addEventListener('keydown', (evt) => {
      if (evt.key === 'Enter' && !evt.shiftKey) {
        try { handleSubmit(); } catch (e) {
          console.warn('[sqz][' + siteName + '] keydown handler error:', e);
        }
      }
    });

    return true;
  }

  // Retry attachment via MutationObserver (handles SPA navigation)
  if (!tryAttach()) {
    const observer = new MutationObserver(() => {
      try {
        if (tryAttach()) observer.disconnect();
      } catch (e) {
        console.warn('[sqz][' + siteName + '] DOM observer error:', e);
        observer.disconnect();
      }
    });
    observer.observe(document.body, { childList: true, subtree: true });
  }
}
