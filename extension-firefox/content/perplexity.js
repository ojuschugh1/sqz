// sqz content script — Perplexity (perplexity.ai)
// Intercepts the prompt textarea and compresses content > 500 tokens.

'use strict';

(function () {
  function getInput() {
    return (
      document.querySelector('textarea[placeholder]') ||
      document.querySelector('div[contenteditable="true"]') ||
      document.querySelector('textarea') ||
      null
    );
  }

  function getText(el) {
    if (!el) return '';
    if (el.tagName === 'TEXTAREA') return el.value;
    return el.innerText || el.textContent || '';
  }

  function setText(el, text) {
    if (!el) return;
    if (el.tagName === 'TEXTAREA') {
      const nativeInputValueSetter = Object.getOwnPropertyDescriptor(
        window.HTMLTextAreaElement.prototype, 'value'
      );
      if (nativeInputValueSetter && nativeInputValueSetter.set) {
        nativeInputValueSetter.set.call(el, text);
      } else {
        el.value = text;
      }
      el.dispatchEvent(new Event('input', { bubbles: true }));
    } else {
      el.focus();
      const selection = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(el);
      selection.removeAllRanges();
      selection.addRange(range);
      document.execCommand('insertText', false, text);
      el.dispatchEvent(new InputEvent('input', { bubbles: true }));
    }
  }

  function getSubmit() {
    return (
      document.querySelector('button[aria-label="Submit"]') ||
      document.querySelector('button[type="submit"]') ||
      null
    );
  }

  try {
    sqzAttachInterceptor({ getInput, getText, setText, getSubmit, siteName: 'Perplexity' });
  } catch (e) {
    console.warn('[sqz][Perplexity] DOM structure changed, falling back to pass-through:', e);
  }
})();
