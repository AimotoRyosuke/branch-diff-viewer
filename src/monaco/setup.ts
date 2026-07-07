// Monaco Editor bootstrap — fully offline / locally bundled.
//
// This module has import-time side effects and MUST be imported once before any
// Monaco editor is created (see main.tsx):
//   1. Wires self.MonacoEnvironment.getWorker to a Vite-bundled local worker.
//      No CDN getWorkerUrl — the worker ships inside the app bundle (DESIGN.md 2.1).
//   2. Defines the "bdv-light" / "bdv-dark" themes derived from docs/design/tokens.css.
//
// For a diff-only use case the single editor.worker is sufficient: Monaco runs
// its diff computation inside that worker. We intentionally do NOT register the
// json/ts/css/html language workers (not needed, and they would pull extra
// weight). Basic syntax highlighting comes from the bundled basic-languages.

import * as monaco from "monaco-editor";
// ?worker makes Vite emit this as a separate, locally-served worker chunk.
import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";

self.MonacoEnvironment = {
  getWorker() {
    return new EditorWorker();
  },
};

// Theme colors are taken verbatim from docs/design/tokens.css. Monaco cannot
// read CSS custom properties, so the hex values are inlined here. The base
// ("vs" / "vs-dark") supplies the token syntax colors; we only override editor
// surfaces, line numbers, and the diff add/delete backgrounds to match tokens.
monaco.editor.defineTheme("bdv-light", {
  base: "vs",
  inherit: true,
  rules: [],
  colors: {
    "editor.background": "#ffffff", // --bg
    "editor.foreground": "#1c2128", // --text
    "editorLineNumber.foreground": "#8b96a3", // --dim
    "editorLineNumber.activeForeground": "#59636e", // --muted
    "editorGutter.background": "#ffffff", // --bg
    "editorIndentGuide.background1": "#eceff2", // --border2
    "editor.lineHighlightBackground": "#f6f7f9", // --panel
    "diffEditor.insertedLineBackground": "#e9f7ee", // --add-bg
    "diffEditor.removedLineBackground": "#fdeef0", // --del-bg
    "diffEditor.insertedTextBackground": "#1a7f3726", // --green @ ~15%
    "diffEditor.removedTextBackground": "#d1242f26", // --red @ ~15%
    "diffEditor.diagonalFill": "#eceff2", // --border2 (empty side / --zebra)
  },
});

monaco.editor.defineTheme("bdv-dark", {
  base: "vs-dark",
  inherit: true,
  rules: [],
  colors: {
    "editor.background": "#10151b", // --bg
    "editor.foreground": "#e3e9ef", // --text
    "editorLineNumber.foreground": "#5a6673", // --dim
    "editorLineNumber.activeForeground": "#8b96a3", // --muted
    "editorGutter.background": "#10151b", // --bg
    "editorIndentGuide.background1": "#232c38", // --border2
    "editor.lineHighlightBackground": "#161c23", // --panel
    "diffEditor.insertedLineBackground": "#12261a", // --add-bg
    "diffEditor.removedLineBackground": "#2b161b", // --del-bg
    "diffEditor.insertedTextBackground": "#4cc38a26", // --green @ ~15%
    "diffEditor.removedTextBackground": "#f27d7d26", // --red @ ~15%
    "diffEditor.diagonalFill": "#182030", // --hunk-bg
  },
});

export const BDV_LIGHT = "bdv-light";
export const BDV_DARK = "bdv-dark";

/**
 * Best-effort language id from a file path, matched against Monaco's registered
 * languages by extension (then by exact filename, e.g. "Dockerfile").
 * Falls back to "plaintext". Everything is resolved locally — no network.
 */
export function detectLanguage(path: string): string {
  const lower = path.toLowerCase();
  const base = lower.split("/").pop() ?? lower;
  const languages = monaco.languages.getLanguages();

  for (const lang of languages) {
    if (lang.extensions?.some((ext) => lower.endsWith(ext.toLowerCase()))) {
      return lang.id;
    }
  }
  for (const lang of languages) {
    if (lang.filenames?.some((name) => name.toLowerCase() === base)) {
      return lang.id;
    }
  }
  return "plaintext";
}
