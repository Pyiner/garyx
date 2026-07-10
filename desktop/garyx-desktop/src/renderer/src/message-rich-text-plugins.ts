import remarkBreaks from 'remark-breaks';
import {
  defaultRehypePlugins,
  defaultRemarkPlugins,
  type StreamdownProps,
} from 'streamdown';

import { rehypeLocalMessageImages } from './message-local-images.ts';

export const CHAT_MESSAGE_REHYPE_PLUGINS: NonNullable<
  StreamdownProps['rehypePlugins']
> = [
  ...Object.values(defaultRehypePlugins),
  rehypeLocalMessageImages,
];

export const CHAT_MESSAGE_REMARK_PLUGINS: NonNullable<
  StreamdownProps['remarkPlugins']
> = [
  ...Object.values(defaultRemarkPlugins),
  remarkBreaks,
];
