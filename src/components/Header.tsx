import { useGraphStore } from '../store/useGraphStore';
import { serializeProject, deserializeProject, saveFileAs, saveFile } from '../utils/projectIO';
import { useRef, useState, useEffect, useCallback } from 'react';
import { useReactFlow } from '@xyflow/react';
import { VERSION } from '../version';
import type { DataType } from '../types';
import { CUSTOM_SHADER_CODE, CUSTOM_2IN1_SHADER, predefinedShaders } from '../engine/predefinedShaders';

export function Header() {
  const { nodes, edges, projectName, savedFilePath, setProjectName, setSavedFilePath, isRunning, setRunning, loadGraph, clearGraph, undo, redo, undoStack, redoStack } = useGraphStore();
  const { fitView } = useReactFlow();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const fileHandleRef = useRef<FileSystemFileHandle | null>(null);

  const fitAfterLoad = useCallback(() => {
    requestAnimationFrame(() => fitView({ duration: 200 }));
  }, [fitView]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === 'z') {
        e.preventDefault();
        if (e.shiftKey) {
          redo();
        } else {
          undo();
        }
      }
      if ((e.ctrlKey || e.metaKey) && e.key === 'y') {
        e.preventDefault();
        redo();
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [undo, redo]);

  const handleSave = async () => {
    if (!savedFilePath) return;
    const project = serializeProject(nodes, edges, projectName);
    await saveFile(project, fileHandleRef.current, savedFilePath);
  };

  const handleSaveAs = async () => {
    const project = serializeProject(nodes, edges, projectName);
    const result = await saveFileAs(project);
    if (!result) return;
    fileHandleRef.current = result.handle;
    const baseName = result.name.replace(/\.quartz\.json$/i, '');
    setSavedFilePath(result.name);
    setProjectName(baseName);
  };

  const handleLoad = () => {
    fileInputRef.current?.click();
  };

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (ev) => {
      try {
        const result = deserializeProject(ev.target?.result as string);
        const baseName = file.name.replace(/\.quartz\.json$/i, '');
        fileHandleRef.current = null;
        loadGraph(result.nodes, result.edges);
        setProjectName(baseName);
        setSavedFilePath(file.name);
        fitAfterLoad();
      } catch (err) {
        console.error('Failed to load project:', err);
        alert('Failed to load project file.');
      }
    };
    reader.readAsText(file);
    e.target.value = '';
  };

  const btnClass = 'flex flex-col items-center gap-0.5 px-1.5 py-1 text-[9px] font-bold text-[#1d1d1f] hover:text-[#007aff] transition-colors cursor-default';
  const btnDisabledClass = 'flex flex-col items-center gap-0.5 px-1.5 py-1 text-[9px] font-bold text-[#aeaeb2] cursor-default';
  const iconClass = 'text-[14px] leading-none font-normal';
  const svgClass = 'w-[14px] h-[14px]';

  const [inputOpen, setInputOpen] = useState(false);
  const [shaderOpen, setShaderOpen] = useState(false);

  const shaderItems = [
    { label: 'CUSTOM SHADER', code: CUSTOM_SHADER_CODE, custom: true },
    { label: 'CUSTOM 2IN-1OUT', code: CUSTOM_2IN1_SHADER, custom: true },
    { separator: true },
    ...predefinedShaders.map((s) => ({ label: s.label, code: s.code, custom: false })),
  ];

  const inputTypes: { label: string; type: DataType }[] = [
    { label: 'FLOAT', type: 'float' },
    { label: 'INT', type: 'int' },
    { label: 'BOOL', type: 'bool' },
    { label: 'VEC2', type: 'vec2' },
    { label: 'VEC3', type: 'vec3' },
    { label: 'VEC4', type: 'vec4' },
    { label: 'IMAGE', type: 'sampler2D' },
  ];

  return (
    <header className="flex items-center gap-1 px-4 py-1 bg-white border-b border-[#d2d2d7] select-none text-[11px]">
      <span className="flex items-baseline gap-1.5 mr-2">
        <span className="font-bold text-[#1d1d1f] text-[13px] tracking-wider">OPENQUARTZ</span>
        <span className="text-[11px] text-[#aeaeb2]">v{VERSION}</span>
      </span>

      <span className="text-[12px] text-[#1d1d1f] font-medium px-1">{projectName}</span>

      <span className="mx-1 text-[#c7c7cc]">|</span>

      <button onClick={handleSave} disabled={!savedFilePath} className={savedFilePath ? btnClass : btnDisabledClass}>
        <svg viewBox="0 0 16 16" className={svgClass} fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M2 1a1 1 0 0 0-1 1v12a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V4.414a1 1 0 0 0-.293-.707l-2.414-2.414A1 1 0 0 0 11.586 1H2z" />
          <path d="M3 1v3a1 1 0 0 0 1 1h4a1 1 0 0 0 1-1V1" />
          <path d="M5 9a2 2 0 1 1 4 0 2 2 0 0 1-4 0z" />
        </svg>
        <span>SAVE</span>
      </button>
      <button onClick={handleSaveAs} disabled={nodes.length === 0} className={nodes.length > 0 ? btnClass : btnDisabledClass}>
        <svg viewBox="0 0 16 16" className={svgClass} fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M2 1a1 1 0 0 0-1 1v12a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V4.414a1 1 0 0 0-.293-.707l-2.414-2.414A1 1 0 0 0 11.586 1H2z" />
          <path d="M3 1v3a1 1 0 0 0 1 1h4a1 1 0 0 0 1-1V1" />
          <path d="M5 9a2 2 0 1 1 4 0 2 2 0 0 1-4 0z" />
          <text x="8" y="11" textAnchor="middle" fontSize="7" fill="currentColor" stroke="none" fontWeight="bold">AS</text>
        </svg>
        <span>SAVE AS</span>
      </button>
      <button onClick={handleLoad} className={btnClass}>
        <svg viewBox="0 0 16 16" className={svgClass} fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M1.5 4.5h4l1.5 1.5H13a1 1 0 0 1 1 1V12a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V5a1 1 0 0 1 .5-.5z" />
          <path d="M1 7.5h8.5l2 4H2.5z" />
        </svg>
        <span>LOAD</span>
      </button>
      <input ref={fileInputRef} type="file" accept=".quartz.json" onChange={handleFileChange} className="hidden" />

      <span className="mx-1 text-[#c7c7cc]">|</span>

      <div className="relative">
        <button onClick={() => setShaderOpen(!shaderOpen)} className={btnClass}>
          <svg viewBox="0 0 16 16" className={svgClass} fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round">
            <rect x="3.5" y="3.5" width="9" height="9" rx="1" />
            <line x1="5" y1="3.5" x2="5" y2="1.5" />
            <line x1="8" y1="3.5" x2="8" y2="1" />
            <line x1="11" y1="3.5" x2="11" y2="1.5" />
            <line x1="5" y1="12.5" x2="5" y2="14.5" />
            <line x1="8" y1="12.5" x2="8" y2="15" />
            <line x1="11" y1="12.5" x2="11" y2="14.5" />
            <line x1="3.5" y1="5" x2="1.5" y2="5" />
            <line x1="3.5" y1="8" x2="1" y2="8" />
            <line x1="3.5" y1="11" x2="1.5" y2="11" />
            <line x1="12.5" y1="5" x2="14.5" y2="5" />
            <line x1="12.5" y1="8" x2="15" y2="8" />
            <line x1="12.5" y1="11" x2="14.5" y2="11" />
          </svg>
          <span className="flex items-center gap-px">
            <span>SHADER</span>
            <span className="text-[8px] leading-none font-normal">▾</span>
          </span>
        </button>
        {shaderOpen && (
          <>
            <div className="fixed inset-0 z-10" onClick={() => setShaderOpen(false)} />
            <div className="absolute top-full left-0 mt-0.5 bg-white border border-[#d2d2d7] rounded-lg shadow-lg z-20 py-1 min-w-[160px]">
              {shaderItems.map((item, i) => {
                if ('separator' in item) {
                  return <div key={`sep_${i}`} className="mx-2 my-1 border-t border-[#e8e8ed]" />;
                }
                return (
                  <button
                    key={item.label}
                    onClick={() => {
                      useGraphStore.getState().addShaderNode(item.code, item.label);
                      setShaderOpen(false);
                    }}
                    className="block w-full text-left px-3 py-1 text-[9px] font-bold text-[#1d1d1f] hover:text-[#007aff] hover:bg-[#f5f5f7] transition-colors cursor-default"
                  >
                    {item.label}
                  </button>
                );
              })}
            </div>
          </>
        )}
      </div>

      <div className="relative">
        <button onClick={() => setInputOpen(!inputOpen)} className={btnClass}>
          <svg viewBox="0 0 16 16" className={svgClass} fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round">
            <circle cx="4.5" cy="8" r="3" />
            <line x1="7.5" y1="8" x2="14" y2="8" />
          </svg>
          <span className="flex items-center gap-px">
            <span>INPUT</span>
            <span className="text-[8px] leading-none font-normal">▾</span>
          </span>
        </button>
        {inputOpen && (
          <>
            <div className="fixed inset-0 z-10" onClick={() => setInputOpen(false)} />
            <div className="absolute top-full left-0 mt-0.5 bg-white border border-[#d2d2d7] rounded-lg shadow-lg z-20 py-1 min-w-[100px]">
              {inputTypes.map(({ label, type }) => (
                <button
                  key={type}
                  onClick={() => { useGraphStore.getState().addInputNode(type); setInputOpen(false); }}
                  className="block w-full text-left px-3 py-1 text-[9px] font-bold text-[#1d1d1f] hover:text-[#007aff] hover:bg-[#f5f5f7] transition-colors cursor-default"
                >
                  {label}
                </button>
              ))}
            </div>
          </>
        )}
      </div>

      <button onClick={() => useGraphStore.getState().addNode('output')} className={btnClass}>
        <svg viewBox="0 0 16 16" className={svgClass} fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round">
          <line x1="2" y1="8" x2="8.5" y2="8" />
          <circle cx="11.5" cy="8" r="3" />
        </svg>
        <span>OUTPUT</span>
      </button>

      <span className="mx-1 text-[#c7c7cc]">|</span>

      <button
        onClick={undo}
        disabled={undoStack.length === 0}
        className={`flex flex-col items-center gap-0.5 px-1.5 py-1 text-[9px] font-bold transition-colors cursor-default ${
          undoStack.length === 0 ? 'text-[#aeaeb2]' : 'text-[#1d1d1f] hover:text-[#007aff]'
        }`}
      >
        <span className={iconClass}>↩</span>
        <span>UNDO</span>
      </button>
      <button
        onClick={redo}
        disabled={redoStack.length === 0}
        className={`flex flex-col items-center gap-0.5 px-1.5 py-1 text-[9px] font-bold transition-colors cursor-default ${
          redoStack.length === 0 ? 'text-[#aeaeb2]' : 'text-[#1d1d1f] hover:text-[#007aff]'
        }`}
      >
        <span className={iconClass}>↪</span>
        <span>REDO</span>
      </button>

      <span className="mx-1 text-[#c7c7cc]">|</span>

      <button onClick={() => { fileHandleRef.current = null; clearGraph(); }} className={btnClass}>
        <span className={iconClass}>✕</span>
        <span>CLEAR</span>
      </button>

      <div className="ml-auto">
        <button
          onClick={() => setRunning(!isRunning)}
          className={`flex flex-col items-center gap-0.5 px-1.5 py-1 text-[9px] font-bold transition-colors cursor-default ${
            isRunning ? 'text-[#ff3b30] hover:text-[#ff3b30]' : 'text-[#1d1d1f] hover:text-[#007aff]'
          }`}
        >
          <span className={iconClass}>{isRunning ? '□' : '▷'}</span>
          <span>{isRunning ? 'STOP' : 'RUN'}</span>
        </button>
      </div>
    </header>
  );
}
