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
    // DOM: div.relative.flex.group\/file with button containing × icon
    try {
      var fileTiles = document.querySelectorAll(
        '[class*="group/file"], [class*="file-tile"], [class*="file_tile"]'
      );
      fileTiles.forEach(function(tile) {
        // Look for a close/remove button inside
        var closeBtn = tile.querySelector(
          'button[aria-label*="Remove"], button[aria-label*="remove"], ' +
          'button[aria-label*="Delete"], button[aria-label*="Close"], ' +
          'button[class*="interactive-bg-secondary"]'
        );
        if (closeBtn) {
          closeBtn.click();
        } else {
          tile.remove();
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
      // contenteditable — use execCommand for undo-stack compatibility
      el.focus();
      document.execCommand('selectAll', false, null);
      document.execCommand('insertText', false, text);
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
