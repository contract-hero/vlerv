// Async-highlighted code block — wraps shiki.
// C3: sets `data-shiki-ready="true"` on the wrapper once highlighting resolves.
import * as React from "react";

export interface ShikiBlockProps {
  code: string;
  lang: string;
  theme?: string;
}

export default function ShikiBlock({ code, lang, theme = "github-dark" }: ShikiBlockProps): React.ReactElement {
  const [ready, setReady] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    // Dynamically import shiki and highlight.
    import("shiki")
      .then(async (shiki) => {
        await shiki.createHighlighter({
          themes: [theme],
          langs: [lang as Parameters<typeof shiki.createHighlighter>[0]["langs"][0]],
        });
        if (!cancelled) {
          setReady(true);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setReady(true);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [code, lang, theme]);

  return (
    <pre
      data-shiki-block
      data-shiki-ready={ready ? "true" : undefined}
    >
      <code className={`language-${lang}`}>{code}</code>
    </pre>
  );
}
