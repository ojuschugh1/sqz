// sqz content script — ChatGPT (chat.openai.com / chatgpt.com)
// Intercepts the prompt textarea and compresses content > 500 tokens.

'use strict';

(function () {
  function getInput() {
    // ChatGPT uses a contenteditable div as the prompt editor
    return (
      document.querySelector('#prompt-textarea') ||
      document.querySelector('div[contenteditable="true"][data-id]') ||
      document.querySelector('textarea[data-id]') ||
      document.querySelector('div[contenteditable="true"].ProseMirror') ||
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

    // Remove ChatGPT's auto-generated file attachments from pasted text
    // Search the entire document for "Remove file" buttons — most reliable approach
    try {
      // Find ALL "Remove file" buttons anywhere in the document
      var allButtons = document.querySelectorAll('button[aria-label]');
      allButtons.forEach(function(btn) {
        var label = btn.getAttribute('aria-label') || '';
        if (label.startsWith('Remove file') || label.startsWith('Remove File')) {
          btn.click();
        }
      });
    } catch (e) {
      console.warn('[sqz][ChatGPT] Could not remove attachments:', e);
    }

    if (el.tagName === 'TEXTAREA') {
      // Use native input value setter so React state updates
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
      // contenteditable (ProseMirror) — use Selection API scoped to the element
      // to avoid document.execCommand('selectAll') navigating away on Chrome
      el.focus();
      try {
        const selection = window.getSelection();
        const range = document.createRange();
        range.selectNodeContents(el);
        selection.removeAllRanges();
        selection.addRange(range);
        document.execCommand('insertText', false, text);
        // If execCommand didn't work (some Chrome versions), fall back to direct DOM
        if (el.innerText !== text && el.textContent !== text) {
          el.innerText = text;
          el.dispatchEvent(new InputEvent('input', { bubbles: true, data: text }));
        }
      } catch (e) {
        // Last resort: direct assignment
        el.innerText = text;
        el.dispatchEvent(new InputEvent('input', { bubbles: true }));
      }
    }
  }

  function getSubmit() {
    return (
      document.querySelector('button[data-testid="send-button"]') ||
      document.querySelector('button[aria-label="Send message"]') ||
      null
    );
  }

  try {
    sqzAttachInterceptor({ getInput, getText, setText, getSubmit, siteName: 'ChatGPT' });
  } catch (e) {
    console.warn('[sqz][ChatGPT] DOM structure changed, falling back to pass-through:', e);
  }
})();
