import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// Expanded Codex activity grows with its content and lets the transcript own
// vertical scrolling. Nested scroll traps make completed commands and results
// look truncated and are especially awkward with a trackpad.

const rendererDir = path.dirname(fileURLToPath(import.meta.url));
const css = readFileSync(path.join(rendererDir, 'styles/turn-summary.css'), 'utf8');
const component = readFileSync(path.join(rendererDir, 'tool-trace.tsx'), 'utf8');

function declarationsFor(selector) {
  const stripped = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const rulePattern = /([^{}]+)\{([^{}]*)\}/g;
  const declarations = [];
  let match;
  while ((match = rulePattern.exec(stripped)) !== null) {
    const selectors = match[1].split(',').map((part) => part.trim());
    if (selectors.includes(selector)) {
      declarations.push(
        ...match[2]
          .split(';')
          .map((part) => part.trim())
          .filter(Boolean),
      );
    }
  }
  assert.ok(declarations.length > 0, `missing tool trace rule ${selector}`);
  return declarations;
}

test('expanded tool activity grows naturally without nested vertical scrolling', () => {
  for (const selector of ['.tool-trace-group-list', '.tool-trace-children-scroll']) {
    const declarations = declarationsFor(selector);
    assert.ok(
      declarations.includes('overflow: visible'),
      `${selector} must leave vertical scrolling to the transcript`,
    );
    assert.ok(
      declarations.every((declaration) => !declaration.startsWith('max-height:')),
      `${selector} must not cap expanded activity height`,
    );
    assert.ok(
      declarations.every(
        (declaration) => !declaration.startsWith('overflow') || declaration === 'overflow: visible',
      ),
      `${selector} must not create a nested vertical scroller`,
    );
  }
});

test('expanded projected sections keep File then Call then Diff then Result order', () => {
  const detailStart = component.indexOf('<div className="tool-trace-details">');
  assert.ok(detailStart >= 0, 'missing expanded detail container');
  const detail = component.slice(detailStart, component.indexOf('{nestedChildren ?', detailStart));
  const positions = [
    detail.indexOf('merged.pathDetail'),
    detail.indexOf('merged.inputDetail'),
    detail.indexOf('merged.diffLines'),
    detail.indexOf('merged.resultDetail'),
  ];
  assert.ok(positions.every((position) => position >= 0), 'all four projected sections must exist');
  assert.deepEqual([...positions].sort((left, right) => left - right), positions);
});
