/**
 * The canonical wordmark, as the app ships it.
 *
 * Purpose: `apps/web/public/lockup-color.svg` and its dark counterpart are the
 * brand artwork. They are imported here rather than redrawn: the kit's
 * `Wordmark` is a component-sized approximation, and a second drawing of a logo
 * is a second logo.
 *
 * The two files differ only in the colour of the word, so the mode picks one
 * with CSS and the browser never downloads the one it will not show — `display:
 * none` also keeps the hidden copy out of the accessibility tree.
 */

import lockupDark from "../../../apps/web/public/lockup-color-on-dark.svg";
import lockupLight from "../../../apps/web/public/lockup-color.svg";

export function Brand({ className = "h-7" }: { className?: string }) {
  return (
    <>
      <img src={lockupLight} alt="Marula" className={`${className} w-auto dark:hidden`} />
      <img src={lockupDark} alt="Marula" className={`hidden w-auto dark:block ${className}`} />
    </>
  );
}
