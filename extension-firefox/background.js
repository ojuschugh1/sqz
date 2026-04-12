// sqz background script (Firefox MV3)
// Loads WASM compression engine and handles compression requests from
// content scripts. Background scripts have no CSP restrictions.

'use strict';

const api = typeof browser !== 'undefined' ? browser : chrome;

// ---------------------------------------------------------------------------
// WASM engine state
// ---------------------------------------------------------------------------

let wasmReady = false;
let wasmEngine = null;

async function initWasm() {
  if (wasmReady) return true;
  try {
    // In Firefox, sqz_wasm.js is loaded as a background script in manifest.
    // wasm_bindgen is already available as a global.
    if (typeof wasm_bindgen !== 'function') {
      console.warn('[sqz] wasm_bindgen not available');
      return false;
    }
    const wasmUrl = api.runtime.getURL('sqz_wasm_bg.wasm');
    await wasm_bindgen(wasmUrl);
    wasmEngine = new wasm_bindgen.SqzWasm('{}');
    wasmReady = true;
    console.log('[sqz] WASM engine initialized in background');
    return true;
  } catch (err) {
    console.warn('[sqz] WASM init failed in background:', err);
    return false;
  }
}

// Init WASM eagerly
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

api.runtime.onInstalled.addListener((details) => {
  if (details.reason === 'install') {
    api.storage.local.set({
      sqzSettings: { enabled: true, showPreview: true, preset: 'default' },
      sqzStats: { totalOriginal: 0, totalCompressed: 0, compressions: 0 },
    });
  }
});

// ---------------------------------------------------------------------------
// Message handler
// ---------------------------------------------------------------------------

api.runtime.onMessage.addListener((message, _sender) => {
  // Firefox expects a returned Promise for async responses (not sendResponse + return true)
  if (message.type === 'COMPRESS') {
    return initWasm().then((ok) => {
      if (!ok) return { compressed: null, error: 'wasm_unavailable' };
      const compressed = compressText(message.text);
      return { compressed: compressed, error: null };
    });
  }

  if (message.type === 'GET_SETTINGS') {
    return api.storage.local.get(['sqzSettings']).then((result) => {
      return result.sqzSettings || { enabled: true, showPreview: true, preset: 'default' };
    });
  }

  if (message.type === 'GET_STATS') {
    return api.storage.local.get(['sqzStats']).then((result) => {
      return result.sqzStats || { totalOriginal: 0, totalCompressed: 0, compressions: 0 };
    });
  }
});
