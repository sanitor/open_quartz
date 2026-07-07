import { useCallback, useMemo, useEffect, useRef, useState } from 'react';
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  useReactFlow,
  useOnViewportChange,
  useConnection,
  getBezierPath,
  type Node,
  type NodeTypes,
  type EdgeTypes,
  type ConnectionLineComponentProps,
  type Connection,
  type Edge,
  SelectionMode,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useGraphStore } from '../../store/useGraphStore';
import type { ShaderNodeData } from '../../types';
import { ShaderNode } from './nodes/ShaderNode';
import { InputNode } from './nodes/InputNode';
import { OutputNode } from './nodes/OutputNode';
import { CustomEdge } from './edges/CustomEdge';

const nodeTypes: NodeTypes = {
  shader: ShaderNode,
  input: InputNode,
  output: OutputNode,
  constant: ShaderNode,
};

const edgeTypes: EdgeTypes = {
  bezier: CustomEdge,
};

interface ClipboardNode {
  id: string;
  type: string;
  position: { x: number; y: number };
  data: ShaderNodeData;
}

function MiniMapController() {
  const reactFlowInstance = useReactFlow();
  const [showMiniMap, setShowMiniMap] = useState(false);

  const checkNodesFit = useCallback(() => {
    const flowNodes = reactFlowInstance.getNodes();
    if (flowNodes.length === 0) {
      setShowMiniMap(false);
      return;
    }

    const viewport = reactFlowInstance.getViewport();
    const bounds = reactFlowInstance.getNodesBounds(flowNodes);

    const container = document.querySelector('.react-flow') as HTMLElement | null;
    if (!container) return;
    const { width, height } = container.getBoundingClientRect();

    const viewLeft = -viewport.x / viewport.zoom;
    const viewTop = -viewport.y / viewport.zoom;
    const viewRight = viewLeft + width / viewport.zoom;
    const viewBottom = viewTop + height / viewport.zoom;

    const fits = viewLeft <= bounds.x &&
                 viewTop <= bounds.y &&
                 viewRight >= bounds.x + bounds.width &&
                 viewBottom >= bounds.y + bounds.height;

    setShowMiniMap(!fits);
  }, [reactFlowInstance]);

  useOnViewportChange({ onChange: checkNodesFit });

  useEffect(() => {
    checkNodesFit();

    const container = document.querySelector('.react-flow') as HTMLElement | null;
    if (!container) return;

    const observer = new ResizeObserver(() => checkNodesFit());
    observer.observe(container);
    return () => observer.disconnect();
  }, [checkNodesFit]);

  if (!showMiniMap) return null;

  return (
    <MiniMap
      className="!rounded-lg !shadow-sm"
      nodeColor="#d0d0d0"
      maskColor="rgba(0,0,0,0.3)"
    />
  );
}

function isConnectionValid(connection: Connection | Edge): boolean {
  const { nodes } = useGraphStore.getState();
  const sourceNode = nodes.find((n) => n.id === connection.source);
  const targetNode = nodes.find((n) => n.id === connection.target);
  if (!sourceNode || !targetNode) return false;

  const sourcePort = sourceNode.data.outputs.find((p) => p.id === connection.sourceHandle);
  const targetPort = targetNode.data.inputs.find((p) => p.id === connection.targetHandle);
  if (!sourcePort || !targetPort) return false;

  if (targetPort.dataType === 'sampler2D' || targetPort.dataType === 'samplerCube') {
    if (sourcePort.dataType === 'sampler2D' || sourcePort.dataType === 'samplerCube') return true;
    const srcType = sourceNode.data.type;
    if (srcType === 'shader' || srcType === 'output' || srcType === 'constant') return true;
    if (srcType === 'input' && sourceNode.data.inputDataType === 'sampler2D') return true;
    return false;
  }
  return sourcePort.dataType === targetPort.dataType;
}

function ConnectionLine({ fromX, fromY, toX, toY, fromPosition, toPosition }: ConnectionLineComponentProps) {
  const { isValid } = useConnection();

  const [path] = getBezierPath({
    sourceX: fromX,
    sourceY: fromY,
    sourcePosition: fromPosition,
    targetX: toX,
    targetY: toY,
    targetPosition: toPosition,
  });

  let stroke = '#8e8e93';
  let strokeWidth = 1.5;
  if (isValid === true) {
    stroke = '#007aff';
    strokeWidth = 2;
  } else if (isValid === false) {
    stroke = '#ff3b30';
    strokeWidth = 3;
  }

  return <path d={path} fill="none" stroke={stroke} strokeWidth={strokeWidth} />;
}

export function NodeGraph() {
  const {
    nodes,
    edges,
    onNodesChange,
    onEdgesChange,
    onConnect,
    setSelectedNode,
    removeSelectedElements,
  } = useGraphStore();

  const clipboardRef = useRef<{ nodes: ClipboardNode[]; edges: { source: string; target: string; sourceHandle: string | null; targetHandle: string | null }[] } | null>(null);

  const isInputFocused = () => {
    const el = document.activeElement;
    if (!el) return false;
    const tag = el.tagName;
    if (tag === 'INPUT' || tag === 'TEXTAREA') return true;
    if (el.classList.contains('cm-editor') || el.closest('.cm-editor')) return true;
    if (el.getAttribute('contenteditable') === 'true') return true;
    return false;
  };

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (isInputFocused()) return;

      if (e.key === 'Delete' || e.key === 'Backspace') {
        e.preventDefault();
        removeSelectedElements();
        return;
      }

      if ((e.ctrlKey || e.metaKey) && e.key === 'c') {
        e.preventDefault();
        const store = useGraphStore.getState();
        const selectedNodes = store.nodes.filter((n) => n.selected);
        if (selectedNodes.length === 0) return;
        const selectedIds = selectedNodes.map((n) => n.id);
        const clipboard: ClipboardNode[] = selectedNodes.map((n) => ({
          id: n.id,
          type: n.type!,
          position: { ...n.position },
          data: structuredClone(n.data),
        }));
        const internalEdges = store.edges.filter(
          (e) => selectedIds.includes(e.source) && selectedIds.includes(e.target)
        ).map((e) => ({ source: e.source, target: e.target, sourceHandle: e.sourceHandle ?? null, targetHandle: e.targetHandle ?? null }));
        clipboardRef.current = { nodes: clipboard, edges: internalEdges };
        return;
      }

      if ((e.ctrlKey || e.metaKey) && e.key === 'v') {
        e.preventDefault();
        const cb = clipboardRef.current;
        if (!cb || cb.nodes.length === 0) return;

        const store = useGraphStore.getState();
        const existingIds = new Set(store.nodes.map((n) => n.id));
        const idMap = new Map<string, string>();
        const newNodes: Node<ShaderNodeData>[] = [];

        for (const cn of cb.nodes) {
          let newId = cn.id;
          while (existingIds.has(newId)) {
            newId = cn.id + '_copy';
          }
          existingIds.add(newId);
          idMap.set(cn.id, newId);

          newNodes.push({
            id: newId,
            type: cn.type,
            position: { x: cn.position.x + 40, y: cn.position.y + 40 },
            data: structuredClone(cn.data),
          });
        }

        const newEdges = cb.edges.map((e) => ({
          id: `${idMap.get(e.source)}_${idMap.get(e.target)}`,
          source: idMap.get(e.source)!,
          target: idMap.get(e.target)!,
          sourceHandle: e.sourceHandle,
          targetHandle: e.targetHandle,
          type: 'bezier' as const,
        }));

        store.pushHistory();
        useGraphStore.setState((state) => {
          state.nodes.push(...newNodes);
          state.edges.push(...newEdges);
        });
        return;
      }
    };
    document.addEventListener('keydown', handler);
    return () => document.removeEventListener('keydown', handler);
  }, [removeSelectedElements]);

  const defaultEdgeOptions = useMemo(() => ({
    type: 'bezier' as const,
  }), []);

  const onNodeClick = useCallback(
    (_: React.MouseEvent, node: Node<ShaderNodeData>) => {
      setSelectedNode(node.id);
    },
    [setSelectedNode],
  );

  const onPaneClick = useCallback(() => {
    setSelectedNode(null);
  }, [setSelectedNode]);

  return (
    <ReactFlow
      nodes={nodes}
      edges={edges}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      onConnect={onConnect}
      isValidConnection={isConnectionValid}
      connectionLineComponent={ConnectionLine}
      nodeTypes={nodeTypes}
      edgeTypes={edgeTypes}
      defaultEdgeOptions={defaultEdgeOptions}
      onNodeClick={onNodeClick}
      onPaneClick={onPaneClick}
      selectionMode={SelectionMode.Partial}
      fitView
      className="bg-[#e0e0e0]"
    >
      <Background variant={BackgroundVariant.Cross} color="#c0c0c0" gap={20} size={1.5} bgColor="#e0e0e0" />
      <Controls className="!bg-white !border !border-[#d2d2d7] !rounded-lg !shadow-sm !text-[#1d1d1f]" />
      <MiniMapController />
    </ReactFlow>
  );
}
