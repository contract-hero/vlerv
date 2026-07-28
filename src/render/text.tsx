// Text/code renderer — shiki highlighting for known extensions; raw for unknown.
import * as React from "react";
import ShikiBlock from "./shiki-block";

const SHIKI_EXTS: Record<string, string> = {
  ".ts": "typescript", ".tsx": "typescript", ".js": "javascript",
  ".jsx": "javascript", ".json": "json", ".rs": "rust", ".move": "rust",
  ".toml": "toml", ".yml": "yaml", ".yaml": "yaml", ".css": "css",
  ".html": "html", ".sh": "bash", ".py": "python", ".go": "go",
  ".md": "markdown", ".markdown": "markdown",
};

function langOf(path: string): string | null {
  const i = path.lastIndexOf(".");
  if (i < 0) return null;
  return SHIKI_EXTS[path.slice(i).toLowerCase()] ?? null;
}

export interface TextRendererProps {
  source: string;
  path: string;
}

export default function TextRenderer({ source, path }: TextRendererProps): React.ReactElement {
  const lang = langOf(path);
  if (lang) {
    return <ShikiBlock code={source} lang={lang} />;
  }
  return (
    <pre data-fallback="monospace" style={{ fontFamily: "var(--font-mono)" }}>
      <code>{source}</code>
    </pre>
  );
}
