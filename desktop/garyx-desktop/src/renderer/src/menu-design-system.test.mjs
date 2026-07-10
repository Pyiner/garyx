import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Contract tests for the shared floating menu / popover design system.
// The reference values were extracted 1:1 from the ChatGPT/Codex Mac app via
// CDP-measured computed styles (see styles/menus.css header). These tests pin
// the recipe so it cannot silently drift back into per-surface forks.

const rendererDir = path.dirname(fileURLToPath(import.meta.url));

const read = (relativePath) =>
  readFileSync(path.join(rendererDir, relativePath), 'utf8');

const menusCss = read('styles/menus.css');
const baseCss = read('styles/base.css');

test('menu tokens pin the extracted ChatGPT/Codex recipe', () => {
  const expectations = [
    // surface: white @90% + 8px blur, radius 15, padding 4, hairline+drop shadow
    '--menu-surface-bg: rgba(255, 255, 255, 0.9);',
    '--menu-surface-blur: 8px;',
    '--menu-surface-radius: var(--radius-xl);',
    '--menu-surface-padding: 4px;',
    '0 0 0 0.5px var(--color-token-border)',
    '0 8px 16px -4px rgba(0, 0, 0, 0.12);',
    // item: 13px/445, radius 12.5, padding 5px 8px, 6px gap, icon 16 @75%
    '--menu-item-radius: var(--radius-lg);',
    '--menu-item-padding-y: 5px;',
    '--menu-item-padding-x: 8px;',
    '--menu-item-font-size: var(--text-base);',
    '--menu-item-font-weight: 445;',
    '--menu-item-line-height: 1.4286;',
    '--menu-item-gap: 6px;',
    '--menu-item-hover-bg: var(--color-token-row-hover);',
    '--menu-item-icon-size: 16px;',
    '--menu-item-icon-opacity: 0.75;',
    // shortcut and separator
    '--menu-shortcut-color: var(--color-token-description-foreground);',
    '--menu-separator-color: var(--color-token-border);',
    // trigger: 28×28, radius 10, tertiary icon color, hover wash
    '--menu-trigger-size: 28px;',
    '--menu-trigger-radius: var(--radius-md);',
    '--menu-trigger-color: var(--color-token-description-foreground);',
    '--menu-trigger-hover-bg: var(--color-token-row-hover);',
  ];
  for (const expected of expectations) {
    assert.ok(menusCss.includes(expected), `menus.css must define ${expected}`);
  }
});

test('token indirections resolve to the measured reference values', () => {
  // radius scale: md 10px, lg 12.5px, xl 15px (measured trigger/item/surface)
  assert.ok(baseCss.includes('--radius-md: calc(0.5rem * 1.25);'));
  assert.ok(baseCss.includes('--radius-lg: calc(0.625rem * 1.25);'));
  assert.ok(baseCss.includes('--radius-xl: calc(0.75rem * 1.25);'));
  // foreground family and washes (measured rgba(26, 28, 31, …))
  assert.ok(baseCss.includes('--color-token-foreground: #1a1c1f;'));
  assert.ok(baseCss.includes('--color-token-row-hover: rgba(26, 28, 31, 0.053);'));
  assert.ok(baseCss.includes('--color-token-border: rgba(26, 28, 31, 0.078);'));
  assert.ok(
    baseCss.includes('--color-token-description-foreground: rgba(26, 28, 31, 0.495);'),
  );
  // item font size token: 13px
  assert.ok(baseCss.includes('--text-base: 13px;'));
});

test('recipe styles every shared floating slot from the tokens', () => {
  const slots = [
    "[data-slot='dropdown-menu-content']",
    "[data-slot='dropdown-menu-sub-content']",
    "[data-slot='select-content']",
    "[data-slot='dropdown-menu-item']",
    "[data-slot='dropdown-menu-checkbox-item']",
    "[data-slot='dropdown-menu-sub-trigger']",
    "[data-slot='select-item']",
    "[data-slot='dropdown-menu-shortcut']",
    "[data-slot='dropdown-menu-separator']",
    '.icon-menu-trigger',
  ];
  for (const slot of slots) {
    assert.ok(menusCss.includes(slot), `menus.css must style ${slot}`);
  }
  // The recipe must stay overridable by per-surface CSS files.
  assert.ok(menusCss.includes('@layer components'));
});

test('shared components carry no local surface theming', () => {
  const dropdownSource = read('components/ui/dropdown-menu.tsx');
  const selectSource = read('components/ui/select.tsx');
  for (const source of [dropdownSource, selectSource]) {
    assert.ok(!/#[0-9a-fA-F]{3,8}\b/.test(source), 'no hard-coded hex colors');
    assert.ok(!source.includes('shadow-lg'), 'no local shadow recipe');
    assert.ok(!source.includes('bg-popover'), 'surface color comes from menus.css');
  }
  assert.ok(
    dropdownSource.includes("data-slot=\"dropdown-menu-shortcut\""),
    'DropdownMenuShortcut is exported from the shared component',
  );
});

test('retired per-surface menu forks stay deleted', () => {
  const conversationCss = read('styles/conversation.css');
  assert.ok(!conversationCss.includes('.thread-title-menu-item'));
  assert.ok(!conversationCss.includes('.thread-title-menu-shortcut'));
  assert.ok(!conversationCss.includes('.conversation-title-menu-trigger'));
  const composerCss = read('styles/composer.css');
  assert.ok(!composerCss.includes('.floating-action-menu-row {'));
  assert.ok(!composerCss.includes('rgba(13, 13, 13, 0.52)'));
  const taskForestCss = read('styles/task-forest.css');
  assert.ok(!taskForestCss.includes('#ececea'));
  const gatewaySwitcherSource = read('GatewaySwitcher.tsx');
  assert.ok(
    gatewaySwitcherSource.includes('menu-popover-surface gateway-switcher-popover'),
    'gateway popover opts into the shared surface marker',
  );
  assert.ok(
    gatewaySwitcherSource.includes('menu-item-two-line gateway-switcher-item'),
    'gateway rows use the shared two-line variant',
  );
});

test('per-surface CSS never redefines the menu surface or row identity', () => {
  // The recipe in menus.css is the only place allowed to give menus their
  // visual identity. Per-surface CSS may set widths, heights, and layout for
  // menu slots, but any of these properties on a menu selector is a fork.
  const slotPattern =
    /dropdown-menu-content|dropdown-menu-sub-content|select-content|dropdown-menu-item|dropdown-menu-checkbox-item|dropdown-menu-sub-trigger|select-item(?!-indicator)|dropdown-menu-label|select-label|dropdown-menu-separator|select-separator|dropdown-menu-shortcut|menu-item-two-line|menu-popover-surface|popover-content|-popover\b/;
  const forbiddenProps = [
    'background',
    'background-color',
    'border',
    'border-radius',
    'box-shadow',
    'backdrop-filter',
    '-webkit-backdrop-filter',
    'color',
    'font-size',
    'font-weight',
    'line-height',
    'min-height',
  ];
  const stylesDirPath = path.join(rendererDir, 'styles');
  const violations = [];
  for (const file of readdirSync(stylesDirPath)) {
    if (!file.endsWith('.css') || file === 'menus.css') {
      continue;
    }
    const css = readFileSync(path.join(stylesDirPath, file), 'utf8')
      .replace(/\/\*[\s\S]*?\*\//g, '');
    // Walk rule blocks: selector { declarations }
    const rulePattern = /([^{}]+)\{([^{}]*)\}/g;
    let match;
    while ((match = rulePattern.exec(css)) !== null) {
      const selector = match[1].trim();
      const body = match[2];
      // Judge the subject (last compound) of each selector: descendant chrome
      // inside a menu (badges, glyphs) is not the menu surface itself.
      const subjects = selector
        .split(',')
        .map((part) => part.trim().split(/[\s>+~]+/).pop() ?? '');
      if (!subjects.some((subject) => slotPattern.test(subject))) {
        continue;
      }
      // Pseudo-element indicators (selection dots, chevrons) draw semantic
      // marks, not the row surface.
      if (/::(?:after|before)/.test(selector)) {
        continue;
      }
      for (const declaration of body.split(';')) {
        const property = declaration.split(':')[0]?.trim();
        if (property && forbiddenProps.includes(property)) {
          violations.push(`${file}: "${selector}" declares ${property}`);
        }
      }
    }
  }
  assert.deepEqual(
    violations,
    [],
    `menu identity forks found:\n${violations.join('\n')}`,
  );
});

test('no hand-rolled DOM menus outside the shared components', () => {
  // Every DOM menu must be a shared DropdownMenu so the recipe applies.
  // (Native main-process menus use aria-haspopup on the trigger only.)
  const offenders = [];
  const walk = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const entryPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (entry.name === 'node_modules') continue;
        walk(entryPath);
        continue;
      }
      if (!/\.(tsx|ts)$/.test(entry.name)) continue;
      if (entryPath.includes(`${path.sep}components${path.sep}ui${path.sep}`)) {
        continue;
      }
      const source = readFileSync(entryPath, 'utf8');
      if (/role="menu"/.test(source)) {
        offenders.push(path.relative(rendererDir, entryPath));
      }
    }
  };
  walk(rendererDir);
  assert.deepEqual(
    offenders,
    [],
    `hand-rolled role="menu" found in: ${offenders.join(', ')}`,
  );
});

test('menus.css is imported into the renderer stylesheet chain', () => {
  const stylesCss = read('styles.css');
  const menusIndex = stylesCss.indexOf('./styles/menus.css');
  const baseIndex = stylesCss.indexOf('./styles/base.css');
  const conversationIndex = stylesCss.indexOf('./styles/conversation.css');
  assert.ok(menusIndex > -1, 'menus.css must be imported');
  assert.ok(baseIndex > -1 && menusIndex > baseIndex, 'tokens load before recipe');
  assert.ok(
    conversationIndex > menusIndex,
    'per-surface CSS loads after the shared recipe so overrides keep working',
  );
});
