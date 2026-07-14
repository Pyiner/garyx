import { useId } from 'react';
import { Star } from 'lucide-react';

export function CapsuleFavoriteStar({
  favorited,
  size = 15,
}: {
  favorited: boolean;
  size?: number;
}) {
  const gradientId = `capsule-favorite-${useId().replace(/:/g, '')}`;

  return (
    <Star
      aria-hidden
      className={`capsule-favorite-star${favorited ? ' is-favorited' : ''}`}
      fill={favorited ? `url(#${gradientId})` : 'none'}
      size={size}
      stroke={favorited ? 'var(--color-capsule-favorite-stroke)' : 'currentColor'}
    >
      {favorited ? (
        <defs>
          <linearGradient id={gradientId} x1="0" x2="0" y1="0" y2="1">
            <stop offset="0%" stopColor="var(--color-capsule-favorite-gold-top)" />
            <stop offset="100%" stopColor="var(--color-capsule-favorite-gold-bottom)" />
          </linearGradient>
        </defs>
      ) : null}
    </Star>
  );
}
