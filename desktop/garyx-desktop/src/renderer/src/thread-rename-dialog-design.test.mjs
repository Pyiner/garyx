import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

// Contract for the conversation rename dialog. Its geometry comes from the
// live Codex Mac app (Chrome 150) measured over CDP on 2026-07-17: 420×188,
// 20px insets, 12px section spacing, a 39px input, and 32px action buttons.
// Garyx deliberately strengthens muted-copy contrast and keyboard focus.

const rendererDir = path.dirname(fileURLToPath(import.meta.url));
const read = (relativePath) =>
  readFileSync(path.join(rendererDir, relativePath), 'utf8');

function parseRules(css) {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const rules = [];
  const rulePattern = /([^{}]+)\{([^{}]*)\}/g;
  let match;
  while ((match = rulePattern.exec(stripped)) !== null) {
    rules.push({
      selectors: match[1]
        .split(',')
        .map((selector) => selector.trim())
        .filter(Boolean),
      declarations: match[2]
        .split(';')
        .map((declaration) => declaration.trim())
        .filter(Boolean),
    });
  }
  return rules;
}

function expectRule(rules, selector, declarations) {
  const candidates = rules.filter((rule) => rule.selectors.includes(selector));
  assert.ok(candidates.length > 0, `expected a CSS rule for ${selector}`);
  for (const declaration of declarations) {
    assert.ok(
      candidates.some((rule) => rule.declarations.includes(declaration)),
      `${selector} must declare ${declaration}`,
    );
  }
}

test('rename dialog pins Codex geometry and accessible text states', () => {
  const rules = parseRules(read('styles/dialogs.css'));

  expectRule(rules, '.app-dialog-overlay.thread-rename-overlay', [
    'background: rgba(0, 0, 0, 0.133)',
  ]);
  expectRule(
    rules,
    '.thread-rename-dialog[data-slot="dialog-content"][data-size="compact"]',
    [
      '--app-dialog-card-width: min(420px, var(--app-dialog-available-width))',
      'padding: 0',
      'border: 0',
      'border-radius: 25px',
      'background: rgba(255, 255, 255, 0.9)',
      'color: rgb(13, 13, 13)',
      'font: 445 16px/24px -apple-system, "system-ui", "Segoe UI", sans-serif',
      '-webkit-backdrop-filter: blur(24px)',
      'backdrop-filter: blur(24px)',
      'box-shadow: 0 0 0 0.5px rgba(13, 13, 13, 0.08), 0 4px 8px -2px rgba(0, 0, 0, 0.1)',
    ],
  );
  expectRule(rules, '.thread-rename-form', [
    'display: flex',
    'flex-direction: column',
    'gap: 0',
    'padding: 20px',
    'font: 445 14px/21px -apple-system, "system-ui", "Segoe UI", sans-serif',
  ]);
  expectRule(rules, '.thread-rename-copy', [
    'gap: 4px',
    'padding: 0',
  ]);
  expectRule(rules, '.thread-rename-title[data-slot="dialog-title"]', [
    'font-size: 20px',
    'font-weight: 600',
    'line-height: 28px',
  ]);
  expectRule(rules, '.thread-rename-description[data-slot="dialog-description"]', [
    'color: rgba(13, 13, 13, 0.62)',
    'font-size: 14px',
    'font-weight: 445',
    'line-height: 21px',
  ]);
  expectRule(rules, '.thread-rename-input', [
    'height: 39px',
    'margin-top: 12px',
    'padding: 8px 12px',
    'border: 1px solid rgba(13, 13, 13, 0.08)',
    'border-radius: 15px',
    'background: transparent',
    'font: 445 14px/21px -apple-system, "system-ui", "Segoe UI", sans-serif',
    'box-shadow: 0 1px 2px -1px rgba(0, 0, 0, 0.08)',
  ]);
  expectRule(rules, '.thread-rename-input:focus-visible', [
    'border-color: rgb(1, 105, 204)',
    'box-shadow: 0 0 0 2px rgba(1, 105, 204, 0.2), 0 1px 2px -1px rgba(0, 0, 0, 0.08)',
  ]);
  expectRule(rules, '.thread-rename-input::placeholder', [
    'color: rgba(13, 13, 13, 0.62)',
  ]);
  expectRule(rules, '.thread-rename-actions', [
    'height: 32px',
    'margin-top: 12px',
    'gap: 12px',
  ]);
  expectRule(rules, '.thread-rename-button', [
    'height: 32px',
    'padding: 6px 16px',
    'border: 1px solid rgba(13, 13, 13, 0.08)',
    'border-radius: 12.5px',
    'font: 445 14px/18px -apple-system, "system-ui", "Segoe UI", sans-serif',
  ]);
  expectRule(rules, '.thread-rename-button:disabled', [
    'cursor: not-allowed',
    'opacity: 0.4',
  ]);
});

test('rename dialog pins Codex close and action states', () => {
  const rules = parseRules(read('styles/dialogs.css'));

  expectRule(rules, '.thread-rename-close', [
    'top: 16px',
    'right: 16px',
    'width: 24px',
    'height: 24px',
    'padding: 4px',
    'border-radius: 4px',
    'color: rgba(13, 13, 13, 0.8)',
  ]);
  expectRule(rules, '.thread-rename-close:hover', [
    'background: rgba(13, 13, 13, 0.055)',
  ]);
  expectRule(rules, '.thread-rename-close:focus-visible', [
    'box-shadow: 0 0 0 1px rgb(1, 105, 204)',
  ]);
  expectRule(rules, '.thread-rename-close svg', [
    'width: 16px',
    'height: 16px',
  ]);
  expectRule(rules, '.thread-rename-button-secondary', [
    'background: rgba(255, 255, 255, 0.96)',
  ]);
  expectRule(rules, '.thread-rename-button-secondary:hover:not(:disabled)', [
    'background: rgba(13, 13, 13, 0.055)',
  ]);
  expectRule(rules, '.thread-rename-button-primary', [
    'background: rgb(13, 13, 13)',
  ]);
  expectRule(rules, '.thread-rename-button-primary:hover:not(:disabled)', [
    'background: rgba(13, 13, 13, 0.8)',
  ]);
});

test('rename dialog owns its overlay and Radix open-focus behavior', () => {
  const source = read('ConversationHeaderTitle.tsx');
  const dialogSource = read('components/ui/dialog.tsx');
  const titleRootSource = read('app-shell/components/ConversationTitleRoot.tsx');
  const channelPluginsCss = read('styles/channel-plugins.css');

  assert.match(source, /overlayClassName="thread-rename-overlay"/);
  assert.match(source, /<X aria-hidden size=\{16\} strokeWidth=\{2\} \/>/);
  assert.match(source, /placeholder=\{t\('Add title…'\)\}/);
  assert.match(
    source,
    /const titleInputRef = useRef<HTMLInputElement \| null>\(null\)/,
  );
  assert.match(
    source,
    /onOpenAutoFocus=\{\(event\) => \{[\s\S]*?event\.preventDefault\(\);[\s\S]*?input\.focus\(\);[\s\S]*?input\.select\(\);[\s\S]*?\}\}/,
  );
  assert.doesNotMatch(titleRootSource, /threadTitleInputRef/);
  assert.doesNotMatch(channelPluginsCss, /\[data-slot="dialog-overlay"\]/);
  assert.doesNotMatch(channelPluginsCss, /\.modal-overlay/);
  assert.doesNotMatch(channelPluginsCss, /\.tasks-modal-backdrop/);
  assert.match(dialogSource, /overlayClassName\?: string/);
  assert.match(dialogSource, /<DialogOverlay className=\{overlayClassName\} \/>/);
});
