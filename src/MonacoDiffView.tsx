import { useEffect, useRef } from "react";
import * as monaco from "monaco-editor";
import { BDV_DARK, BDV_LIGHT, detectLanguage } from "./monaco/setup";

export interface MonacoDiffViewProps {
  /** File path — used only for language detection. */
  path: string;
  /** Base (original / target) full text. */
  original: string;
  /** Head (modified / source) full text. */
  modified: string;
  theme: "light" | "dark";
}

/**
 * Side-by-side Monaco Diff View over two full-text models (DESIGN.md 3.5 / 4.4).
 * Monaco computes the diff itself from the original/modified models — no patch
 * is fed in. The editor is created once and reused; models are swapped when the
 * selected file changes and disposed to avoid leaks.
 */
export function MonacoDiffView({ path, original, modified, theme }: MonacoDiffViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneDiffEditor | null>(null);
  const modelsRef = useRef<{
    original: monaco.editor.ITextModel;
    modified: monaco.editor.ITextModel;
  } | null>(null);

  // Create the diff editor once.
  useEffect(() => {
    if (!containerRef.current) return;
    const editor = monaco.editor.createDiffEditor(containerRef.current, {
      readOnly: true,
      originalEditable: false,
      automaticLayout: true,
      renderSideBySide: true,
      ignoreTrimWhitespace: true, // Hide-whitespace approximation (DESIGN.md 3.5)
      maxComputationTime: 5000, // fall back to plain render on huge diffs (4.4)
      theme: theme === "dark" ? BDV_DARK : BDV_LIGHT,
      scrollBeyondLastLine: false,
      fontSize: 12,
      renderOverviewRuler: true,
      minimap: { enabled: false },
    });
    editorRef.current = editor;

    return () => {
      editor.dispose();
      modelsRef.current?.original.dispose();
      modelsRef.current?.modified.dispose();
      modelsRef.current = null;
      editorRef.current = null;
    };
    // Intentionally create-once; theme/model updates handled in effects below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Swap models whenever the file or its contents change.
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor) return;
    const language = detectLanguage(path);

    const previous = modelsRef.current;
    const originalModel = monaco.editor.createModel(original, language);
    const modifiedModel = monaco.editor.createModel(modified, language);
    editor.setModel({ original: originalModel, modified: modifiedModel });
    modelsRef.current = { original: originalModel, modified: modifiedModel };

    previous?.original.dispose();
    previous?.modified.dispose();
  }, [path, original, modified]);

  // React to theme toggles (setTheme is global to all Monaco editors).
  useEffect(() => {
    monaco.editor.setTheme(theme === "dark" ? BDV_DARK : BDV_LIGHT);
  }, [theme]);

  return <div ref={containerRef} className="monaco-diff-view" />;
}
