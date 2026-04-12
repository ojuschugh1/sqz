// sqz popup — settings and stats display (Firefox)
'use strict';

const api = typeof browser !== 'undefined' ? browser : chrome;

const DEFAULT_SETTINGS = {
  enabled: true,
  showPreview: true,
  preset: 'default',
};

function loadStats(cb) {
  api.storage.local.get(['sqzStats']).then((result) => {
    cb(result.sqzStats || { totalOriginal: 0, totalCompressed: 0, compressions: 0 });
  });
}

function loadSettings(cb) {
  api.storage.local.get(['sqzSettings']).then((result) => {
    cb(Object.assign({}, DEFAULT_SETTINGS, result.sqzSettings || {}));
  });
}

function saveSettings(settings) {
  api.storage.local.set({ sqzSettings: settings });
}

function renderStats(stats) {
  document.getElementById('stat-compressions').textContent = stats.compressions;
  const saved = stats.totalOriginal - stats.totalCompressed;
  document.getElementById('stat-saved').textContent = saved > 0 ? saved.toLocaleString() : '0';
  if (stats.totalOriginal > 0) {
    const pct = Math.round((saved / stats.totalOriginal) * 100);
    document.getElementById('stat-reduction').textContent = pct + '%';
  } else {
    document.getElementById('stat-reduction').textContent = '—';
  }
}

function renderSettings(settings) {
  document.getElementById('setting-enabled').checked = settings.enabled;
  document.getElementById('setting-preview').checked = settings.showPreview;
  document.getElementById('setting-preset').value = settings.preset;
}

document.addEventListener('DOMContentLoaded', () => {
  loadStats(renderStats);
  loadSettings(renderSettings);

  document.getElementById('setting-enabled').addEventListener('change', (e) => {
    loadSettings((s) => { s.enabled = e.target.checked; saveSettings(s); });
  });

  document.getElementById('setting-preview').addEventListener('change', (e) => {
    loadSettings((s) => { s.showPreview = e.target.checked; saveSettings(s); });
  });

  document.getElementById('setting-preset').addEventListener('change', (e) => {
    loadSettings((s) => { s.preset = e.target.value; saveSettings(s); });
  });

  document.getElementById('btn-reset').addEventListener('click', () => {
    const empty = { totalOriginal: 0, totalCompressed: 0, compressions: 0 };
    api.storage.local.set({ sqzStats: empty }).then(() => renderStats(empty));
  });
});
