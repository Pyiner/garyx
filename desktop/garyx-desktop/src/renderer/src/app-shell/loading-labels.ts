export const RUN_LOADING_LABEL = "Thinking";
const LEGACY_RUN_LOADING_PLACEHOLDER_TEXTS = new Set([
  "Garyx is working through the run…",
  "Garyx is working through the run...",
]);

export function isRunLoadingPlaceholderText(value: string | undefined): boolean {
  return LEGACY_RUN_LOADING_PLACEHOLDER_TEXTS.has(value?.trim() || "");
}

export function isRunLoadingPlaceholderMessage(message: {
  role?: string;
  text?: string;
}): boolean {
  return message.role === "assistant" && isRunLoadingPlaceholderText(message.text);
}
