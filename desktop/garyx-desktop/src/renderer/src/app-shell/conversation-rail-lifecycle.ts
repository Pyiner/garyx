export type ConversationRailIntent =
  | { kind: "closed" }
  | { kind: "recent" }
  | { kind: "bot"; groupId: string };

export function deferConversationRailUnmount(
  current: ConversationRailIntent,
  next: ConversationRailIntent,
): ConversationRailIntent {
  return next.kind === "closed" && current.kind !== "closed"
    ? current
    : next;
}

export function settleDeferredConversationRailUnmount(
  applied: ConversationRailIntent,
  desired: ConversationRailIntent,
  presented: boolean,
): ConversationRailIntent {
  if (
    presented ||
    desired.kind !== "closed" ||
    applied.kind === "closed"
  ) {
    return applied;
  }
  return { kind: "closed" };
}
