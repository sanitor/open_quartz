import { useCallback, useMemo } from 'react';
import {
  ReactFlow,
  Background,
  BackgroundVariant,
  Controls,
  MiniMap,
  type Node,
  type NodeTypes,
  SelectionMode,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { useGraphStore } from '../../store/useGraphStore';
import type { ShaderNodeData } from '../../types';
import { ShaderNode } from './nodes/ShaderNode';
import { InputNode } from './nodes/InputNode';
import { OutputNode } from './nodes/OutputNode';

const nodeTypes: NodeTypes = {
  shader: ShaderNode,
  input: InputNode,
  output: OutputNode,
  constant: ShaderNode,
};

export function NodeGraph() {
  const {
    nodes,
    edges,
    onNodesChange,
    onEdgesChange,
    onConnect,
    setSelectedNode,
  } = useGraphStore();

  const defaultEdgeOptions = useMemo(() => ({
    style: { stroke: '#8e8e93', strokeWidth: 1.5 },
    type: 'bezier',
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
      nodeTypes={nodeTypes}
      defaultEdgeOptions={defaultEdgeOptions}
      onNodeClick={onNodeClick}
      onPaneClick={onPaneClick}
      selectionMode={SelectionMode.Partial}
      fitView
      className="bg-[#e0e0e0]"
    >
      <Background variant={BackgroundVariant.Cross} color="#c0c0c0" gap={20} size={1.5} bgColor="#e0e0e0" />
      <Controls className="!bg-white !border !border-[#d2d2d7] !rounded-lg !shadow-sm !text-[#1d1d1f]" />
      <MiniMap
        className="!bg-[#00000066] !border !border-[#ffffff33] !rounded-lg !shadow-sm"
        nodeColor="#d0d0d0"
        maskColor="rgba(0,0,0,0.3)"
      />
    </ReactFlow>
  );
}
