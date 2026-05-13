// File-type icon mapping via lucide-react.
import * as React from "react";
import {
  File,
  FileText,
  FileCode,
  FileJson,
  FileType,
  FileImage,
  FileArchive,
  FileVideo,
  FileAudio,
  FileLock,
  FileCog,
  Folder,
  FolderOpen,
  Settings,
  type LucideIcon,
} from "lucide-react";

const EXT_MAP: Record<string, { icon: LucideIcon; color: string }> = {
  // Markup / docs
  ".md": { icon: FileText, color: "#80cbc4" },
  ".markdown": { icon: FileText, color: "#80cbc4" },
  ".txt": { icon: FileText, color: "#bbbbbb" },
  ".rst": { icon: FileText, color: "#bbbbbb" },
  ".pdf": { icon: FileText, color: "#ef5350" },

  // Web
  ".html": { icon: FileCode, color: "#e34c26" },
  ".htm": { icon: FileCode, color: "#e34c26" },
  ".css": { icon: FileCode, color: "#42a5f5" },
  ".scss": { icon: FileCode, color: "#cd6799" },
  ".sass": { icon: FileCode, color: "#cd6799" },

  // JS/TS
  ".ts": { icon: FileCode, color: "#3178c6" },
  ".tsx": { icon: FileCode, color: "#3178c6" },
  ".js": { icon: FileCode, color: "#f7df1e" },
  ".jsx": { icon: FileCode, color: "#f7df1e" },
  ".mjs": { icon: FileCode, color: "#f7df1e" },
  ".cjs": { icon: FileCode, color: "#f7df1e" },

  // Systems languages
  ".rs": { icon: FileCode, color: "#dea584" },
  ".move": { icon: FileCode, color: "#6fc7e0" },
  ".go": { icon: FileCode, color: "#00add8" },
  ".py": { icon: FileCode, color: "#ffd43b" },
  ".rb": { icon: FileCode, color: "#cc342d" },
  ".swift": { icon: FileCode, color: "#fa7343" },
  ".java": { icon: FileCode, color: "#f89820" },
  ".kt": { icon: FileCode, color: "#a97bff" },
  ".c": { icon: FileCode, color: "#a8b9cc" },
  ".cpp": { icon: FileCode, color: "#00599c" },
  ".h": { icon: FileCode, color: "#a8b9cc" },
  ".sh": { icon: FileCode, color: "#89e051" },
  ".bash": { icon: FileCode, color: "#89e051" },
  ".zsh": { icon: FileCode, color: "#89e051" },
  ".sql": { icon: FileCode, color: "#e38c00" },

  // Data / config
  ".json": { icon: FileJson, color: "#ffca28" },
  ".jsonc": { icon: FileJson, color: "#ffca28" },
  ".yml": { icon: FileCog, color: "#cb171e" },
  ".yaml": { icon: FileCog, color: "#cb171e" },
  ".toml": { icon: FileCog, color: "#9c4221" },
  ".ini": { icon: FileCog, color: "#9c4221" },
  ".env": { icon: FileCog, color: "#a0a0a0" },
  ".conf": { icon: FileCog, color: "#9c4221" },
  ".xml": { icon: FileCode, color: "#e37933" },

  // Images
  ".png": { icon: FileImage, color: "#ce9178" },
  ".jpg": { icon: FileImage, color: "#ce9178" },
  ".jpeg": { icon: FileImage, color: "#ce9178" },
  ".gif": { icon: FileImage, color: "#ce9178" },
  ".webp": { icon: FileImage, color: "#ce9178" },
  ".bmp": { icon: FileImage, color: "#ce9178" },
  ".svg": { icon: FileImage, color: "#ffb86c" },
  ".ico": { icon: FileImage, color: "#ce9178" },

  // Media
  ".mp4": { icon: FileVideo, color: "#bd93f9" },
  ".mov": { icon: FileVideo, color: "#bd93f9" },
  ".webm": { icon: FileVideo, color: "#bd93f9" },
  ".mp3": { icon: FileAudio, color: "#f1fa8c" },
  ".wav": { icon: FileAudio, color: "#f1fa8c" },
  ".flac": { icon: FileAudio, color: "#f1fa8c" },

  // Archives
  ".zip": { icon: FileArchive, color: "#ffb86c" },
  ".tar": { icon: FileArchive, color: "#ffb86c" },
  ".gz": { icon: FileArchive, color: "#ffb86c" },
  ".tgz": { icon: FileArchive, color: "#ffb86c" },
  ".bz2": { icon: FileArchive, color: "#ffb86c" },
  ".7z": { icon: FileArchive, color: "#ffb86c" },

  // Fonts
  ".ttf": { icon: FileType, color: "#bd93f9" },
  ".otf": { icon: FileType, color: "#bd93f9" },
  ".woff": { icon: FileType, color: "#bd93f9" },
  ".woff2": { icon: FileType, color: "#bd93f9" },

  // Locks
  ".lock": { icon: FileLock, color: "#a0a0a0" },
  ".lockb": { icon: FileLock, color: "#a0a0a0" },
};

const BASENAME_MAP: Record<string, { icon: LucideIcon; color: string }> = {
  "package.json": { icon: FileJson, color: "#cb3837" },
  "package-lock.json": { icon: FileLock, color: "#cb3837" },
  "pnpm-lock.yaml": { icon: FileLock, color: "#f69220" },
  "yarn.lock": { icon: FileLock, color: "#2188b6" },
  "Cargo.toml": { icon: Settings, color: "#dea584" },
  "Cargo.lock": { icon: FileLock, color: "#dea584" },
  "tsconfig.json": { icon: Settings, color: "#3178c6" },
  "tsconfig.node.json": { icon: Settings, color: "#3178c6" },
  "vite.config.ts": { icon: Settings, color: "#bd34fe" },
  "tauri.conf.json": { icon: Settings, color: "#ffc131" },
  ".gitignore": { icon: FileCog, color: "#f1502f" },
  ".gitattributes": { icon: FileCog, color: "#f1502f" },
  "Move.toml": { icon: Settings, color: "#6fc7e0" },
  "Move.lock": { icon: FileLock, color: "#6fc7e0" },
  "Dockerfile": { icon: Settings, color: "#2496ed" },
  "Makefile": { icon: Settings, color: "#a0a0a0" },
  "README.md": { icon: FileText, color: "#42a5f5" },
  "LICENSE": { icon: FileText, color: "#a0a0a0" },
};

function lower(s: string): string {
  return s.toLowerCase();
}

export function iconForFile(name: string): { Icon: LucideIcon; color: string } {
  // Exact basename match wins
  const direct = BASENAME_MAP[name] || BASENAME_MAP[lower(name)];
  if (direct) return { Icon: direct.icon, color: direct.color };

  // Extension match
  const dot = name.lastIndexOf(".");
  if (dot > 0) {
    const ext = lower(name.slice(dot));
    const hit = EXT_MAP[ext];
    if (hit) return { Icon: hit.icon, color: hit.color };
  }

  return { Icon: File, color: "#9aa0a6" };
}

export function FileGlyph({ name, size = 17 }: { name: string; size?: number }): React.ReactElement {
  const { Icon, color } = iconForFile(name);
  return <Icon size={size} color={color} strokeWidth={1.75} />;
}

export function FolderGlyph({
  open,
  size = 17,
}: {
  open: boolean;
  size?: number;
}): React.ReactElement {
  const Icon = open ? FolderOpen : Folder;
  return <Icon size={size} color="#7bafd4" strokeWidth={1.75} />;
}
