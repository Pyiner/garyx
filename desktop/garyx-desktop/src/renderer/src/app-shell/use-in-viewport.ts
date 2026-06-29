import { useEffect, useState, type RefObject } from 'react';

/**
 * Report whether `ref` is in (or near) the viewport, so Capsule thumbnails only
 * mount/fetch their iframe when visible. Falls back to always-visible when
 * IntersectionObserver is unavailable.
 */
export function useInViewport<T extends Element>(
  ref: RefObject<T | null>,
  options: { rootMargin?: string } = {},
): boolean {
  const [visible, setVisible] = useState(false);
  const { rootMargin = '200px' } = options;
  useEffect(() => {
    const el = ref.current;
    if (!el || typeof IntersectionObserver === 'undefined') {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          setVisible(entry.isIntersecting);
        }
      },
      { rootMargin },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, [ref, rootMargin]);
  return visible;
}
