import { useEffect, useRef, useCallback } from 'react';
import { EditorView, basicSetup } from 'codemirror';
import { EditorState } from '@codemirror/state';
import { linter, type Diagnostic } from '@codemirror/lint';
import { wgsl } from '@iizukak/codemirror-lang-wgsl';
import { parseWgslShader } from '../../engine/gpu/wgslParser';
import { validateWgslEdit } from '../../engine/gpu/wgslCompiler';

interface ShaderEditorProps {
  code: string;
  onChange: (code: string) => void;
  readOnly?: boolean;
}

/** Debounce delay before committing code changes to the store (port reparse). */
const COMMIT_DELAY_MS = 400;

/**
 * WGSL linter that runs async GPU validation via createShaderModule +
 * getCompilationInfo.  Errors appear as red squiggly underlines in the editor.
 * CodeMirror's linter extension already debounces internally (~750ms default).
 */
const wgslLinter = linter(async (view): Promise<Diagnostic[]> => {
  const code = view.state.doc.toString();
  if (!code.trim()) return [];

  // Parse to extract ports (needed for preamble generation)
  const parsed = parseWgslShader(code);
  if (parsed.parseError) {
    // wgsl_reflect caught a syntax error — show on line 1
    return [{ from: 0, to: Math.min(code.length, 1), severity: 'error', message: parsed.parseError }];
  }

  const errors = await validateWgslEdit(code, parsed.inputs);
  const diagnostics: Diagnostic[] = [];
  for (const err of errors) {
    // Map 1-based line to CodeMirror offset
    const lineNum = Math.min(err.line, view.state.doc.lines);
    const line = view.state.doc.line(lineNum);
    const from = line.from + Math.min(err.column, line.length);
    const to = err.length > 0
      ? Math.min(from + err.length, line.to)
      : line.to; // underline to end of line if no span
    diagnostics.push({ from, to, severity: 'error', message: err.message });
  }
  return diagnostics;
}, { delay: 500 });

export function ShaderEditor({ code, onChange, readOnly }: ShaderEditorProps) {
  const editorRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const commitTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  // Debounced commit: only push code to store after user stops typing
  const scheduleCommit = useCallback((newCode: string) => {
    if (commitTimer.current) clearTimeout(commitTimer.current);
    commitTimer.current = setTimeout(() => {
      onChangeRef.current(newCode);
    }, COMMIT_DELAY_MS);
  }, []);

  useEffect(() => {
    if (!editorRef.current) return;

    const state = EditorState.create({
      doc: code,
      extensions: [
        basicSetup,
        EditorView.lineWrapping,
        wgsl(),
        ...(readOnly
          ? [EditorState.readOnly.of(true), EditorView.editable.of(false)]
          : [wgslLinter]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) {
            scheduleCommit(update.state.doc.toString());
          }
        }),
        EditorView.theme({
          '&': { fontSize: '12px', backgroundColor: readOnly ? '#f9f9f9' : '#ffffff', height: '100%' },
          '.cm-scroller': { fontFamily: "'SF Mono', 'Fira Code', 'Consolas', monospace", overflow: 'auto' },
          '.cm-gutters': { backgroundColor: readOnly ? '#f0f0f0' : '#fafafa', borderRight: '1px solid #e8e8ed' },
          '.cm-activeLineGutter': { backgroundColor: '#f0f0f0' },
          '.cm-activeLine': { backgroundColor: 'rgba(245, 245, 247, 0.5)' },
          '.cm-cursor': { borderLeftColor: '#007aff' },
          '.cm-selectionBackground': { backgroundColor: '#b3d7ff' },
          '&.cm-focused .cm-selectionBackground': { backgroundColor: '#b3d7ff' },
        }),
      ],
    });

    const view = new EditorView({
      state,
      parent: editorRef.current,
    });

    viewRef.current = view;

    return () => {
      if (commitTimer.current) clearTimeout(commitTimer.current);
      view.destroy();
      viewRef.current = null;
    };
  }, []);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;

    const current = view.state.doc.toString();
    if (current !== code) {
      view.dispatch({
        changes: { from: 0, to: current.length, insert: code },
      });
    }
  }, [code]);

  return <div ref={editorRef} className="h-full w-full overflow-hidden" />;
}
