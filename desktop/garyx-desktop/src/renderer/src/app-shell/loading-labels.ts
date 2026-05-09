export const RUN_LOADING_LABEL = "Garyx is working through the run…";
const RUN_LOADING_LABEL_ALIASES = new Set([
  RUN_LOADING_LABEL,
  "Garyx is working through the run...",
]);

export function isRunLoadingPlaceholderText(value: string | undefined): boolean {
  return RUN_LOADING_LABEL_ALIASES.has(value?.trim() || "");
}

export function isRunLoadingPlaceholderMessage(message: {
  role?: string;
  text?: string;
}): boolean {
  return message.role === "assistant" && isRunLoadingPlaceholderText(message.text);
}
