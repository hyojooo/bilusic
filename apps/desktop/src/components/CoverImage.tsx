import { useEffect, useRef, useState } from 'react';

interface CoverImageProps {
  /** Cover URL. When missing or it fails to load, `placeholder` is shown. */
  src?: string | null;
  alt?: string;
  /** Extra classes on the `<img>` itself (e.g. `object-cover`). */
  className?: string;
  /** Classes on the wrapping container — aspect ratio, rounding, overflow. */
  wrapperClassName?: string;
  /** Fallback shown when `src` is empty or errors. */
  placeholder?: string;
}

/**
 * Cover image with a smooth fade-in.
 *
 *  • The wrapper sits on a neutral `bg-surface-2` so, while the bytes arrive,
 *    the card is a calm, consistent surface instead of a jarring empty box —
 *    this is what removes the "covers flicker / load one-by-one" feeling when
 *    returning to a cached page.
 *  • The `<img>` starts at `opacity-0` and transitions to `opacity-100` on
 *    `onLoad`, so a freshly-fetched cover eases in rather than popping.
 *  • If the image is already in the browser cache on mount (e.g. switching
 *    tabs back to a page whose data came from localStorage), we detect
 *    `img.complete` and show it instantly — no fade-from-blank double flash.
 *  • A failed/empty `src` falls back to the placeholder, which also fades in.
 */
export default function CoverImage({
  src,
  alt = '',
  className = '',
  wrapperClassName = '',
  placeholder = '/placeholder-cover.svg',
}: CoverImageProps) {
  const [loaded, setLoaded] = useState(false);
  const [errored, setErrored] = useState(false);
  const imgRef = useRef<HTMLImageElement>(null);

  const finalSrc = src && !errored ? src : placeholder;

  // Reset when the source changes (keep the fade honest for new covers).
  useEffect(() => {
    setLoaded(false);
    setErrored(false);
  }, [src]);

  // Browser-cached images are `complete` on mount — show them immediately
  // instead of fading up from blank.
  useEffect(() => {
    const el = imgRef.current;
    if (el && el.complete && el.naturalWidth > 0) {
      setLoaded(true);
    }
  }, [finalSrc]);

  return (
    <div className={`relative overflow-hidden bg-surface-2 ${wrapperClassName}`}>
      <img
        ref={imgRef}
        src={finalSrc}
        alt={alt}
        loading="lazy"
        decoding="async"
        onLoad={() => setLoaded(true)}
        onError={() => setErrored(true)}
        className={`h-full w-full transition-opacity duration-500 ease-out ${
          loaded ? 'opacity-100' : 'opacity-0'
        } ${className}`}
      />
    </div>
  );
}
