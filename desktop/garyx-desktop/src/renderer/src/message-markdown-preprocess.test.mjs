import test from 'node:test';
import assert from 'node:assert/strict';
import { defaultSchema } from 'hast-util-sanitize';
import { HTML_TAG_NAMES, stripGaryxInternalTags, escapeNonHtmlTagsOutsideCode, prepareMessageMarkdown } from './message-markdown-preprocess.ts';
const noGaryx = (s) => assert.doesNotMatch(s, /<\s*\/?\s*garyx_|<\s*\/?\s*system_instructions/);

test('paired block', () => assert.equal(stripGaryxInternalTags('<garyx_thread_metadata>\nthread_id: 1\n</garyx_thread_metadata>\n\nhello').trim(), 'hello'));
test('self-closing', () => assert.equal(stripGaryxInternalTags('a <garyx_models foo="1"/> b'), 'a  b'));
test('self-closing > in attr', () => assert.equal(stripGaryxInternalTags('<garyx_models data=">"/> visible').trim(), 'visible'));
test('system_instructions', () => assert.equal(stripGaryxInternalTags('<system_instructions>be nice</system_instructions>\n\nbody').trim(), 'body'));
test('multiple blocks', () => { const o = stripGaryxInternalTags('<garyx_models a="1"/>\nkeep me\n<garyx_memory_context>secret</garyx_memory_context>\ntail'); assert.match(o,/keep me/); assert.match(o,/tail/); noGaryx(o); assert.doesNotMatch(o,/secret/); });
test('streaming unclosed', () => assert.equal(stripGaryxInternalTags('visible\n\n<garyx_memory_context>\nhalf...').trim(), 'visible'));
test('streaming digits', () => assert.equal(stripGaryxInternalTags('x\n<garyx_model1').trim(), 'x'));
test('streaming prefix', () => { assert.equal(stripGaryxInternalTags('visible\n<garyx_').trim(), 'visible'); assert.equal(stripGaryxInternalTags('visible\n<system_instr').trim(), 'visible'); });
test('no strip prefix-match name', () => { assert.match(prepareMessageMarkdown('<system_instructions-v2>hi</system_instructions-v2>'), /system_instructions-v2/); });
test('no strip short partials', () => { assert.equal(prepareMessageMarkdown('prose <g'),'prose <g'); assert.equal(prepareMessageMarkdown('prose <s'),'prose <s'); assert.equal(prepareMessageMarkdown('prose <sys'),'prose <sys'); });
test('no strip mid-line partial', () => { const o = prepareMessageMarkdown('prose <garyx_me'); assert.match(o,/prose/); assert.match(o,/garyx_me/); });
test('prose complete mention surfaced (mid-line)', () => { const o = prepareMessageMarkdown('see the <garyx_memory_context> token'); assert.match(o,/&lt;garyx_memory_context&gt;/); assert.match(o,/token/); });
test('inline-code mention preserved', () => { const i='use `<garyx_memory_context>` here'; assert.equal(prepareMessageMarkdown(i), i); });
test('distant whitespace untouched', () => assert.equal(stripGaryxInternalTags('intro\n\n\nparagraph\n<garyx_models/>'), 'intro\n\n\nparagraph\n'));

// round-4
test('R4 P1a fenced inside internal removed', () => { const o = stripGaryxInternalTags('<garyx_memory_context>\n```\nsecret code\n```\n</garyx_memory_context>\n\nhi'); assert.equal(o.trim(),'hi'); assert.doesNotMatch(o,/secret/); assert.doesNotMatch(o,/```/); noGaryx(o); });
test('R4 P1a inline inside internal removed', () => assert.equal(stripGaryxInternalTags('<garyx_models>a `b` c</garyx_models>X').trim(), 'X'));
test('R4 P1b backtick-in-attr escaped', () => { const o = escapeNonHtmlTagsOutsideCode('<custom title="`code`">x</custom>'); assert.match(o,/&lt;custom title="`code`"&gt;/); assert.match(o,/x/); assert.match(o,/&lt;\/custom&gt;/); assert.doesNotMatch(o,/<custom/); });
test('R4 P1c streaming incomplete quoted attr stripped', () => assert.equal(stripGaryxInternalTags('visible\n<garyx_models data="').trim(), 'visible'));

// round-5
test('R5 P1a malformed many tags linear & unchanged', () => { const i='<custom "'.repeat(20000); assert.equal(escapeNonHtmlTagsOutsideCode(i), i); });
test('R5 P1b streaming > in unclosed quote stripped', () => assert.equal(stripGaryxInternalTags('visible\n<garyx_models data=">').trim(), 'visible'));
test('R5 P2d entity in attr escaped (& first)', () => assert.equal(escapeNonHtmlTagsOutsideCode('<custom data="&lt;">'), '&lt;custom data="&amp;lt;"&gt;'));

// round-6
test('R6 P1 dotted internal name surfaced, not stripped', () => {
  assert.match(prepareMessageMarkdown('<garyx_models.v2>hi</garyx_models.v2>'), /&lt;garyx_models\.v2&gt;/);
  assert.match(prepareMessageMarkdown('<system_instructions.v2>x</system_instructions.v2>'), /&lt;system_instructions\.v2&gt;/);
});
test('R6 P1 dotted <b.foo> escaped (surfaced), not kept raw', () => assert.equal(escapeNonHtmlTagsOutsideCode('<b.foo>x</b.foo>'), '&lt;b.foo&gt;x&lt;/b.foo&gt;'));
test('R6 P1 many MID-LINE internal openers stay linear & surfaced', () => { const i='x <garyx_a> '.repeat(20000); const o = prepareMessageMarkdown(i); assert.match(o,/&lt;garyx_a&gt;/); assert.doesNotMatch(o,/<garyx_a>/); });
test('R6 P1 non-code-aware close avoids over-strip w/ unbalanced inline backtick', () => { const o = stripGaryxInternalTags('<garyx_memory_context>secret `\n</garyx_memory_context>\nVISIBLE ` tail'); assert.match(o,/VISIBLE/); assert.doesNotMatch(o,/secret/); noGaryx(o); });
test('R6 P2 unclosed quote in non-internal tag does not suppress later escaping', () => { const o = escapeNonHtmlTagsOutsideCode('<custom title="unclosed\n<note>after'); assert.match(o,/&lt;note&gt;/); assert.match(o,/after/); });

// round-7
test('R7 P1a internal tag inside fenced code is intentionally PRESERVED', () => {
  const i = '```\n<garyx_memory_context>secret</garyx_memory_context>\n```';
  assert.equal(stripGaryxInternalTags(i), i); assert.equal(prepareMessageMarkdown(i), i);
});
test('R7 P1a internal tag inside inline code preserved', () => {
  const i = 'see `<garyx_memory_context>x</garyx_memory_context>` ok';
  assert.equal(prepareMessageMarkdown(i), i);
});
test('R7 P1b <garyx_models=foo> (no name terminator) NOT stripped, surfaced', () => {
  const o = prepareMessageMarkdown('<garyx_models=foo>VISIBLE'); assert.match(o,/VISIBLE/); assert.match(o,/&lt;garyx_models=foo&gt;/);
});
test('R7 P1b <b=1> not treated as allowlisted <b>, escaped', () => assert.equal(escapeNonHtmlTagsOutsideCode('<b=1>x'), '&lt;b=1&gt;x'));
test('R7 P1c streaming <system_status (not system_instructions prefix) NOT stripped', () => {
  assert.match(stripGaryxInternalTags('visible\n<system_status data="'), /system_status/);
});

// round-8
test('R8 P1 malformed garyx_ partial with bad terminator/dot/dash NOT stripped', () => {
  assert.match(stripGaryxInternalTags('<garyx_models=foo\nVISIBLE'), /VISIBLE/);
  assert.match(stripGaryxInternalTags('<garyx_models.v2\nVISIBLE'), /VISIBLE/);
  assert.match(stripGaryxInternalTags('<garyx_models-x\nVISIBLE'), /VISIBLE/);
});
test('R8 clean incomplete garyx_ family still stripped at block boundary', () => {
  assert.equal(stripGaryxInternalTags('visible\n<garyx_models foo').trim(), 'visible');
});

// round-9
test('R9 slash-junk after internal name NOT stripped (surfaced)', () => {
  assert.match(prepareMessageMarkdown('<garyx_models/foo>VISIBLE'), /VISIBLE/);
  assert.match(stripGaryxInternalTags('<garyx_models/foo\nVISIBLE'), /VISIBLE/);
});
test('R9 regression: <br/> stays allowlisted; <garyx_models/> still dropped', () => {
  assert.equal(escapeNonHtmlTagsOutsideCode('a<br/>b'), 'a<br/>b');
  assert.equal(stripGaryxInternalTags('a <garyx_models/> b'), 'a  b');
});

// round-10
test('R10 P1a URL autolink preserved (not escaped)', () => {
  assert.equal(escapeNonHtmlTagsOutsideCode('see <https://example.com> ok'), 'see <https://example.com> ok');
  assert.equal(prepareMessageMarkdown('<https://example.com/path?q=1>'), '<https://example.com/path?q=1>');
});
test('R10 P1a email autolink preserved', () => assert.equal(escapeNonHtmlTagsOutsideCode('mail <a.b@example.com> ok'), 'mail <a.b@example.com> ok'));
test('R10 P1a non-autolink custom still escaped', () => assert.equal(escapeNonHtmlTagsOutsideCode('<custom>x</custom>'), '&lt;custom&gt;x&lt;/custom&gt;'));
test('R10 P1a autolink-look does not break internal strip', () => assert.equal(stripGaryxInternalTags('<garyx_memory_context>s</garyx_memory_context>\n\nhi').trim(), 'hi'));
test('R10 P1b streaming partial self-closing internal stripped', () => {
  assert.equal(stripGaryxInternalTags('visible\n<garyx_models/').trim(), 'visible');
  assert.equal(stripGaryxInternalTags('visible\n<system_instructions/').trim(), 'visible');
});
test('R10 P1b /foo still surfaced (not over-stripped)', () => assert.match(stripGaryxInternalTags('<garyx_models/foo\nVISIBLE'), /VISIBLE/));

// 自定义标签可见
test('custom tag visible, block md preserved', () => { const o = escapeNonHtmlTagsOutsideCode('<custom a="1">\n## Title\n</custom>'); assert.match(o,/&lt;custom a="1"&gt;/); assert.match(o,/&lt;\/custom&gt;/); assert.match(o,/\n## Title\n/); });
test('self-closing custom escaped', () => assert.equal(escapeNonHtmlTagsOutsideCode('<skill name="x" />'), '&lt;skill name="x" /&gt;'));
test('custom > in attr escaped', () => { const o = escapeNonHtmlTagsOutsideCode('<custom data=">">x</custom>'); assert.match(o,/&lt;custom data="&gt;"&gt;/); assert.match(o,/x/); assert.match(o,/&lt;\/custom&gt;/); });
test('article surfaced', () => assert.equal(escapeNonHtmlTagsOutsideCode('<article>x</article>'), '&lt;article&gt;x&lt;/article&gt;'));
test('underscore tag escaped', () => assert.equal(escapeNonHtmlTagsOutsideCode('<_custom>x</_custom>'), '&lt;_custom&gt;x&lt;/_custom&gt;'));

// 零回归 + 白名单
test('allowlist no regression', () => { assert.equal(escapeNonHtmlTagsOutsideCode('a<br>b'),'a<br>b'); assert.equal(escapeNonHtmlTagsOutsideCode('<sub>x</sub>'),'<sub>x</sub>'); assert.equal(escapeNonHtmlTagsOutsideCode('<strike>x</strike> <tt>y</tt>'),'<strike>x</strike> <tt>y</tt>'); });
test('pin allowlist', () => assert.deepEqual([...HTML_TAG_NAMES].sort(), [...(defaultSchema.tagNames||[])].sort()));
test('a<b>c keeps b', () => assert.equal(escapeNonHtmlTagsOutsideCode('a<b>c'), 'a<b>c'));

// 代码区跳过
test('fenced untouched', () => { const i='```\n<custom>hi</custom>\n```'; assert.equal(escapeNonHtmlTagsOutsideCode(i), i); });
test('4-backtick fence', () => { const i='````\n<custom>x</custom>\n````'; assert.equal(escapeNonHtmlTagsOutsideCode(i), i); });
test('mixed-char no close', () => { const i='```\n<custom>\n~~~\nstill code\n```'; assert.equal(escapeNonHtmlTagsOutsideCode(i), i); });
test('CRLF fence', () => { const i='```\r\n<custom>\r\n```\r\nafter <note>'; const o = escapeNonHtmlTagsOutsideCode(i); assert.match(o,/```\r\n<custom>\r\n```/); assert.match(o,/&lt;note&gt;/); });
test('backtick info not fence', () => { const o = escapeNonHtmlTagsOutsideCode('```x`\n<custom>\nmore'); assert.match(o,/&lt;custom&gt;/); });
test('unclosed fence to EOF', () => { const i='```\n<custom>partial'; assert.equal(escapeNonHtmlTagsOutsideCode(i), i); });
test('inline exact-length', () => { const i='`code `` still code <custom> end`'; assert.equal(escapeNonHtmlTagsOutsideCode(i), i); });
test('double-backtick inline', () => assert.equal(escapeNonHtmlTagsOutsideCode('``<custom>``'), '``<custom>``'));
test('indented code limitation (tag still surfaced; exact whitespace is don\'t-care)', () => assert.match(escapeNonHtmlTagsOutsideCode('    <custom>'), /&lt;custom&gt;/));

// 非标签 / no-op
test('non-tag <', () => assert.equal(escapeNonHtmlTagsOutsideCode('a < b and 5 < 10'), 'a < b and 5 < 10'));
test('no-op byte-identical', () => { assert.equal(prepareMessageMarkdown('\nhello\n'),'\nhello\n'); assert.equal(prepareMessageMarkdown('    code line'),'    code line'); assert.equal(prepareMessageMarkdown('hard break  \nnext'),'hard break  \nnext'); assert.equal(prepareMessageMarkdown('```\n\n\n\nx\n```'),'```\n\n\n\nx\n```'); });
test('combined', () => { const o = prepareMessageMarkdown('<garyx_thread_metadata>x</garyx_thread_metadata>\n\n<custom>\n## Hi\n</custom>'); noGaryx(o); assert.match(o,/&lt;custom&gt;/); assert.match(o,/\n## Hi\n/); });
test('assistant mode keeps custom XML raw for Streamdown sanitization', () => {
  const o = prepareMessageMarkdown('<custom>\n## Hi\n</custom>', { surfaceCustomXmlTags: false });
  assert.equal(o, '<custom>\n## Hi\n</custom>');
});
test('assistant mode still strips Garyx internal notification fallback', () => {
  const o = prepareMessageMarkdown('<garyx_task_notification>hidden</garyx_task_notification>\n\nshown', { surfaceCustomXmlTags: false });
  assert.equal(o.trim(), 'shown');
});
test('R-final P1 unmatched backtick run stays linear & unchanged', () => {
  const i = 'x ' + '`'.repeat(40000);
  assert.equal(escapeNonHtmlTagsOutsideCode(i), i); // must not hang (was O(n^2))
});
test('R-final2 descending unmatched backtick runs stay linear & unchanged', () => {
  const bt = '`'; let s = '';
  for (let L = 1200; L >= 1; L--) s += bt.repeat(L) + 'x'; // distinct lengths, none match (~720KB)
  assert.equal(escapeNonHtmlTagsOutsideCode(s), s); // must not hang (was O(n^1.5))
});

// XML 折行:被显示的独占一行的标签各自成行(前后空行);行内标签不拆
test('LB standalone surfaced tag gets its own line (blank lines around)', () => {
  assert.equal(escapeNonHtmlTagsOutsideCode('p\n<note>\nq'), 'p\n\n&lt;note&gt;\n\nq');
});
test('LB consecutive standalone tags each on their own line', () => {
  const o = escapeNonHtmlTagsOutsideCode('<a1>\n<a2>');
  assert.match(o, /&lt;a1&gt;\n\n&lt;a2&gt;/);
});
test('LB inline tag in prose is NOT broken onto its own line', () => {
  assert.equal(escapeNonHtmlTagsOutsideCode('see <note> here'), 'see &lt;note&gt; here');
});
test('LB allowlisted standalone tag is not blank-wrapped', () => {
  assert.equal(escapeNonHtmlTagsOutsideCode('x\n<br>\ny'), 'x\n<br>\ny');
});

test('LB CRLF standalone surfaced tag also gets its own line', () => {
  assert.match(escapeNonHtmlTagsOutsideCode('p\r\n<note>\r\nq'), /&lt;note&gt;\n\n/);
});
test('P1 inline-code span must not swallow a block-boundary internal opener', () => {
  const bt = '`';
  const input = 'visible ' + bt + '\n<garyx_memory_context>secret ' + bt + '\n</garyx_memory_context>\nshown';
  const o = prepareMessageMarkdown(input);
  assert.doesNotMatch(o, /garyx_memory_context/);
  assert.doesNotMatch(o, /secret/);
  assert.match(o, /shown/);
});
test('P1b non-internal tag in multiline inline code stays verbatim (no over-bail)', () => {
  const bt = '`';
  const ss = 'a ' + bt + '\n<system_status>\n' + bt + ' b';
  assert.equal(escapeNonHtmlTagsOutsideCode(ss), ss);
  assert.equal(prepareMessageMarkdown(ss), ss);
  const dv = 'a ' + bt + '\n<garyx_models.v2>\n' + bt + ' b';
  assert.equal(escapeNonHtmlTagsOutsideCode(dv), dv);
});
test('P1b internal-opener crossing stays linear (precompute + binary search)', () => {
  const bt = '`'; let s = '';
  for (let L = 1; L <= 1200; L++) s += bt.repeat(L) + 'x';
  s += '\n<garyx_memory_context>secret</garyx_memory_context>\n';
  for (let L = 1; L <= 1200; L++) s += bt.repeat(L) + 'y';
  const o = prepareMessageMarkdown(s);
  assert.doesNotMatch(o, /secret/);
});
