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

test('workspace picker recipe stays in an always-loaded owner stylesheet', () => {
  const entryCss = read('styles.css');
  const pickerCss = read('styles/workspace-picker.css');
  const dialogsCss = read('styles/dialogs.css');
  const rules = parseRules(pickerCss);
  const dialogRules = parseRules(dialogsCss);

  assert.equal(
    entryCss.match(/@import "\.\/styles\/workspace-picker\.css";/g)?.length,
    1,
    'styles.css must import the picker owner exactly once',
  );
  assert.equal(
    entryCss.match(/@import "\.\/styles\/dialogs\.css";/g)?.length,
    1,
    'styles.css must import the dialog owner exactly once',
  );

  // The shared picker body (WorkspacePickerContent) has one stylesheet
  // owner for both hosts (composer chip popover + in-form dialog).
  for (const selector of [
    '.workspace-picker-search',
    '.workspace-picker-search-input',
    '.workspace-picker-list',
    '.workspace-picker-item',
    '.workspace-picker-item-name',
    '.workspace-picker-item-path',
    '.workspace-picker-empty',
    '.workspace-picker-footer',
    '.workspace-picker-popover',
  ]) {
    assert.ok(
      rules.some((rule) => rule.selectors.includes(selector)),
      `workspace-picker.css must own ${selector}`,
    );
    assert.ok(
      !dialogRules.some((rule) => rule.selectors.includes(selector)),
      `dialogs.css must not fork ${selector}`,
    );
  }

  // The dialog host chrome stays with the dialog owner.
  assert.ok(
    dialogRules.some((rule) => rule.selectors.includes('.workspace-picker-dialog')),
    'dialogs.css owns the dialog host chrome',
  );

  expectRule(rules, '.workspace-picker-item', [
    'display: flex',
    'width: 100%',
  ]);
  expectRule(rules, '.workspace-picker-item-name', [
    'overflow: hidden',
    'text-overflow: ellipsis',
    'white-space: nowrap',
  ]);
  expectRule(rules, '.workspace-picker-item-path', [
    'flex: 1',
    'min-width: 0',
    'overflow: hidden',
    'text-overflow: ellipsis',
    'white-space: nowrap',
  ]);
  expectRule(rules, '.workspace-picker-list', [
    'display: flex',
    'flex-direction: column',
    'overflow-y: auto',
  ]);
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
  assert.match(
    appShellSource,
    /function openSettingsView\(\)[\s\S]*?\{ kind: "settings", tabId: "provider" \}/,
    'the Mac Settings button must always enter Providers first',
  );

  const settingsTabsSource = read('settings-tabs.ts');
  assert.ok(
    settingsTabsSource.indexOf("id: 'provider'") < settingsTabsSource.indexOf("id: 'labs'"),
    'Providers must be the first settings destination',
  );
});

test('provider accounts stay flat and multi-step sign-in keeps stable geometry', () => {
  const providerSource = read('settings/ProviderSettingsPanel.tsx');
  const providerRules = parseRules(read('styles/providers.css'));

  assert.match(providerSource, /className="codex-list-card provider-section-rows"/);
  assert.doesNotMatch(providerSource, /provider-account-(?:avatar|current)/);
  assert.match(
    providerSource,
    /className="provider-login-dialog"[\s\S]*?scroll="content"/,
    'the sign-in flow must keep its header and footer anchored while the step body changes',
  );
  expectRule(providerRules, '.provider-login-dialog[data-slot="dialog-content"]', [
    'width: min(640px, calc(100vw - 40px))',
    'height: min(300px, calc(100dvh - 40px))',
    'min-height: min(300px, calc(100dvh - 40px))',
    'max-height: min(300px, calc(100dvh - 40px))',
  ]);
  expectRule(providerRules, '.provider-default-row .provider-config-default-cell', [
    'width: 100%',
    'justify-content: flex-end',
  ]);
  expectRule(providerRules, '.provider-config-default-chip', [
    'flex: 0 1 auto',
    'max-width: 100%',
  ]);
  for (const level of ['healthy', 'warning', 'critical']) {
    expectRule(providerRules, `.provider-usage-meter[data-level="${level}"]`, [
      '--provider-usage-color: var(--color-token-text-primary)',
    ]);
  }
});
