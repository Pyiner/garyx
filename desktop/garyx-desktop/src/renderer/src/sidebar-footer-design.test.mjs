import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Contract for the bottom-left sidebar footer. The measurements are from the
// Codex Mac app's live DOM/computed styles: a 46px footer, 8px horizontal
// insets and gap, a 29px identity row, and a 32px trailing utility button.

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
      selectors: match[1].split(',').map((selector) => selector.trim()),
      declarations: match[2]
        .split(';')
        .map((declaration) => declaration.trim())
        .filter(Boolean),
    });
  }
  return rules;
}

function expectRule(css, selector, expectedDeclarations) {
  const rules = parseRules(css).filter((candidate) =>
    candidate.selectors.includes(selector));
  assert.ok(rules.length > 0, `missing sidebar footer rule ${selector}`);
  for (const expected of expectedDeclarations) {
    assert.ok(
      rules.some((rule) =>
        rule.declarations.some((declaration) => declaration.includes(expected))),
      `${selector} must declare ${expected}`,
    );
  }
}

test('sidebar footer pins the Codex frame and divider geometry', () => {
  const css = read('styles/providers.css');
  expectRule(css, '.sidebar-footer', [
    'flex: 0 0 46px',
    'height: 46px',
    'margin: 0 -8px -10px',
    'padding: 0',
  ]);
  expectRule(css, '.sidebar-footer::before', [
    'height: 0.5px',
    'background: rgba(26, 28, 31, 0.1)',
  ]);
});

test('gateway and settings controls pin the Codex footer alignment', () => {
  const layoutCss = read('styles/workflows.css');
  expectRule(layoutCss, '.gateway-identity-bar', [
    'gap: 8px',
    'height: 46px',
    'padding: 0 8px',
  ]);
  expectRule(layoutCss, '.gateway-identity-main', [
    'gap: 8px',
    'height: 29px',
    'padding: 0 8px',
    'border-radius: 10px',
    'font-family: -apple-system',
    'font-size: 14px',
    'font-weight: 445',
    'line-height: 21px',
  ]);
  expectRule(layoutCss, '.gateway-identity-main:hover', [
    'background: var(--color-token-row-hover)',
  ]);

  const controlsCss = read('styles/task-forest.css');
  expectRule(controlsCss, '.gateway-identity-name', [
    'font-size: inherit',
    'font-weight: inherit',
    'line-height: inherit',
  ]);
  expectRule(controlsCss, '.gateway-identity-gear', [
    'gap: 4px',
    'width: 32px',
    'height: 32px',
    'padding: 4px 0',
    'border: 1px solid transparent',
    'border-radius: 10px',
    'color: var(--color-token-description-foreground)',
    'font: 445 16px/24px -apple-system',
  ]);
  expectRule(controlsCss, '.gateway-identity-gear:hover', [
    'background: var(--color-token-row-hover)',
  ]);
});

test('gateway identity keeps Codex icon scale while preserving status semantics', () => {
  const glyphCss = read('styles/gateway-status.css');
  expectRule(glyphCss, '.gateway-identity-glyph', [
    'width: 18px',
    'height: 18px',
    'border-radius: 5px',
  ]);
  expectRule(glyphCss, '.gateway-identity-glyph > svg', [
    'width: 11px',
    'height: 11px',
  ]);
  expectRule(glyphCss, '.gateway-identity-glyph > .gateway-glyph-badge', [
    'width: 7px',
    'height: 7px',
    'box-shadow: 0 0 0 1.5px #f6f6f4',
  ]);

  const source = read('GatewaySwitcher.tsx');
  assert.ok(source.includes('size={18} strokeWidth={1.7}'));
  assert.ok(
    source.includes("${t('Switch gateway')}: ${currentLabel} · ${toneLabel}"),
  );
  assert.ok(!source.includes('gateway-identity-status'));
});
