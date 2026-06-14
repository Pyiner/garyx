// Render-time preprocessing for chat message markdown.
//
// Garyx desktop renders messages with Streamdown, whose pipeline sanitizes
// against an allowlist (hast-util-sanitize defaultSchema) and silently drops
// any tag outside it. This module runs BEFORE Streamdown to strip Garyx's own
// injected internal tags (`garyx_*` family and `system_instructions`) and,
// when requested, surface remaining non-allowlisted tags as visible literal
// text, leaving allowlisted HTML and code (fences/inline) untouched.
//
// One left-to-right character scanner. Tags are parsed BEFORE inline code (so
// backticks/`>` inside attributes don't break tag detection); on a malformed
// tag the scan returns where it stopped so the caller advances past it (linear,
// no re-scan). An internal tag is only treated as an injected block at a LINE
// boundary (mid-line internal tags are prose → surfaced); its block is removed
// through the first literal `</name>`. Internal payloads are XML-escaped
// upstream by garyx-bridge, so they never contain a raw close marker — a
// first-match close search is correct. Display-only; agent-bound text is never
// touched. No whitespace normalization, so untouched text is byte-identical.

export const HTML_TAG_NAMES = new Set<string>([
  'a', 'b', 'blockquote', 'br', 'code', 'dd', 'del', 'details', 'div', 'dl',
  'dt', 'em', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'hr', 'i', 'img', 'input',
  'ins', 'kbd', 'li', 'ol', 'p', 'picture', 'pre', 'q', 'rp', 'rt', 'ruby',
  's', 'samp', 'section', 'source', 'span', 'strike', 'strong', 'sub',
  'summary', 'sup', 'table', 'tbody', 'td', 'tfoot', 'th', 'thead', 'tr',
  'tt', 'ul', 'var',
]);

function isInternalName(name: string): boolean {
  return name === 'system_instructions' || /^garyx_\w+$/.test(name);
}
// A tag name is validly terminated by whitespace, '>', EOF, or a self-closing
// '/' only when it is immediately the '/>' slash (so '<garyx_models/foo>' is NOT
// a clean internal name and gets surfaced, while '<br/>' stays allowlisted).
function isNameTerminator(text: string, j: number): boolean {
  if (j >= text.length) return true;
  const c = text[j];
  return /[\s>]/.test(c) || (c === '/' && (text[j + 1] === '>' || j + 1 >= text.length));
}
// At text[i]==='<': a malformed/incomplete internal OPENER — the `garyx_`
// family, or a (partial) prefix of `system_instructions`. Name read index-based.
function malformedInternalOpener(text: string, i: number): boolean {
  let j = i + 1;
  if (text[j] === '/') return false;
  const s = j;
  while (j < text.length && /[\w.:-]/.test(text[j])) j++;
  const partial = text.slice(s, j);
  const boundary = isNameTerminator(text, j); // same validity as nameOk
  if (!boundary) return false;
  if (/^garyx_\w*$/.test(partial)) return true; // garyx_ family, clean name only
  return partial.startsWith('system_') && 'system_instructions'.startsWith(partial);
}

type ParsedTag = { name: string; nameOk: boolean; closing: boolean; selfClosing: boolean; end: number };
// On success {tag, end:tag.end}; on failure {tag:null, end:<scan stop>} so the
// caller advances past the scanned span (keeps the whole transform linear).
type TagScan = { tag: ParsedTag | null; end: number };

function scanTag(text: string, i: number): TagScan {
  let j = i + 1;
  let closing = false;
  if (text[j] === '/') { closing = true; j++; }
  const nameStart = j;
  if (!/[A-Za-z_]/.test(text[j] || '')) return { tag: null, end: i + 1 };
  while (j < text.length && /[\w.:-]/.test(text[j])) j++; // names may contain '.'
  const name = text.slice(nameStart, j);
  // A real tag name is followed by whitespace, '/', '>', or EOF; otherwise it's
  // not a clean name (e.g. <garyx_models=foo>, <b=1>) → not internal/allowlist.
  const nameOk = isNameTerminator(text, j);
  while (j < text.length) {
    const c = text[j];
    if (c === '"' || c === "'") {
      // scan to the matching quote; an unquoted '<' first bounds the failure.
      let k = j + 1;
      while (k < text.length && text[k] !== c && text[k] !== '<') k++;
      if (text[k] === c) { j = k + 1; }
      else if (text[k] === '<') return { tag: null, end: k };
      else return { tag: null, end: text.length };
    } else if (c === '>') {
      return { tag: { name, nameOk, closing, selfClosing: text[j - 1] === '/', end: j + 1 }, end: j + 1 };
    } else if (c === '<') {
      return { tag: null, end: j };
    } else {
      j++;
    }
  }
  return { tag: null, end: text.length };
}

function fenceOpenerAt(text: string, i: number): { char: string; len: number; lineEnd: number } | null {
  let j = i, indent = 0;
  while (text[j] === ' ' && indent < 3) { j++; indent++; }
  const ch = text[j];
  if (ch !== '`' && ch !== '~') return null;
  let len = 0;
  while (text[j] === ch) { j++; len++; }
  if (len < 3) return null;
  let lineEnd = text.indexOf('\n', j);
  if (lineEnd === -1) lineEnd = text.length;
  if (ch === '`') { for (let p = j; p < lineEnd; p++) if (text[p] === '`') return null; }
  return { char: ch, len, lineEnd };
}
function fencedBlockEnd(text: string, opener: { char: string; len: number; lineEnd: number }): number {
  const n = text.length;
  let pos = opener.lineEnd;
  while (pos < n) {
    const lineStart = pos + 1;
    let j = lineStart, indent = 0;
    while (text[j] === ' ' && indent < 3) { j++; indent++; }
    let len = 0;
    while (text[j] === opener.char) { j++; len++; }
    if (len >= opener.len) {
      let k = j;
      while (text[k] === ' ' || text[k] === '\t') k++;
      if (text[k] === '\r') k++;
      if (k === n || text[k] === '\n') return k;
    }
    const nl = text.indexOf('\n', lineStart);
    if (nl === -1) return n;
    pos = nl;
  }
  return n;
}
// Precompute, in one linear pass, every backtick run and the next run of EQUAL
// length (CommonMark: a run of length N closes at the first later run of length
// N). Returns a lookup that, for a run starting at `i`, gives the index just
// past its closing run, or null. O(1) per lookup, so unmatched runs don't
// re-scan the suffix each time (which was superlinear). Behaviour is identical
// to a byte-scan-from-i for matching run starts.
function buildInlineCodeLookup(text: string): (i: number) => number | null {
  const pos: number[] = [];
  const len: number[] = [];
  const idxByPos = new Map<number, number>();
  for (let p = 0; p < text.length; ) {
    if (text[p] === '`') {
      let l = 0;
      while (text[p + l] === '`') l++;
      idxByPos.set(p, pos.length);
      pos.push(p);
      len.push(l);
      p += l;
    } else {
      p++;
    }
  }
  const next = new Array<number>(pos.length).fill(-1);
  const last = new Map<number, number>();
  for (let r = pos.length - 1; r >= 0; r--) {
    const L = len[r];
    next[r] = last.has(L) ? (last.get(L) as number) : -1;
    last.set(L, r);
  }
  return (i: number): number | null => {
    const r = idxByPos.get(i);
    if (r === undefined) return null;
    const c = next[r];
    return c === -1 ? null : pos[c] + len[c];
  };
}

// First literal `</name\s*>` at/after `from`, or -1. (Internal payloads are
// XML-escaped upstream, so they hold no raw close marker — see header.)
function findCloseTag(text: string, from: number, name: string): number {
  const re = new RegExp(`</${name}\\s*>`, 'g');
  re.lastIndex = from;
  const m = re.exec(text);
  return m ? m.index + m[0].length : -1;
}

function atBlockBoundary(text: string, i: number): boolean {
  let k = i - 1;
  while (k >= 0 && (text[k] === ' ' || text[k] === '\t')) k--;
  return k < 0 || text[k] === '\n';
}
// CommonMark autolink: <scheme:...> (scheme 2-32 chars) or <email>, no inner
// whitespace/'<'. Scanned char-by-char and bailed at the first delimiter so it
// stays linear. Returns the index just past '>' on match, else -1.
function autolinkEnd(text: string, i: number): number {
  let j = i + 1;
  const n = text.length;
  while (j < n) {
    const c = text[j];
    if (c === '>') break;
    if (c === '<' || c === ' ' || c === '\t' || c === '\n' || c === '\r') return -1;
    j++;
  }
  if (j >= n || text[j] !== '>') return -1;
  const inner = text.slice(i + 1, j);
  if (/^[A-Za-z][A-Za-z0-9+.-]{1,31}:[^\s<>]*$/.test(inner)) return j + 1;
  if (/^[^\s<>@]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/.test(inner)) return j + 1;
  return -1;
}

function escapeTag(raw: string): string {
  return raw.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

// Only whitespace between `end` and the next newline / EOF (tag alone on its line).
function restOfLineIsBlank(text: string, end: number): boolean {
  let k = end;
  while (text[k] === ' ' || text[k] === '\t') k++;
  if (text[k] === '\r') k++; // tolerate CRLF line endings
  return k >= text.length || text[k] === '\n';
}

function transform(text: string, opts: { strip: boolean; escape: boolean }): string {
  let out = '';
  let trailingNl = 0;       // count of '\n' currently at the end of `out`
  let pendingBlank = false; // a standalone surfaced tag asked for a blank line before the next content
  const appendRaw = (s: string) => {
    if (!s) return;
    out += s;
    let t = 0;
    while (t < s.length && s[s.length - 1 - t] === '\n') t++;
    trailingNl = t === s.length ? trailingNl + t : t;
  };
  const ensureBlank = () => { if (out.length > 0 && trailingNl < 2) appendRaw('\n'.repeat(2 - trailingNl)); };
  const emit = (s: string) => { if (!s) return; if (pendingBlank) { ensureBlank(); pendingBlank = false; } appendRaw(s); };

  let i = 0;
  const n = text.length;
  const inlineCodeEnd = buildInlineCodeLookup(text);
  // Precompute the positions of block-boundary internal openers the strip logic
  // would actually remove (using the SAME isInternalName / malformed rules, not a
  // loose prefix). A binary search then tells, in O(log n), whether a would-be
  // inline-code span crosses one — so internal stripping takes precedence over
  // inline-code skipping without re-scanning each span (keeps the pass linear).
  const internalOpeners: number[] = [];
  if (opts.strip) {
    for (let p = 0; p < n; p++) {
      if (p !== 0 && text[p - 1] !== '\n') continue;
      let q = p;
      while (text[q] === ' ' || text[q] === '\t') q++;
      if (text[q] !== '<') continue;
      const { tag } = scanTag(text, q);
      if ((tag && tag.nameOk && !tag.closing && isInternalName(tag.name)) ||
          (!tag && malformedInternalOpener(text, q))) {
        internalOpeners.push(q);
      }
    }
  }
  const crossesInternalOpener = (start: number, end: number): boolean => {
    let lo = 0, hi = internalOpeners.length;
    while (lo < hi) { const mid = (lo + hi) >> 1; if (internalOpeners[mid] < start) lo = mid + 1; else hi = mid; }
    return lo < internalOpeners.length && internalOpeners[lo] < end;
  };
  while (i < n) {
    const c = text[i];
    if (i === 0 || text[i - 1] === '\n') {
      const opener = fenceOpenerAt(text, i);
      if (opener) { const end = fencedBlockEnd(text, opener); emit(text.slice(i, end)); i = end; continue; }
    }
    if (c === '<') {
      const al = autolinkEnd(text, i);
      if (al !== -1) { emit(text.slice(i, al)); i = al; continue; } // CommonMark autolink: pass through
      const { tag, end } = scanTag(text, i);
      if (tag) {
        if (opts.strip && tag.nameOk && !tag.closing && isInternalName(tag.name)) {
          if (tag.selfClosing) { i = tag.end; continue; }     // self-closing: drop
          if (atBlockBoundary(text, i)) {                     // block-level => real injected block
            const close = findCloseTag(text, tag.end, tag.name);
            if (close !== -1) { i = close; continue; }
            i = n; continue;                                  // streaming unclosed -> EOF
          }
          // mid-line opener => prose; fall through to escape
        }
        const raw = text.slice(i, tag.end);
        const keep = opts.escape ? (tag.nameOk && HTML_TAG_NAMES.has(tag.name.toLowerCase())) : true;
        if (opts.escape && !keep) {
          // Surface a custom XML tag. If it stands alone on its source line, give
          // it its own rendered line (blank line before + after) so surfaced XML
          // reads as structured lines instead of soft-wrapping into one paragraph.
          if (atBlockBoundary(text, i) && restOfLineIsBlank(text, tag.end)) {
            if (pendingBlank) { ensureBlank(); pendingBlank = false; }
            ensureBlank();
            appendRaw(escapeTag(raw));
            pendingBlank = true;
            let k = tag.end;
            while (text[k] === ' ' || text[k] === '\t') k++;
            if (text[k] === '\r') k++; // tolerate CRLF
            if (text[k] === '\n') k++; // consume one trailing soft newline (blank already added)
            i = k;
            continue;
          }
          emit(escapeTag(raw));
        } else {
          emit(raw);
        }
        i = tag.end;
        continue;
      }
      // malformed/incomplete tag
      if (opts.strip && atBlockBoundary(text, i) && malformedInternalOpener(text, i)) { i = n; continue; }
      emit(text.slice(i, end)); // emit raw, advance to scan end (linear)
      i = end;
      continue;
    }
    if (c === '`') {
      const end = inlineCodeEnd(i);
      // Internal-block stripping takes precedence: don't let an inline-code span
      // swallow a block-boundary internal opener (would leak the hidden block).
      if (end !== null && !(opts.strip && crossesInternalOpener(i, end))) {
        emit(text.slice(i, end)); i = end; continue;
      }
      // Unclosed run, or a run that would cross an internal opener: the opening
      // backtick run is literal; skip past it so the scanner reaches the opener.
      let k = i;
      while (text[k] === '`') k++;
      emit(text.slice(i, k)); i = k; continue;
    }
    emit(c); i++;
  }
  return out;
}

export function stripGaryxInternalTags(text: string): string { return transform(text, { strip: true, escape: false }); }
export function escapeNonHtmlTagsOutsideCode(text: string): string { return transform(text, { strip: false, escape: true }); }
export function prepareMessageMarkdown(
  text: string,
  options: { surfaceCustomXmlTags?: boolean } = {},
): string {
  return transform(text, {
    strip: true,
    escape: options.surfaceCustomXmlTags !== false,
  });
}
