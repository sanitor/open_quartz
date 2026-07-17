import { useEffect, useRef, useCallback } from 'react';
import { EditorView, basicSetup } from 'codemirror';
import { EditorState } from '@codemirror/state';
import { glsl } from 'codemirror-lang-glsl';
import { linter } from '@codemirror/lint';
import { autocompletion } from '@codemirror/autocomplete';
import { glslLinter } from '../../engine/shaderLinter';
import { glslCompletions } from '../../engine/shaderCompletions';

interface ShaderEditorProps {
  code: string;
  onChange: (code: string) => void;
}

export function ShaderEditor({ code, onChange }: ShaderEditorProps) {
  const editorRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);

  const onUpdate = useCallback(
    (update: any) => {
      if (update.docChanged) {
        onChange(update.state.doc.toString());
      }
    },
    [onChange],
  );

  useEffect(() => {
    if (!editorRef.current) return;

    const state = EditorState.create({
      doc: code,
      extensions: [
        basicSetup,
        EditorView.lineWrapping,
        glsl(),
        linter(glslLinter),
        autocompletion({ override: [glslCompletions] }),
        EditorView.updateListener.of(onUpdate),
        EditorView.theme({
          '&': { fontSize: '12px', backgroundColor: '#ffffff', height: '100%' },
          '.cm-scroller': { fontFamily: "'SF Mono', 'Fira Code', 'Consolas', monospace", overflow: 'auto' },
          '.cm-gutters': { backgroundColor: '#fafafa', borderRight: '1px solid #e8e8ed' },
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
