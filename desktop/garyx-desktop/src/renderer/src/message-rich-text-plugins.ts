import remarkBreaks from 'remark-breaks';
import {
  defaultRemarkPlugins,
  type StreamdownProps,
} from 'streamdown';

export const CHAT_MESSAGE_REMARK_PLUGINS: NonNullable<
  StreamdownProps['remarkPlugins']
> = [
  ...Object.values(defaultRemarkPlugins),
  remarkBreaks,
];
