// sqz content script — Gemini (gemini.google.com)
// Intercepts the prompt input and compresses content > 500 tokens.

'use strict';

(function () {
  function getInput() {
    // Gemini uses a rich-text contenteditable div
    return (
      document.querySelector('div[contenteditable="true"].ql-editor') ||
      document.querySelector('rich-textarea div[contenteditable="true"]') ||
      document.querySelector('div[contenteditable="true"][aria-label]') ||
      document.querySelector('div[contenteditable="true"]') ||
      null
    );
  }

  function getText(el) {
    if (!el) return '';
    return el.innerText || el.textContent || '';
  }

  function setText(el, text) {
    if (!el) return;
    el.focus();
    const selection = window.getSelection();
    const range = document.createRange();
    range.selectNodeContents(el);
    selection.removeAllRanges();
    selection.addRange(range);
    document.execCommand('insertText', false, text);
    el.dispatchEvent(new InputEvent('input', { bubbles: true }));
  }

  function getSubmit() {
    return (
      document.querySelector('button[aria-label="Send message"]') ||
      document.querySelector('button.send-button') ||
      null
    );
  }

  try {
    sqzAttachInterceptor({ getInput, getText, setText, getSubmit, siteName: 'Gemini' });
  } catch (e) {
    console.warn('[sqz][Gemini] DOM structure changed, falling back to pass-through:', e);
  }
})();
