import test from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { cjk } from '@streamdown/cjk';
import { createCodePlugin } from '@streamdown/code';
import { Streamdown } from 'streamdown';

import { prepareMessageMarkdown } from './message-markdown-preprocess.ts';
import { CHAT_MESSAGE_REMARK_PLUGINS } from './message-rich-text-plugins.ts';

const garyxCodePlugin = createCodePlugin({
  themes: ['github-light', 'github-dark'],
});

const STREAMDOWN_CONTROLS = {
  code: {
    copy: true,
    download: false,
  },
  mermaid: false,
  table: false,
};

function renderChatMarkdown({ text, tone = 'default' }) {
  const prepared = prepareMessageMarkdown(text, {
    surfaceCustomXmlTags: tone !== 'assistant',
  });
  return renderToStaticMarkup(
    React.createElement(
      'div',
      {
        className: `message-rich ${
          tone === 'assistant' ? 'message-rich-assistant' : 'message-rich-default'
        }`,
      },
      React.createElement(
        Streamdown,
        {
          controls: STREAMDOWN_CONTROLS,
          dir: 'auto',
          lineNumbers: false,
          mode: 'streaming',
          normalizeHtmlIndentation: true,
          plugins: { cjk, code: garyxCodePlugin },
          remarkPlugins: CHAT_MESSAGE_REMARK_PLUGINS,
        },
        prepared,
      ),
    ),
  );
}

function brCount(html) {
  return (html.match(/<br\/?>/g) || []).length;
}

for (const tone of ['default', 'assistant']) {
  test(`single newline renders as a soft break for ${tone} messages`, () => {
    const html = renderChatMarkdown({
      text: '第一行\n第二行',
      tone,
    });

    assert.match(html, /第一行<br\/?>\s*第二行/);
    assert.equal(brCount(html), 1);
  });
}

test('hard line break still renders as one break', () => {
  const html = renderChatMarkdown({
    text: 'hard break  \nnext',
  });

  assert.match(html, /hard break<br\/?>\s*next/);
  assert.equal(brCount(html), 1);
});

test('blank line still creates separate paragraphs', () => {
  const html = renderChatMarkdown({
    text: 'first paragraph\n\nsecond paragraph',
  });

  assert.match(html, /<p>first paragraph<\/p>/);
  assert.match(html, /<p>second paragraph<\/p>/);
  assert.equal(brCount(html), 0);
});

test('list items do not get injected breaks', () => {
  const html = renderChatMarkdown({
    text: '- first item\n- second item',
  });

  assert.match(html, /<ul\b/);
  assert.match(html, /<li\b[^>]*>first item<\/li>/);
  assert.match(html, /<li\b[^>]*>second item<\/li>/);
  assert.equal(brCount(html), 0);
});

test('fenced code keeps literal newlines instead of soft break elements', () => {
  const html = renderChatMarkdown({
    text: '```text\nalpha\nbeta\n```',
  });

  assert.doesNotMatch(html, /alpha<br\/?>beta/);
  assert.match(html, /data-streamdown="code-block"/);
});

test('inline code stays inline while surrounding text can soft-break', () => {
  const html = renderChatMarkdown({
    text: 'before `inline code`\nafter',
  });

  assert.match(html, /<code[^>]*>inline code<\/code><br\/?>\s*after/);
  assert.equal(brCount(html), 1);
});

test('GFM tables still render after preserving Streamdown default remark plugins', () => {
  const html = renderChatMarkdown({
    text: '| Name | Value |\n| --- | --- |\n| alpha | beta |',
  });

  assert.match(html, /<table\b/);
  assert.match(html, /<td\b[^>]*>alpha<\/td>/);
  assert.equal(brCount(html), 0);
});
