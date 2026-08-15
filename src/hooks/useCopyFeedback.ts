// useCopyFeedback — clipboard write with a transient "copied" flag, shared
// by every copy affordance (Toolbar's copy-path, Beam's copy-link) so the
// timer/cleanup mechanics exist once.
import * as React from "react";

const RESET_MS = 1200;

export function useCopyFeedback(): { copied: boolean; copy: (text: string) => void } {
  const [copied, setCopied] = React.useState(false);
  const resetTimer = React.useRef<number | null>(null);

  React.useEffect(() => {
    return () => {
      if (resetTimer.current) window.clearTimeout(resetTimer.current);
    };
  }, []);

  const copy = React.useCallback((text: string) => {
    void navigator.clipboard
      ?.writeText(text)
      .then(() => {
        setCopied(true);
        if (resetTimer.current) window.clearTimeout(resetTimer.current);
        resetTimer.current = window.setTimeout(() => setCopied(false), RESET_MS);
      })
      .catch(() => {
        // Clipboard unavailable — fail silently, matching existing copy
        // affordances.
      });
  }, []);

  return { copied, copy };
}
