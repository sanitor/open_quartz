import { useGraphStore } from '../store/useGraphStore';
import { serializeProject, deserializeProject, saveFileAs, saveFile } from '../utils/projectIO';
import { useRef, useState, useEffect, useCallback } from 'react';
import { useReactFlow } from '@xyflow/react';
import { VERSION } from '../version';
import type { DataType, InputMode, ShaderNodeData } from '../types';
import { CUSTOM_SHADER_CODE, CUSTOM_2IN1_SHADER, shaderGroups } from '../engine/shaders';
import { ONNX_CATALOG, CATALOG_CATEGORIES } from '../engine/onnxCatalog';
import { MATH_CATEGORIES, MATH_OPS } from '../engine/mathOps';

const isMac = navigator.platform.startsWith('Mac');

function isInteractiveTarget(el: HTMLElement, boundary: HTMLElement): boolean {
  let cur: HTMLElement | null = el;
  while (cur && cur !== boundary) {
    const tag = cur.tagName;
    if (tag === 'BUTTON' || tag === 'INPUT' || tag === 'A' || tag === 'SELECT') return true;
    cur = cur.parentElement;
  }
  return false;
}

export function Header() {
  const { nodes, edges, projectName, savedFilePath, setProjectName, setSavedFilePath, loadGraph, clearGraph, undo, redo, undoStack, redoStack, loopState, fps, currentTime, play, pause, resume, stop, addRendererNode, addSystemNode, addMathNode } = useGraphStore();
  const { fitView } = useReactFlow();
  const fileInputRef = useRef<HTMLInputElement>(null);
  const headerRef = useRef<HTMLElement>(null);

  const [tauriApp, setTauriApp] = useState(false);

  useEffect(() => {
    let cancelled = false;
    import('@tauri-apps/api/core').then(({ isTauri }) => {
      if (cancelled) return;
      if (!isTauri()) return;
      setTauriApp(true);
      if (!isMac) {
        import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
          getCurrentWindow().setDecorations(false);
        });
      }
    }).catch(() => {});
    return () => { cancelled = true; };
  }, []);

  const handleHeaderMouseDown = useCallback((e: React.MouseEvent<HTMLElement>) => {
    if (!tauriApp) return;
    if (isInteractiveTarget(e.target as HTMLElement, e.currentTarget)) return;
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      getCurrentWindow().startDragging();
    });
  }, [tauriApp]);

  const handleWindowMinimize = () => {
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      getCurrentWindow().minimize();
    });
  };
  const handleWindowMaximize = () => {
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      getCurrentWindow().toggleMaximize();
    });
  };
  const handleWindowClose = () => {
    import('@tauri-apps/api/window').then(({ getCurrentWindow }) => {
      getCurrentWindow().close();
    });
  };

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

  const [saveAsOpen, setSaveAsOpen] = useState(false);
  const saveAsInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (saveAsOpen) {
      requestAnimationFrame(() => {
        const el = saveAsInputRef.current;
        if (el) {
          el.focus();
          const dot = el.value.indexOf('.');
          el.setSelectionRange(0, dot > 0 ? dot : el.value.length);
        }
      });
    }
  }, [saveAsOpen]);

  const handleSave = () => {
    if (!savedFilePath) return;
    const project = serializeProject(nodes, edges, projectName);
    saveFile(project, savedFilePath);
  };

  const handleSaveAs = () => {
    setSaveAsOpen(true);
  };

  const confirmSaveAs = () => {
    const raw = saveAsInputRef.current?.value.trim();
    if (!raw) { setSaveAsOpen(false); return; }
    const filename = raw.endsWith('.quartz.json') ? raw : `${raw}.quartz.json`;
    const project = serializeProject(nodes, edges, projectName);
    saveFileAs(project, filename);
    const baseName = filename.replace(/\.quartz\.json$/i, '');
    setSavedFilePath(filename);
    setProjectName(baseName);
    setSaveAsOpen(false);
  };

  const handleLoad = () => {
    fileInputRef.current?.click();
  };

  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = async (ev) => {
      try {
        const result = await deserializeProject(ev.target?.result as string);
        const baseName = file.name.replace(/\.quartz\.json$/i, '');
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

  const [editingName, setEditingName] = useState(false);
  const nameInputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editingName) nameInputRef.current?.select();
  }, [editingName]);

  const commitName = () => {
    const val = nameInputRef.current?.value.trim();
    if (val) {
      setProjectName(val);
      if (savedFilePath) setSavedFilePath(`${val}.quartz.json`);
    }
    setEditingName(false);
  };

  const [sourceOpen, setSourceOpen] = useState(false);
  const [shaderOpen, setShaderOpen] = useState(false);
  const [onnxOpen, setOnnxOpen] = useState(false);
  const [mathOpen, setMathOpen] = useState(false);

  const templateItems = [
    { label: 'CUSTOM SHADER', code: CUSTOM_SHADER_CODE },
    { label: 'CUSTOM 2IN-1', code: CUSTOM_2IN1_SHADER },
  ];

  const [shaderHoveredGroup, setShaderHoveredGroup] = useState<string | null>(null);
  const [mathHoveredGroup, setMathHoveredGroup] = useState<string | null>(null);

  type SourceItem = { label: string; type?: DataType; mode?: InputMode; system?: NonNullable<ShaderNodeData['systemSource']> };
  const sourceGroups: { label: string; items: SourceItem[] }[] = [
    { label: 'SYSTEM', items: [
      { label: 'TIME', system: 'time' },
      { label: 'TIME DELTA', system: 'timeDelta' },
      { label: 'FRAME', system: 'frame' },
      { label: 'MOUSE', system: 'mouse' },
      { label: 'RESOLUTION', system: 'resolution' },
    ]},
    { label: 'CONSTANTS', items: [
      { label: 'FLOAT', type: 'float' },
      { label: 'INT', type: 'int' },
      { label: 'UINT', type: 'uint' },
      { label: 'BOOL', type: 'bool' },
      { label: 'VEC2', type: 'vec2' },
      { label: 'VEC3', type: 'vec3' },
      { label: 'VEC4', type: 'vec4' },
      { label: 'IVEC2', type: 'ivec2' },
      { label: 'IVEC3', type: 'ivec3' },
      { label: 'IVEC4', type: 'ivec4' },
      { label: 'MAT2', type: 'mat2' },
      { label: 'MAT3', type: 'mat3' },
      { label: 'MAT4', type: 'mat4' },
    ]},
    { label: 'EXTERNAL', items: [
      { label: 'IMAGE', type: 'sampler2D', mode: 'image' },
      { label: 'FRAMEBUFFER', type: 'sampler2D', mode: 'framebuffer' },
      { label: 'VIDEO', type: 'sampler2D', mode: 'video' },
    ]},
  ];

  const [hoveredGroup, setHoveredGroup] = useState<string | null>(null);

  return (
    <header
      ref={headerRef}
      onMouseDown={handleHeaderMouseDown}
      className={`flex items-center gap-1 px-4 py-1 bg-white border-b border-[#d2d2d7] select-none text-[11px]${tauriApp && isMac ? ' pl-[88px]' : ''}`}
    >
      <span className="flex items-center gap-1.5 mr-2">
        <img src="/favicon.svg" alt="" className="w-[16px] h-[16px]" />
        <span className="font-bold text-[#1d1d1f] text-[13px] tracking-wider">OPENQUARTZ</span>
        <span className="text-[11px] text-[#aeaeb2]">v{VERSION}</span>
      </span>

      {editingName ? (
        <input
          ref={nameInputRef}
          defaultValue={projectName}
          onBlur={commitName}
          onKeyDown={(e) => { if (e.key === 'Enter') commitName(); if (e.key === 'Escape') setEditingName(false); }}
          className="text-[12px] text-[#1d1d1f] font-medium px-1 border border-[#007aff] rounded outline-none w-[120px]"
        />
      ) : (
        <span
          onDoubleClick={() => setEditingName(true)}
          className="text-[12px] text-[#1d1d1f] font-medium px-1 cursor-pointer hover:text-[#007aff]"
          title="Double-click to rename"
        >{projectName}</span>
      )}

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
      <input ref={fileInputRef} type="file" accept=".quartz.json,.json" onChange={handleFileChange} className="hidden" />

      <span className="mx-1 text-[#c7c7cc]">|</span>

      <div className="relative">
        <button onClick={() => setSourceOpen(!sourceOpen)} className={btnClass}>
          <svg viewBox="0 0 16 16" className={svgClass} fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round">
            <circle cx="4.5" cy="8" r="3" />
            <line x1="7.5" y1="8" x2="14" y2="8" />
          </svg>
          <span className="flex items-center gap-px">
            <span>SOURCE</span>
            <span className="text-[16px] leading-none font-normal">▾</span>
          </span>
        </button>
        {sourceOpen && (
          <>
            <div className="fixed inset-0 z-10" onMouseDown={() => { setSourceOpen(false); setHoveredGroup(null); }} />
            <div
              className="absolute top-full left-0 mt-0.5 bg-white border border-[#d2d2d7] rounded-lg shadow-lg z-20 py-1 min-w-[120px]"
              onMouseLeave={() => setHoveredGroup(null)}
            >
              {sourceGroups.map((group) => (
                <div
                  key={group.label}
                  className="relative"
                  onMouseEnter={() => setHoveredGroup(group.label)}
                >
                  <div className="flex items-center justify-between px-3 py-1 text-[9px] font-bold text-[#1d1d1f] hover:text-[#007aff] hover:bg-[#f5f5f7] transition-colors cursor-default">
                    <span>{group.label}</span>
                    <span className="text-[16px] ml-2">▸</span>
                  </div>
                  {hoveredGroup === group.label && (
                    <div className="absolute left-full top-0 -ml-1 pl-1 z-30">
                      <div className="bg-white border border-[#d2d2d7] rounded-lg shadow-lg py-1 min-w-[120px]">
                        {group.items.map((item) => (
                          <button
                            key={item.label}
                            onClick={() => {
                              if (item.system) {
                                addSystemNode(item.system);
                              } else if (item.type) {
                                useGraphStore.getState().addInputNode(item.type, undefined, item.mode);
                              }
                              setSourceOpen(false);
                              setHoveredGroup(null);
                            }}
                            className="block w-full text-left px-3 py-1 text-[9px] font-bold text-[#1d1d1f] hover:text-[#007aff] hover:bg-[#f5f5f7] transition-colors cursor-default"
                          >
                            {item.label}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              ))}
            </div>
          </>
        )}
      </div>

      <div className="relative">
        <button onClick={() => setMathOpen(!mathOpen)} className={btnClass}>
          <svg viewBox="0 0 16 16" className={svgClass} fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round">
            <text x="8" y="12" textAnchor="middle" fontSize="12" fill="currentColor" stroke="none" fontWeight="bold">±</text>
          </svg>
          <span className="flex items-center gap-px">
            <span>MATH</span>
            <span className="text-[16px] leading-none font-normal">▾</span>
          </span>
        </button>
        {mathOpen && (
          <>
            <div className="fixed inset-0 z-10" onMouseDown={() => { setMathOpen(false); setMathHoveredGroup(null); }} />
            <div
              className="absolute top-full left-0 mt-0.5 bg-white border border-[#d2d2d7] rounded-lg shadow-lg z-20 py-1 min-w-[140px]"
              onMouseLeave={() => setMathHoveredGroup(null)}
            >
              {MATH_CATEGORIES.map((group) => (
                <div
                  key={group.category}
                  className="relative"
                  onMouseEnter={() => setMathHoveredGroup(group.category)}
                >
                  <div className="flex items-center justify-between px-3 py-1 text-[9px] font-bold text-[#1d1d1f] hover:text-[#007aff] hover:bg-[#f5f5f7] transition-colors cursor-default">
                    <span>{group.category.toUpperCase()}</span>
                    <span className="text-[16px] ml-2">▸</span>
                  </div>
                  {mathHoveredGroup === group.category && (
                    <div className="absolute left-full top-0 -ml-1 pl-1 z-30">
                      <div className="bg-white border border-[#d2d2d7] rounded-lg shadow-lg py-1 min-w-[120px]">
                        {group.ops.map((opId) => {
                          const op = MATH_OPS[opId];
                          if (!op) return null;
                          return (
                            <button
                              key={opId}
                              onClick={() => {
                                addMathNode(opId);
                                setMathOpen(false);
                                setMathHoveredGroup(null);
                              }}
                              className="block w-full text-left px-3 py-1 text-[9px] font-bold text-[#1d1d1f] hover:text-[#007aff] hover:bg-[#f5f5f7] transition-colors cursor-default"
                            >
                              {op.label.toUpperCase()}
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  )}
                </div>
              ))}
            </div>
          </>
        )}
      </div>

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
            <span className="text-[16px] leading-none font-normal">▾</span>
          </span>
        </button>
        {shaderOpen && (
          <>
            <div className="fixed inset-0 z-10" onMouseDown={() => { setShaderOpen(false); setShaderHoveredGroup(null); }} />
            <div
              className="absolute top-full left-0 mt-0.5 bg-white border border-[#d2d2d7] rounded-lg shadow-lg z-20 py-1 min-w-[160px]"
              onMouseLeave={() => setShaderHoveredGroup(null)}
            >
              {templateItems.map((item) => (
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
              ))}
              <div className="mx-2 my-1 border-t border-[#e8e8ed]" />
              {shaderGroups.map((group) => (
                <div
                  key={group.category}
                  className="relative"
                  onMouseEnter={() => setShaderHoveredGroup(group.category)}
                >
                  <div className="flex items-center justify-between px-3 py-1 text-[9px] font-bold text-[#1d1d1f] hover:text-[#007aff] hover:bg-[#f5f5f7] transition-colors cursor-default">
                    <span>{group.category}</span>
                    <span className="text-[16px] ml-2">▸</span>
                  </div>
                  {shaderHoveredGroup === group.category && (
                    <div className="absolute left-full top-0 -ml-1 pl-1 z-30">
                      <div className="bg-white border border-[#d2d2d7] rounded-lg shadow-lg py-1 min-w-[120px]">
                        {group.items.map((item) => (
                          <button
                            key={item.label}
                            onClick={() => {
                              useGraphStore.getState().addShaderNode(item.code, item.label);
                              setShaderOpen(false);
                              setShaderHoveredGroup(null);
                            }}
                            className="block w-full text-left px-3 py-1 text-[9px] font-bold text-[#1d1d1f] hover:text-[#007aff] hover:bg-[#f5f5f7] transition-colors cursor-default"
                          >
                            {item.label.toUpperCase()}
                          </button>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              ))}
            </div>
          </>
        )}
      </div>

      <div className="relative">
        <button onClick={() => setOnnxOpen(!onnxOpen)} className={btnClass}>
          <svg viewBox="0 0 16 16" className={svgClass} fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="8" cy="8" r="6" />
            <path d="M5 8 L8 5 L11 8 L8 11 Z" />
          </svg>
          <span className="flex items-center gap-px">
            <span>ONNX</span>
            <span className="text-[16px] leading-none font-normal">▾</span>
          </span>
        </button>
        {onnxOpen && (
          <>
            <div className="fixed inset-0 z-10" onMouseDown={() => setOnnxOpen(false)} />
            <div className="absolute top-full left-0 mt-0.5 bg-white border border-[#d2d2d7] rounded-lg shadow-lg z-20 py-1 min-w-[220px] max-h-[400px] overflow-y-auto">
              {CATALOG_CATEGORIES.map((cat) => (
                <div key={cat}>
                  <div className="px-3 pt-2 pb-0.5 text-[8px] font-semibold text-[#86868b] uppercase tracking-wider">{cat}</div>
                  {Object.values(ONNX_CATALOG)
                    .filter((e) => e.category === cat)
                    .map((entry) => (
                      <button
                        key={entry.id}
                        onClick={() => {
                          useGraphStore.getState().addOnnxNode(entry.id);
                          setOnnxOpen(false);
                        }}
                        className="block w-full text-left px-3 py-1 text-[9px] font-bold text-[#1d1d1f] hover:text-[#007aff] hover:bg-[#f5f5f7] transition-colors cursor-default"
                      >
                        {entry.label.toUpperCase()}
                      </button>
                    ))}
                </div>
              ))}
              <div className="border-t border-[#d2d2d7] my-1" />
              <button
                onClick={() => {
                  useGraphStore.getState().addCustomOnnxNode();
                  setOnnxOpen(false);
                }}
                className="block w-full text-left px-3 py-1 text-[9px] font-bold text-[#1d1d1f] hover:text-[#007aff] hover:bg-[#f5f5f7] transition-colors cursor-default"
              >
                CUSTOM ONNX MODEL...
              </button>
            </div>
          </>
        )}
      </div>

      <button onClick={() => addRendererNode()} className={btnClass}>
        <svg viewBox="0 0 16 16" className={svgClass} fill="none" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round">
          <rect x="1.5" y="2.5" width="13" height="9" rx="1" />
          <line x1="5" y1="14" x2="11" y2="14" />
          <line x1="8" y1="11.5" x2="8" y2="14" />
        </svg>
        <span>RENDERER</span>
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

      <button onClick={() => { clearGraph(); }} className={btnClass}>
        <span className={iconClass}>✕</span>
        <span>CLEAR</span>
      </button>

      <div className="ml-auto flex items-center gap-1">
        {/* FPS display */}
        {loopState !== 'stopped' && (
          <span className="text-[9px] font-mono text-[#86868b] mr-2">
            {fps > 0 ? `${Math.round(fps)} FPS` : '-- FPS'}
          </span>
        )}

        {/* Time display */}
        {loopState !== 'stopped' && (
          <span className="text-[9px] font-mono text-[#86868b] mr-2">
            {currentTime.toFixed(1)}s
          </span>
        )}


        {/* PLAY / PAUSE / RESUME */}
        {loopState === 'stopped' ? (
          <button onClick={play} className={btnClass}>
            <span className={iconClass}>▶</span>
            <span>PLAY</span>
          </button>
        ) : loopState === 'playing' ? (
          <button onClick={pause} className={btnClass}>
            <span className={iconClass}>⏸</span>
            <span>PAUSE</span>
          </button>
        ) : (
          <button onClick={resume} className={btnClass}>
            <span className={iconClass}>▶</span>
            <span>RESUME</span>
          </button>
        )}

        {/* STOP (only when playing/paused) */}
        {loopState !== 'stopped' && (
          <button onClick={() => { stop(); }} className={`${btnClass} !text-[#ff3b30]`}>
            <span className={iconClass}>■</span>
            <span>STOP</span>
          </button>
        )}
      </div>

      {tauriApp && !isMac && (
        <div className="flex items-center ml-2">
          <button onClick={handleWindowMinimize} className="w-[28px] h-[28px] flex items-center justify-center text-[#86868b] hover:bg-[#e8e8ed] rounded transition-colors" title="Minimize">
            <svg width="10" height="1" viewBox="0 0 10 1"><rect width="10" height="1" fill="currentColor" /></svg>
          </button>
          <button onClick={handleWindowMaximize} className="w-[28px] h-[28px] flex items-center justify-center text-[#86868b] hover:bg-[#e8e8ed] rounded transition-colors" title="Maximize">
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1"><rect x="0.5" y="0.5" width="9" height="9" /></svg>
          </button>
          <button onClick={handleWindowClose} className="w-[28px] h-[28px] flex items-center justify-center text-[#86868b] hover:bg-[#ff3b30] hover:text-white rounded transition-colors" title="Close">
            <svg width="10" height="10" viewBox="0 0 10 10" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round"><line x1="1" y1="1" x2="9" y2="9" /><line x1="9" y1="1" x2="1" y2="9" /></svg>
          </button>
        </div>
      )}

      {saveAsOpen && (
        <>
          <div className="fixed inset-0 bg-black/20 z-50" onClick={() => setSaveAsOpen(false)} />
          <div className="fixed top-1/3 left-1/2 -translate-x-1/2 bg-white rounded-lg shadow-xl border border-[#d2d2d7] p-4 z-50 w-[320px]">
            <div className="text-[11px] font-bold text-[#1d1d1f] mb-2">SAVE AS</div>
            <input
              ref={saveAsInputRef}
              defaultValue={`${projectName}.quartz.json`}
              onKeyDown={(e) => { if (e.key === 'Enter') confirmSaveAs(); if (e.key === 'Escape') setSaveAsOpen(false); }}
              className="w-full text-[12px] px-2 py-1.5 border border-[#d2d2d7] rounded outline-none focus:border-[#007aff]"
            />
            <div className="flex justify-end gap-2 mt-3">
              <button onClick={() => setSaveAsOpen(false)} className="text-[10px] font-bold text-[#86868b] hover:text-[#1d1d1f] px-3 py-1">CANCEL</button>
              <button onClick={confirmSaveAs} className="text-[10px] font-bold text-white bg-[#007aff] hover:bg-[#0066d6] px-3 py-1 rounded">SAVE</button>
            </div>
          </div>
        </>
      )}
    </header>
  );
}
