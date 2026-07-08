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
  /** Hide-whitespace toggle → Monaco `ignoreTrimWhitespace` (DESIGN.md 3.5
   * "各ファイルの2ペイン表示は ignoreTrimWhitespace で近似"). */
  ignoreWhitespace: boolean;
  /** Soft-wrap long lines (diff-pane header "Wrap" toggle). */
  wrap: boolean;
  /** Called after each diff (re)computation with whether Monaco gave up early
   * — i.e. the two sides differ but no line-level changes were produced within
   * `maxComputationTime` (DESIGN.md 4.4 / design 3e/5f). Monaco 0.55 doesn't
   * expose `getDiffComputationResult().quitEarly`, so this is approximated as
   * "models differ but getLineChanges() is null". */
  onTimeoutChange?: (timedOut: boolean) => void;
}

/**
 * Side-by-side Monaco Diff View over two full-text models (DESIGN.md 3.5 / 4.4).
 * Monaco computes the diff itself from the original/modified models — no patch
 * is fed in. The editor is created once and reused; models are swapped when the
 * selected file changes and disposed to avoid leaks.
 */
export function MonacoDiffView({
  path,
  original,
  modified,
  theme,
  ignoreWhitespace,
  wrap,
  onTimeoutChange,
}: MonacoDiffViewProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const editorRef = useRef<monaco.editor.IStandaloneDiffEditor | null>(null);
  const modelsRef = useRef<{
    original: monaco.editor.ITextModel;
    modified: monaco.editor.ITextModel;
  } | null>(null);
  const onTimeoutRef = useRef(onTimeoutChange);
  onTimeoutRef.current = onTimeoutChange;

  // Create the diff editor once.
  useEffect(() => {
    if (!containerRef.current) return;
    const editor = monaco.editor.createDiffEditor(containerRef.current, {
      readOnly: true,
      originalEditable: false,
      automaticLayout: true,
      renderSideBySide: true,
      ignoreTrimWhitespace: true, // updated by an effect below
      maxComputationTime: 5000, // fall back to plain render on huge diffs (4.4)
      theme: theme === "dark" ? BDV_DARK : BDV_LIGHT,
      scrollBeyondLastLine: false,
      fontSize: 12,
      renderOverviewRuler: true,
      minimap: { enabled: false },
    });
    editorRef.current = editor;

    // Approximate Monaco's `quitEarly` (not exposed in 0.55): after each
    // recomputation, if the two sides differ yet no line changes came back,
    // the computation was truncated by `maxComputationTime` (DESIGN.md 4.4).
    const sub = editor.onDidUpdateDiff(() => {
      const m = editor.getModel();
      if (!m) return;
      const differ = m.original.getValue() !== m.modified.getValue();
      const changes = editor.getLineChanges();
      const timedOut = differ && (changes === null || changes.length === 0);
      onTimeoutRef.current?.(timedOut);
    });

    return () => {
      sub.dispose();
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

  // Hide-whitespace + wrap toggles (cheap live updates, no editor recreate).
  useEffect(() => {
    editorRef.current?.updateOptions({
      ignoreTrimWhitespace: ignoreWhitespace,
      diffWordWrap: wrap ? "on" : "off",
      wordWrap: wrap ? "on" : "off",
    });
  }, [ignoreWhitespace, wrap]);

  // React to theme toggles (setTheme is global to all Monaco editors).
  useEffect(() => {
    monaco.editor.setTheme(theme === "dark" ? BDV_DARK : BDV_LIGHT);
  }, [theme]);

  return <div ref={containerRef} className="monaco-diff-view" />;
}
