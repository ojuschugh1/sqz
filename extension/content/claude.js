// sqz content script — Claude.ai
// Intercepts the prompt editor and compresses content > 500 tokens.

'use strict';

(function () {
  function getInput() {
    // Claude uses a ProseMirror contenteditable div
    return (
      document.querySelector('div[contenteditable="true"].ProseMirror') ||
      document.querySelector('div[contenteditable="true"][data-placeholder]') ||
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

    // Remove Claude's "pasted text" file attachments
    // These appear as cards above the editor when Claude auto-converts long pastes
    // DOM: div[data-testid="file-thumbnail"] > button[aria-label*="Pasted"]
    try {
      // Search the whole document for attachment thumbnails
      var thumbnails = document.querySelectorAll('[data-testid="file-thumbnail"]');
      thumbnails.forEach(function(thumb) {
        // Check if this is a "Pasted Text" attachment
        var btn = thumb.querySelector('button[aria-label*="Pasted"], button[aria-label*="pasted"]');
        if (btn) {
          // Look for a remove/close/X button within or near the thumbnail
          var removeBtn = thumb.querySelector('button[aria-label*="Remove"], button[aria-label*="remove"], button[aria-label*="Delete"], button[aria-label*="delete"], button[aria-label*="Close"]');
          if (removeBtn && removeBtn !== btn) {
            removeBtn.click();
          } else {
            // No explicit remove button — try removing the container
            var parent = thumb.closest('[class*="flex"]');
            if (parent && parent !== el && !parent.contains(el)) {
              thumb.remove();
            } else {
              thumb.remove();
            }
          }
        }
      });
    } catch (e) {
      console.warn('[sqz][Claude] Could not remove attachments:', e);
    }

    el.focus();
    // Select all existing content and replace
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
      document.querySelector('button[aria-label="Send Message"]') ||
      document.querySelector('button[type="submit"]') ||
      null
    );
  }

  try {
    console.log('[sqz][Claude] Content script loaded successfully');
    sqzAttachInterceptor({ getInput, getText, setText, getSubmit, siteName: 'Claude' });
  } catch (e) {
    console.warn('[sqz][Claude] DOM structure changed, falling back to pass-through:', e);
  }
})();
