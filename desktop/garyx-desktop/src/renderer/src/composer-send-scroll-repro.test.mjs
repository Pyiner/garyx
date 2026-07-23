import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import test from "node:test";

const moduleStub = (source) =>
  `data:text/javascript,${encodeURIComponent(source)}`;

const reactStub = moduleStub(`
  export const useContext = () => null;
  export const useEffect = (effect, deps) =>
    globalThis.__composerSendHookRuntime.useEffect(effect, deps);
  export const useLayoutEffect = (effect, deps) =>
    globalThis.__composerSendHookRuntime.useLayoutEffect(effect, deps);
  export const useRef = (initialValue) =>
    globalThis.__composerSendHookRuntime.useRef(initialValue);
`);
const messageScrollerStub = moduleStub(`
  export const useMessageScroller = () =>
    globalThis.__composerSendHookRuntime.useMessageScroller();
`);
const gatewayMirrorReactStub = moduleStub(
  "export const GatewayMirrorContext = {};",
);
const transcriptMaterializeStub = moduleStub(`
  export const messagesNearEarlierUserTurnBoundary = () => false;
`);

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === "react") {
      return { shortCircuit: true, url: reactStub };
    }
    if (specifier === "@/components/ui/message-scroller") {
      return { shortCircuit: true, url: messageScrollerStub };
    }
    if (
      specifier === "../../gateway-mirror/react" &&
      context.parentURL?.endsWith("/thread-transcript-scroll.ts")
    ) {
      return { shortCircuit: true, url: gatewayMirrorReactStub };
    }
    if (
      specifier === "../../gateway-mirror/transcript-materialize" &&
      context.parentURL?.endsWith("/thread-transcript-scroll.ts")
    ) {
      return { shortCircuit: true, url: transcriptMaterializeStub };
    }
    if (
      specifier === "./transcript-scroll-anchor" &&
      context.parentURL?.endsWith("/thread-transcript-scroll.ts")
    ) {
      return nextResolve(`${specifier}.ts`, context);
    }
    return nextResolve(specifier, context);
  },
});

const {
  TranscriptScrollBridge,
  useTailThinkingScrollStability,
} = await import("./app-shell/components/thread-transcript-scroll.ts");
const { deriveThreadActivityModel } = await import(
  "./app-shell/thread-activity.ts"
);
const { buildThreadViewRowsWithLocalUsers } = await import(
  "./render-view-model.ts"
);

class HookInstance {
  constructor() {
    this.cursor = 0;
    this.dependencies = [];
    this.layoutEffects = [];
    this.passiveEffects = [];
    this.refs = [];
  }

  render(renderHook) {
    this.cursor = 0;
    this.layoutEffects = [];
    this.passiveEffects = [];
    globalThis.__composerSendHookRuntime = this;
    renderHook();
  }

  useRef(initialValue) {
    const index = this.cursor++;
    if (!this.refs[index]) {
      this.refs[index] = { current: initialValue };
    }
    return this.refs[index];
  }

  useLayoutEffect(effect, dependencies) {
    this.#registerEffect(this.layoutEffects, effect, dependencies);
  }

  useEffect(effect, dependencies) {
    this.#registerEffect(this.passiveEffects, effect, dependencies);
  }

  useMessageScroller() {
    return this.messageScroller;
  }

  runLayoutEffects() {
    globalThis.__composerSendHookRuntime = this;
    for (const effect of this.layoutEffects) {
      effect();
    }
  }

  runPassiveEffects() {
    globalThis.__composerSendHookRuntime = this;
    for (const effect of this.passiveEffects) {
      effect();
    }
  }

  #registerEffect(queue, effect, dependencies) {
    const index = this.cursor++;
    const previous = this.dependencies[index];
    const changed =
      !previous ||
      previous.length !== dependencies.length ||
      dependencies.some((dependency, dependencyIndex) => {
        return !Object.is(dependency, previous[dependencyIndex]);
      });
    this.dependencies[index] = dependencies;
    if (changed) {
      queue.push(effect);
    }
  }
}

class FakeHTMLElement {
  constructor(rect = {}) {
    this.hidden = false;
    this.isConnected = true;
    this.previousElementSibling = null;
    this.rect = rect;
  }

  getBoundingClientRect() {
    return {
      bottom: this.rect.bottom ?? 0,
      height: this.rect.height ?? 0,
      top: this.rect.top ?? 0,
    };
  }
}

globalThis.HTMLElement = FakeHTMLElement;
globalThis.getComputedStyle = () => ({ rowGap: "14px" });

function styleDeclaration() {
  const values = new Map();
  return {
    getPropertyValue: (property) => values.get(property) ?? "",
    removeProperty: (property) => values.delete(property),
    setProperty: (property, value) => values.set(property, value),
  };
}

function renderStateFixture() {
  return {
    based_on_seq: 2,
    rows: [
      {
        kind: "user_turn",
        id: "user_turn:seq:1",
        user: { id: "seq:1", seq: 1, role: "user" },
        activity: [
          {
            kind: "assistant_reply",
            message: { id: "seq:2", seq: 2, role: "assistant" },
          },
        ],
        capsule_cards: [],
        started_at: null,
        finished_at: null,
      },
    ],
    tailActivity: "thinking",
    activeToolGroupId: null,
    progress_locus: "tail",
    filtered_placeholders: [],
  };
}

test("composer send keeps the optimistic user row and thinking tail above the composer", () => {
  const threadId = "thread::scroll-repro";
  const committedMessages = [
    {
      id: "seq:1",
      seq: 1,
      role: "user",
      text: "Earlier question",
      localState: "remote_final",
    },
    {
      id: "seq:2",
      seq: 2,
      role: "assistant",
      text: "Earlier answer",
      localState: "remote_final",
    },
  ];
  const optimisticUser = {
    id: "origin:intent-repro",
    role: "user",
    text: "Run the requested follow-up.",
    timestamp: "2026-07-23T00:00:00.000Z",
    intentId: "intent-repro",
    localState: "optimistic",
  };
  const runningRenderState = renderStateFixture();
  const activeMessages = [...committedMessages, optimisticUser];
  const rows = buildThreadViewRowsWithLocalUsers(
    runningRenderState,
    new Map(committedMessages.map((message) => [message.seq, message])),
    activeMessages,
  );
  const activity = deriveThreadActivityModel({
    messages: activeMessages,
    runtimeBusy: true,
    pendingAckIntentCount: 0,
    remoteAwaitingAckInputCount: 0,
    pendingHistoryIntent: true,
    renderTailActivity: runningRenderState.tailActivity,
    renderActiveToolGroupId: runningRenderState.activeToolGroupId,
  });

  assert.equal(rows.at(-1)?.key, `user-turn:${optimisticUser.id}`);
  assert.equal(runningRenderState.tailActivity, "thinking");
  assert.equal(activity.showPendingAckLoading, true);

  const clientHeight = 600;
  const beforeScrollTop = 3_000;
  const composerScrollClip = 72;
  const composerMessageClearance = 56;
  const rowGap = 14;
  const thinkingHeight = 24;
  const optimisticBubbleHeight = 80;
  const tailReserve = thinkingHeight + rowGap;
  const runningBottomPadding =
    composerScrollClip + composerMessageClearance - tailReserve;
  const optimisticRowExtent = optimisticBubbleHeight + rowGap;
  const latestScrollTop = beforeScrollTop + optimisticRowExtent;

  const anchorDocumentTop = beforeScrollTop + 34;
  const anchor = new FakeHTMLElement();
  const priorUser = new FakeHTMLElement();
  const optimisticRow = new FakeHTMLElement();
  const thinkingRow = new FakeHTMLElement({ height: thinkingHeight });
  thinkingRow.previousElementSibling = optimisticRow;
  const content = new FakeHTMLElement();
  let showTailThinking = false;
  let viewport;
  anchor.getBoundingClientRect = () => {
    const top = anchorDocumentTop - viewport.scrollTop;
    return { bottom: top + 66, height: 66, top };
  };
  content.querySelectorAll = (selector) =>
    selector === '[data-slot="message-scroller-item"]'
      ? [anchor, priorUser]
      : [];
  content.querySelector = (selector) =>
    selector === '[data-tail-thinking-row="true"]' && showTailThinking
      ? thinkingRow
      : null;
  viewport = {
    clientHeight,
    contains: (candidate) => candidate === anchor,
    getBoundingClientRect: () => ({
      bottom: clientHeight,
      height: clientHeight,
      top: 0,
    }),
    querySelector: (selector) =>
      selector === '[data-slot="message-scroller-content"]' ? content : null,
    scrollHeight: beforeScrollTop + clientHeight,
    scrollTo: ({ top }) => {
      viewport.scrollTop = top;
    },
    scrollTop: beforeScrollTop,
    style: styleDeclaration(),
  };
  const messagesRef = { current: viewport };
  const scrollIntent = {
    pendingThreadBottomSnapRef: { current: null },
    forceMessagesBottomSnapRef: { current: false },
    shouldStickMessagesToBottomRef: { current: true },
    pendingMessagesPrependAnchorRef: { current: null },
    lastRenderedMessageThreadRef: { current: null },
    lastRenderedMessageCountRef: { current: 0 },
    lastRenderedMessageTailSignatureRef: { current: "0" },
    selectedThreadIdRef: { current: threadId },
  };

  const tailHook = new HookInstance();
  const bridgeHook = new HookInstance();
  bridgeHook.messageScroller = {
    scrollToEnd: () => {
      viewport.scrollTop = viewport.scrollHeight - viewport.clientHeight;
    },
  };

  tailHook.render(() =>
    useTailThinkingScrollStability({
      messagesRef,
      scopeKey: threadId,
      showTailThinking: false,
    }),
  );
  bridgeHook.render(() =>
    TranscriptScrollBridge({
      activeMessages: committedMessages,
      activeThreadMessageKey: threadId,
      historyLoading: false,
      scrollIntent,
    }),
  );
  // React commits descendant layout effects before the parent ThreadPage hook.
  bridgeHook.runLayoutEffects();
  tailHook.runLayoutEffects();
  tailHook.runPassiveEffects();

  showTailThinking = true;
  viewport.scrollHeight = latestScrollTop + clientHeight;
  scrollIntent.pendingThreadBottomSnapRef.current = threadId;
  scrollIntent.forceMessagesBottomSnapRef.current = true;
  scrollIntent.shouldStickMessagesToBottomRef.current = true;

  tailHook.render(() =>
    useTailThinkingScrollStability({
      messagesRef,
      scopeKey: threadId,
      showTailThinking: true,
    }),
  );
  bridgeHook.render(() =>
    TranscriptScrollBridge({
      activeMessages,
      activeThreadMessageKey: threadId,
      historyLoading: false,
      scrollIntent,
    }),
  );
  bridgeHook.runLayoutEffects();
  assert.equal(
    viewport.scrollTop,
    latestScrollTop,
    "the forced dispatch snap reaches the latest content first",
  );
  assert.equal(scrollIntent.pendingThreadBottomSnapRef.current, null);
  assert.equal(scrollIntent.forceMessagesBottomSnapRef.current, false);
  tailHook.runLayoutEffects();

  const bottomDistance =
    viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
  const tailBottomClearance = runningBottomPadding - bottomDistance;
  const userBubbleBottomClearance = tailBottomClearance + tailReserve;
  assert.deepEqual(
    {
      scrollTop: viewport.scrollTop,
      bottomDistance,
      tailBottomClearance,
      userBubbleBottomClearance,
      tailVisibleAboveComposer: tailBottomClearance >= composerScrollClip,
      userBubbleClearsComposer:
        userBubbleBottomClearance >= composerScrollClip,
    },
    {
      scrollTop: latestScrollTop,
      bottomDistance: 0,
      tailBottomClearance: runningBottomPadding,
      userBubbleBottomClearance: runningBottomPadding + tailReserve,
      tailVisibleAboveComposer: true,
      userBubbleClearsComposer: true,
    },
    "the parent tail anchor restoration must not undo a forced composer-send snap",
  );
});
