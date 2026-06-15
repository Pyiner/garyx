export interface ParsedTaskNotification {
  event: string;
  status: string;
  taskId: string;
  title: string;
  finalMessage: string;
  detailCommand: string;
  reviewCommands: string[];
}

function decodeXmlAttribute(value: string): string {
  return value
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&amp;/g, '&');
}

function parseAttributes(raw: string): Record<string, string> {
  const attrs: Record<string, string> = {};
  for (const match of raw.matchAll(/([\w:-]+)\s*=\s*"([^"]*)"/g)) {
    const [, key, value] = match;
    if (key) {
      attrs[key] = decodeXmlAttribute(value || '');
    }
  }
  return attrs;
}

function stripOuterTaskNotification(text: string): {
  attrs: Record<string, string>;
  body: string;
} | null {
  const open = /^\s*<garyx_task_notification\b([^>]*)>\s*/.exec(text);
  if (!open) {
    return null;
  }
  const closeTag = '</garyx_task_notification>';
  const closeIndex = text.lastIndexOf(closeTag);
  if (closeIndex < open[0].length) {
    return null;
  }
  return {
    attrs: parseAttributes(open[1] || ''),
    body: text.slice(open[0].length, closeIndex).trim(),
  };
}

export function parseTaskNotificationText(
  text: string,
): ParsedTaskNotification | null {
  const parsed = stripOuterTaskNotification(text);
  if (!parsed) {
    return null;
  }

  const lines = parsed.body.split(/\r?\n/);
  const firstLine = lines.find((line) => line.trim())?.trim() || '';
  const firstLineMatch = /^Task\s+(.+?)\s+is ready for review:\s*(.*)$/.exec(firstLine);
  const taskId = parsed.attrs.task_id || firstLineMatch?.[1]?.trim() || '';
  const title = firstLineMatch?.[2]?.trim() || taskId || 'Task ready for review';

  const viewIndex = parsed.body.search(/\r?\nView details:/);
  const messageEnd = viewIndex >= 0 ? viewIndex : parsed.body.length;
  const afterFirstLine = parsed.body.slice(firstLine.length).trim();
  const finalMessage = parsed.body
    .slice(firstLine.length, messageEnd)
    .trim()
    || afterFirstLine
    || 'The task is ready for review.';

  const commandLines = lines
    .map((line) => line.trim())
    .filter((line) => line.startsWith('garyx task '));
  const detailCommand =
    commandLines.find((line) => /^garyx task get\b/.test(line)) ||
    (taskId ? `garyx task get ${taskId}` : '');
  const reviewCommands = commandLines.filter((line) =>
    /^garyx task update\b/.test(line),
  );

  return {
    event: parsed.attrs.event || 'ready_for_review',
    status: parsed.attrs.status || 'in_review',
    taskId,
    title,
    finalMessage,
    detailCommand,
    reviewCommands,
  };
}
