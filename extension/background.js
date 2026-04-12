// sqz service worker (Chrome MV3 background)
// Loads WASM compression engine and handles compression requests from
// content scripts. Background scripts have no CSP restrictions, so WASM
// loads reliably here unlike in content scripts.

'use strict';

// ---------------------------------------------------------------------------
// WASM engine state
// ---------------------------------------------------------------------------

let wasmReady = false;
let wasmEngine = null;
let wasmInitPromise = null;

async function initWasm() {
  if (wasmReady) return true;
  if (wasmInitPromise) return wasmInitPromise;

  wasmInitPromise = (async () => {
    try {
      // Import the no-modules WASM glue (sets global wasm_bindgen)
      importScripts('sqz_wasm.js');
      const wasmUrl = chrome.runtime.getURL('sqz_wasm_bg.wasm');
      await wasm_bindgen(wasmUrl);
      wasmEngine = new wasm_bindgen.SqzWasm('{}');
      wasmReady = true;
      console.log('[sqz] WASM engine initialized in background');
      return true;
    } catch (err) {
      console.warn('[sqz] WASM init failed in background:', err);
      wasmReady = false;
      return false;
    }
  })();

  return wasmInitPromise;
}

// Init WASM eagerly on service worker start
initWasm();

// ---------------------------------------------------------------------------
// Compression handler
// ---------------------------------------------------------------------------

function compressText(text) {
  if (!wasmReady || !wasmEngine) return null;
  try {
    const result = wasmEngine.compress(text);
    const str = (result !== null && result !== undefined) ? String(result) : null;
    if (str && str !== 'null' && str !== 'undefined' && str.length > 0) {
      return str;
    }
    return null;
  } catch (err) {
    console.warn('[sqz] WASM compress error:', err);
    return null;
  }
}

// ---------------------------------------------------------------------------
// Extension lifecycle
// ---------------------------------------------------------------------------

chrome.runtime.onInstalled.addListener((details) => {
  if (details.reason === 'install') {
    chrome.storage.local.set({
      sqzSettings: { enabled: true, showPreview: true, preset: 'default' },
      sqzStats: { totalOriginal: 0, totalCompressed: 0, compressions: 0 },
    });
  }
});

// ---------------------------------------------------------------------------
// Message handler
// ---------------------------------------------------------------------------

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
  if (message.type === 'COMPRESS') {
    // Ensure WASM is loaded, then compress
    initWasm().then((ok) => {
      if (!ok) {
        sendResponse({ compressed: null, error: 'wasm_unavailable' });
        return;
      }
      const compressed = compressText(message.text);
      sendResponse({ compressed: compressed, error: null });
    });
    return true; // keep channel open for async response
  }

  if (message.type === 'GET_SETTINGS') {
    chrome.storage.local.get(['sqzSettings'], (result) => {
      sendResponse(result.sqzSettings || { enabled: true, showPreview: true, preset: 'default' });
    });
    return true;
  }

  if (message.type === 'GET_STATS') {
    chrome.storage.local.get(['sqzStats'], (result) => {
      sendResponse(result.sqzStats || { totalOriginal: 0, totalCompressed: 0, compressions: 0 });
    });
    return true;
  }
});
