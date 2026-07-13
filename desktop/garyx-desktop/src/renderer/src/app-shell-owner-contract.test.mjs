import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const rendererDir = path.dirname(fileURLToPath(import.meta.url));
const stylesDir = path.join(rendererDir, "styles");
const read = (relativePath) =>
  readFileSync(path.join(rendererDir, relativePath), "utf8");
const stripComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, "");

const escapedOwnerPatterns = [
  ["app-shell root", /(?:^|[},])\s*\.app-shell(?=[\s.#:\[>])/m],
  ["main thread tracks", /(?:^|[},])\s*\.thread-layout(?=[\s.#:\[>])/m],
  ["right conversation tracks", /(?:^|[},])\s*\.conversation\.with-side-tools\b/m],
  ["conversation root", /(?:^|})\s*\.conversation\s*\{/m],
  ["shell resizers", /(?:^|[},])\s*\.(?:sidebar-resizer|side-tools-resizer)\b/m],
  ["sidebar toggle", /(?:^|[},])\s*\.sidebar-collapse-toggle\b/m],
  ["L1 root", /\.left-rail(?=\s*\{|::(?:before|after))/m],
  ["L2 root", /(?:^|[},])\s*\.bot-conversation-(?:rail|header)(?=\s*(?:\{|::))/m],
  ["static titlebar", /(?:^|[},])\s*\.settings-window-toolbar\s*\{/m],
];

test("app-shell owner is imported once and horizontal selectors cannot escape", () => {
  const entryCss = read("styles.css");
  assert.equal(
    entryCss.match(/@import "\.\/styles\/app-shell\.css";/g)?.length,
    1,
    "the renderer entrypoint must import the app-shell owner exactly once",
  );

  const offenders = [];
  for (const fileName of readdirSync(stylesDir)) {
    if (!fileName.endsWith(".css") || fileName === "app-shell.css") {
      continue;
    }
    const css = stripComments(read(`styles/${fileName}`));
    for (const [label, pattern] of escapedOwnerPatterns) {
      if (pattern.test(css)) {
        offenders.push(`${fileName}: ${label}`);
      }
    }
  }
  assert.deepEqual(
    offenders,
    [],
    `horizontal shell selectors escaped app-shell.css: ${offenders.join(", ")}`,
  );
});

test("owned horizontal tracks combine frame-owned fixed tracks with one CSS remainder", () => {
  const ownerCss = stripComments(read("styles/app-shell.css"));
  assert.match(
    ownerCss,
    /\.conversation\s*\{[^}]*grid-column:\s*-2\s*\/\s*-1;/s,
    "the main conversation must stay in the final shell column when rail content is transiently absent",
  );
  const columnValues = Array.from(
    ownerCss.matchAll(/grid-template-columns\s*:\s*([^;]+);/g),
    (match) => match[1].replace(/\s+/g, " ").trim(),
  );
  assert.equal(columnValues.length, 5, "every shell column recipe is explicit");
  for (const value of columnValues) {
    assert.equal(
      value.match(/minmax\(0, 1fr\)/g)?.length,
      1,
      `${value}: exactly one mechanical main-track remainder`,
    );
    assert.doesNotMatch(value, /calc\(|%/);
    const fixedTracks = value.replace("minmax(0, 1fr)", "").trim();
    if (fixedTracks) {
      assert.equal(
        fixedTracks
          .split(/\s+/)
          .every((track) => /^var\(--gx-[a-z-]+\)$/.test(track)),
        true,
        `${value}: every non-remainder track comes from the frame`,
      );
    }
  }
  assert.doesNotMatch(ownerCss, /@media\b|@container\b/);

  for (const attribute of [
    "data-sidebar-state",
    "data-conversation-rail-state",
    "data-side-tools-state",
    "data-task-tree-presentation",
  ]) {
    assert.match(ownerCss, new RegExp(attribute));
  }

  const allStyles = readdirSync(stylesDir)
    .filter((fileName) => fileName.endsWith(".css"))
    .map((fileName) => read(`styles/${fileName}`))
    .join("\n");
  assert.doesNotMatch(
    allStyles,
    /--(?:spacing-token-sidebar|spacing-token-rail|app-sidebar-width|side-tools-panel-width|side-tools-resizer-width|thread-log-panel-width|thread-log-resizer-width)\b/,
  );
});

test("task-tree policy consumes the frame while non-horizontal observers remain", () => {
  const popover = read("app-shell/components/ThreadTaskTreePopover.tsx");
  const threadPage = read("app-shell/components/ThreadPage.tsx");
  const controller = read("app-shell/useLayoutResizeController.ts");
  const appShell = read("app-shell/AppShell.tsx");

  assert.match(popover, /taskTreeDocked: boolean/);
  assert.match(popover, /triggerHost && !taskTreeDocked/);
  assert.doesNotMatch(popover, /ResizeObserver|getBoundingClientRect|isDockedTaskTree/);
  assert.match(threadPage, /taskTreeDocked=\{taskTreeDocked\}/);
  assert.match(controller, /taskTreeDocked: frame\.presentation\.taskTreeDocked/);
  assert.match(appShell, /taskTreeDocked=\{embedded \? false : taskTreeDocked\}/);

  for (const relativePath of [
    "BrowserPage.tsx",
    "app-shell/components/SideTerminalTool.tsx",
    "app-shell/components/SideToolsPanel.tsx",
    "app-shell/components/ThreadPage.tsx",
  ]) {
    assert.match(
      read(relativePath),
      /ResizeObserver/,
      `${relativePath} keeps its non-horizontal observer`,
    );
  }
});

test("drag recipe stays owned and the painted no-drag carveout is last", () => {
  const ownerCss = read("styles/app-shell.css");
  assert.match(ownerCss, /-webkit-app-region: drag/);
  assert.match(ownerCss, /-webkit-app-region: no-drag/);

  for (const fileName of [
    "gateway-setup.css",
    "sidebar.css",
    "workspace-rails.css",
    "conversation.css",
    "thread-logs.css",
    "side-tools.css",
  ]) {
    assert.doesNotMatch(
      stripComments(read(`styles/${fileName}`)),
      /-webkit-app-region/,
      `${fileName} must not own shell drag regions`,
    );
  }
  assert.match(read("styles/browser.css"), /-webkit-app-region: drag/);

  const source = read("app-shell/AppShell.tsx");
  const carveoutClass =
    'className="sidebar-collapse-toggle sidebar-collapse-toggle-carveout"';
  assert.equal(source.split(carveoutClass).length - 1, 1);
  const carveoutStart = source.lastIndexOf("<div", source.indexOf(carveoutClass));
  const carveoutTail = source.slice(carveoutStart);
  assert.match(carveoutTail, /aria-hidden="true"/);
  assert.match(carveoutTail, /onClick=\{toggleSidebarCollapsed\}/);
  const carveoutClose = carveoutTail.indexOf("</div>");
  assert.ok(carveoutClose > 0);
  assert.match(
    carveoutTail.slice(carveoutClose + "</div>".length),
    /^\s*<\/div>,\s*\);\s*\}\s*$/,
    "the unconditional painted carveout must remain the final app-shell child",
  );
});
