import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

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
  const rule = rules.find((candidate) => candidate.selectors.includes(selector));
  assert.ok(rule, `expected a CSS rule for ${selector}`);
  for (const declaration of declarations) {
    assert.ok(
      rule.declarations.includes(declaration),
      `${selector} must declare ${declaration}`,
    );
  }
}

function classToken(token) {
  const escaped = token.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return new RegExp(`(^|[^A-Za-z0-9_-])${escaped}(?=$|[^A-Za-z0-9_-])`, 'm');
}

test('workspace picker recipe stays in the always-loaded dialog owner', () => {
  const entryCss = read('styles.css');
  const dialogsCss = read('styles/dialogs.css');
  const rules = parseRules(dialogsCss);

  assert.equal(
    entryCss.match(/@import "\.\/styles\/dialogs\.css";/g)?.length,
    1,
    'styles.css must import the dialog owner exactly once',
  );

  for (const selector of [
    '.workspace-picker-dialog',
    '.workspace-picker-search',
    '.workspace-picker-list',
    '.workspace-picker-row',
    '.workspace-picker-name',
    '.workspace-picker-path',
    '.workspace-picker-check',
    '.workspace-picker-empty',
    '.workspace-picker-footer',
  ]) {
    assert.ok(
      rules.some((rule) => rule.selectors.includes(selector)),
      `dialogs.css must own ${selector}`,
    );
  }

  expectRule(rules, '.workspace-picker-dialog', [
    'display: flex',
    'flex-direction: column',
  ]);
  expectRule(rules, '.workspace-picker-dialog [data-slot="dialog-close"]', [
    'top: 8px',
    'right: 14px',
  ]);
  expectRule(rules, '.workspace-picker-search', [
    'position: relative',
    'display: flex',
    'align-items: center',
  ]);
  expectRule(rules, '.workspace-picker-search svg', [
    'position: absolute',
    'left: 11px',
  ]);
  expectRule(rules, '.workspace-picker-search input', [
    'flex: 1',
    'padding-left: 33px',
  ]);
  expectRule(rules, '.workspace-picker-list', [
    'display: flex',
    'flex-direction: column',
    'max-height: 320px',
    'overflow-y: auto',
  ]);
  expectRule(rules, '.workspace-picker-row', [
    'display: flex',
    'width: 100%',
    'min-width: 0',
  ]);
  expectRule(rules, '.workspace-picker-name', [
    'overflow: hidden',
    'text-overflow: ellipsis',
    'white-space: nowrap',
  ]);
  expectRule(rules, '.workspace-picker-path', [
    'flex: 1',
    'min-width: 0',
    'overflow: hidden',
    'text-overflow: ellipsis',
    'white-space: nowrap',
  ]);
  expectRule(rules, '.workspace-picker-check', ['flex: none']);
});

test('audited orphan hooks are either retired or have a real owner', () => {
  for (const [relativePath, token] of [
    ['NewThreadEmptyState.tsx', 'workspace-picker-trigger'],
    ['GatewaySettingsPanel.tsx', 'gateway-add-dialog'],
    ['app-shell/components/TasksPanel.tsx', 'tasks-agent-menu-item'],
    ['SkillsPanel.tsx', 'skills-create-field-group'],
    ['GatewaySettingsPanel.tsx', 'settings-update-row'],
    ['ComposerForm.tsx', 'composer-bot-submenu'],
    ['ComposerForm.tsx', 'composer-menu-item'],
    ['ComposerForm.tsx', 'composer-attachments'],
    ['BotConsoleView.tsx', 'bot-console-empty-card'],
  ]) {
    assert.doesNotMatch(
      read(relativePath),
      classToken(token),
      `${relativePath} must retire ${token}`,
    );
  }

  const sideToolsSource = read('app-shell/components/SideToolsPanel.tsx');
  assert.match(sideToolsSource, classToken('browser-side-panel-loading'));

  const browserRules = parseRules(read('styles/browser.css'));
  expectRule(browserRules, '.browser-side-panel-loading', [
    'grid-template-rows: minmax(0, 1fr)',
    'place-items: center',
  ]);
  expectRule(browserRules, '.browser-side-panel-loading::after', [
    'content: ""',
    'animation: browser-tab-spin 0.65s linear infinite',
  ]);
});

test('settings colors and provider labels remain on shared presentation paths', () => {
  const gatewaySettingsSource = read('GatewaySettingsPanel.tsx');
  assert.doesNotMatch(
    gatewaySettingsSource,
    /#[0-9a-fA-F]{3,8}\b/,
    'GatewaySettingsPanel must not carry hard-coded hex colors',
  );

  const composerSource = read('ComposerForm.tsx');
  assert.doesNotMatch(composerSource, /function providerOptionLabel\b/);
  assert.match(
    composerSource,
    /providerLabel as sharedProviderLabel/,
  );
  assert.match(
    composerSource,
    /agentLabel \|\| sharedProviderLabel\(composerProviderType\)/,
  );

  const appShellSource = read('app-shell/AppShell.tsx');
  assert.doesNotMatch(
    appShellSource,
    /label: providerLabel\("claude_code"\)/,
    'an empty agent catalog must not materialize Claude',
  );
});
