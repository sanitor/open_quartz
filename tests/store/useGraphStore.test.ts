import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { Node, Edge, NodeChange, EdgeChange, Connection } from '@xyflow/react';
import type { ShaderNodeData, DataType } from '../../src/types';

// Mock @xyflow/react before importing the store
vi.mock('@xyflow/react', () => ({
  applyNodeChanges: (changes: NodeChange[], nodes: Node[]) => {
    // Simple mock: apply 'remove' changes, pass through others
    const removeIds = new Set(
      changes.filter((c): c is NodeChange & { type: 'remove'; id: string } => c.type === 'remove').map(c => c.id),
    );
    if (removeIds.size > 0) return nodes.filter(n => !removeIds.has(n.id));
    return [...nodes]; // Return a copy for position/select changes
  },
  applyEdgeChanges: (changes: EdgeChange[], edges: Edge[]) => {
    const removeIds = new Set(
      changes.filter((c): c is EdgeChange & { type: 'remove'; id: string } => c.type === 'remove').map(c => c.id),
    );
    if (removeIds.size > 0) return edges.filter(e => !removeIds.has(e.id));
    return [...edges];
  },
  addEdge: (connection: Connection & { type?: string }, edges: Edge[]) => {
    const newEdge: Edge = {
      id: `e_${connection.source}_${connection.target}`,
      source: connection.source!,
      target: connection.target!,
      sourceHandle: connection.sourceHandle ?? null,
      targetHandle: connection.targetHandle ?? null,
      type: connection.type,
    };
    return [...edges, newEdge];
  },
}));

import { useGraphStore } from '../../src/store/useGraphStore';

function resetStore() {
  useGraphStore.setState({
    nodes: [],
    edges: [],
    selectedNodeId: null,
    loopState: 'stopped' as const,
    projectName: 'Untitled',
    savedFilePath: null,
    outputPreviews: {},
    nodeErrors: {},
    undoStack: [],
    redoStack: [],
  });
}

describe('useGraphStore', () => {
  beforeEach(() => {
    resetStore();
  });

  describe('initial state', () => {
    it('has empty nodes and edges', () => {
      const { nodes, edges } = useGraphStore.getState();
      expect(nodes).toEqual([]);
      expect(edges).toEqual([]);
    });

    it('has null selectedNodeId', () => {
      expect(useGraphStore.getState().selectedNodeId).toBeNull();
    });

    it('has projectName "Untitled"', () => {
      expect(useGraphStore.getState().projectName).toBe('Untitled');
    });

    it('has loopState "stopped"', () => {
      expect(useGraphStore.getState().loopState).toBe('stopped');
    });

    it('has null savedFilePath', () => {
      expect(useGraphStore.getState().savedFilePath).toBeNull();
    });

    it('has empty outputPreviews and nodeErrors', () => {
      expect(useGraphStore.getState().outputPreviews).toEqual({});
      expect(useGraphStore.getState().nodeErrors).toEqual({});
    });

    it('has empty undo/redo stacks', () => {
      expect(useGraphStore.getState().undoStack).toEqual([]);
      expect(useGraphStore.getState().redoStack).toEqual([]);
    });
  });

  describe('addNode', () => {
    it('adds a shader node with parsed ports', () => {
      useGraphStore.getState().addNode('shader');
      const { nodes } = useGraphStore.getState();
      expect(nodes).toHaveLength(1);
      expect(nodes[0].type).toBe('shader');
      expect(nodes[0].data.type).toBe('shader');
      // Default shader code has uniform sampler2D inputImage and uniform float intensity → 2 inputs
      expect(nodes[0].data.inputs.length).toBeGreaterThanOrEqual(1);
      // Default shader code has "out vec4 fragColor" → 1 output
      expect(nodes[0].data.outputs.length).toBeGreaterThanOrEqual(1);
      expect(nodes[0].data.uniforms).toEqual({});
    });

    it('adds a shader node at custom position', () => {
      useGraphStore.getState().addNode('shader', { x: 42, y: 99 });
      const { nodes } = useGraphStore.getState();
      expect(nodes[0].position).toEqual({ x: 42, y: 99 });
    });

    it('adds an input node', () => {
      useGraphStore.getState().addNode('input');
      const { nodes } = useGraphStore.getState();
      expect(nodes).toHaveLength(1);
      expect(nodes[0].type).toBe('input');
      expect(nodes[0].data.type).toBe('input');
    });


    it('pushes history when adding a node', () => {
      useGraphStore.getState().addNode('shader');
      expect(useGraphStore.getState().undoStack.length).toBe(1);
    });
  });

  describe('addInputNode', () => {
    it('creates a float input node', () => {
      useGraphStore.getState().addInputNode('float');
      const node = useGraphStore.getState().nodes[0];
      expect(node.data.inputDataType).toBe('float');
      expect(node.data.type).toBe('input');
    });

    it('creates a vec2 input node', () => {
      useGraphStore.getState().addInputNode('vec2');
      const node = useGraphStore.getState().nodes[0];
      expect(node.data.inputDataType).toBe('vec2');
    });

    it('creates a sampler2D input node with inputMode', () => {
      useGraphStore.getState().addInputNode('sampler2D', undefined, 'image');
      const node = useGraphStore.getState().nodes[0];
      expect(node.data.inputDataType).toBe('sampler2D');
      expect(node.data.inputMode).toBe('image');
    });

    it('creates a sampler2D input node with framebuffer mode', () => {
      useGraphStore.getState().addInputNode('sampler2D', undefined, 'framebuffer');
      const node = useGraphStore.getState().nodes[0];
      expect(node.data.inputMode).toBe('framebuffer');
    });

    it('sampler2D input defaults inputMode to "image"', () => {
      useGraphStore.getState().addInputNode('sampler2D');
      const node = useGraphStore.getState().nodes[0];
      expect(node.data.inputMode).toBe('image');
    });

    it('non-sampler2D input has no inputMode', () => {
      useGraphStore.getState().addInputNode('float');
      const node = useGraphStore.getState().nodes[0];
      expect(node.data.inputMode).toBeUndefined();
    });

    it('uses custom position', () => {
      useGraphStore.getState().addInputNode('float', { x: 10, y: 20 });
      expect(useGraphStore.getState().nodes[0].position).toEqual({ x: 10, y: 20 });
    });
  });

  describe('addShaderNode', () => {
    it('creates a shader node with custom code and label', () => {
      const code = 'uniform float x;\nout vec4 fragColor;\nvoid main() { fragColor = vec4(x); }';
      useGraphStore.getState().addShaderNode(code, 'Custom');
      const node = useGraphStore.getState().nodes[0];
      expect(node.data.label).toBe('Custom');
      expect(node.data.shaderCode).toBe(code);
      expect(node.data.type).toBe('shader');
      // Should parse the uniform
      expect(node.data.inputs.some(p => p.label === 'x')).toBe(true);
    });

    it('uses custom position', () => {
      useGraphStore.getState().addShaderNode('void main(){}', 'Test', { x: 5, y: 15 });
      expect(useGraphStore.getState().nodes[0].position).toEqual({ x: 5, y: 15 });
    });
  });

  describe('removeNode', () => {
    it('removes a node and associated edges', () => {
      useGraphStore.getState().addNode('shader');
      useGraphStore.getState().addNode('constant');
      const [shader, output] = useGraphStore.getState().nodes;

      // Manually add an edge
      useGraphStore.setState(state => ({
        edges: [...state.edges, {
          id: 'e1', source: shader.id, target: output.id,
          sourceHandle: null, targetHandle: null,
        }],
      }));
      expect(useGraphStore.getState().edges).toHaveLength(1);

      useGraphStore.getState().removeNode(shader.id);
      expect(useGraphStore.getState().nodes.every(n => n.id !== shader.id)).toBe(true);
      expect(useGraphStore.getState().edges).toHaveLength(0);
    });

    it('clears selectedNodeId if removed node was selected', () => {
      useGraphStore.getState().addNode('shader');
      const id = useGraphStore.getState().nodes[0].id;
      useGraphStore.getState().setSelectedNode(id);
      expect(useGraphStore.getState().selectedNodeId).toBe(id);

      useGraphStore.getState().removeNode(id);
      expect(useGraphStore.getState().selectedNodeId).toBeNull();
    });

    it('does not affect selectedNodeId if a different node is removed', () => {
      useGraphStore.getState().addNode('shader');
      useGraphStore.getState().addNode('constant');
      const [n1, n2] = useGraphStore.getState().nodes;
      useGraphStore.getState().setSelectedNode(n1.id);

      useGraphStore.getState().removeNode(n2.id);
      expect(useGraphStore.getState().selectedNodeId).toBe(n1.id);
    });
  });

  describe('removeSelectedElements', () => {
    it('removes selected nodes and their edges', () => {
      useGraphStore.getState().addNode('shader');
      useGraphStore.getState().addNode('constant');
      const nodes = useGraphStore.getState().nodes;

      // Select the first node
      useGraphStore.setState(state => ({
        nodes: state.nodes.map((n, i) => i === 0 ? { ...n, selected: true } : n),
        edges: [{
          id: 'e1', source: nodes[0].id, target: nodes[1].id,
          sourceHandle: null, targetHandle: null,
        }],
      }));

      useGraphStore.getState().removeSelectedElements();
      expect(useGraphStore.getState().nodes).toHaveLength(1);
      expect(useGraphStore.getState().edges).toHaveLength(0);
    });

    it('removes selected edges', () => {
      useGraphStore.setState({
        edges: [
          { id: 'e1', source: 'a', target: 'b', sourceHandle: null, targetHandle: null, selected: true },
          { id: 'e2', source: 'b', target: 'c', sourceHandle: null, targetHandle: null },
        ],
      });

      useGraphStore.getState().removeSelectedElements();
      expect(useGraphStore.getState().edges).toHaveLength(1);
      expect(useGraphStore.getState().edges[0].id).toBe('e2');
    });

    it('does nothing when nothing is selected', () => {
      useGraphStore.getState().addNode('shader');
      const undoLengthBefore = useGraphStore.getState().undoStack.length;

      useGraphStore.getState().removeSelectedElements();
      // No history pushed because nothing was selected
      expect(useGraphStore.getState().undoStack.length).toBe(undoLengthBefore);
    });
  });

  describe('updateNodeData', () => {
    it('with shaderCode triggers re-parse', () => {
      useGraphStore.getState().addNode('shader');
      const id = useGraphStore.getState().nodes[0].id;

      const newCode = 'uniform vec3 color;\nout vec4 fragColor;\nvoid main() { fragColor = vec4(color, 1.0); }';
      useGraphStore.getState().updateNodeData(id, { shaderCode: newCode });

      const node = useGraphStore.getState().nodes.find(n => n.id === id)!;
      expect(node.data.shaderCode).toBe(newCode);
      expect(node.data.inputs.some(p => p.label === 'color' && p.dataType === 'vec3')).toBe(true);
    });

    it('without shaderCode does Object.assign', () => {
      useGraphStore.getState().addNode('shader');
      const id = useGraphStore.getState().nodes[0].id;

      useGraphStore.getState().updateNodeData(id, { label: 'NewLabel' });
      const node = useGraphStore.getState().nodes.find(n => n.id === id)!;
      expect(node.data.label).toBe('NewLabel');
    });

    it('pushes history only when shaderCode is provided', () => {
      useGraphStore.getState().addNode('shader');
      const id = useGraphStore.getState().nodes[0].id;
      const stackBefore = useGraphStore.getState().undoStack.length;

      useGraphStore.getState().updateNodeData(id, { label: 'X' });
      expect(useGraphStore.getState().undoStack.length).toBe(stackBefore);

      useGraphStore.getState().updateNodeData(id, { shaderCode: 'void main(){}' });
      expect(useGraphStore.getState().undoStack.length).toBe(stackBefore + 1);
    });

    it('does nothing for a non-existent node', () => {
      useGraphStore.getState().addNode('shader');
      const nodesBefore = useGraphStore.getState().nodes.map(n => ({ ...n }));
      useGraphStore.getState().updateNodeData('nonexistent', { label: 'X' });
      expect(useGraphStore.getState().nodes).toHaveLength(nodesBefore.length);
    });
  });

  describe('updateNodeInputType', () => {
    it('changes type and regenerates shader code', () => {
      useGraphStore.getState().addInputNode('float');
      const id = useGraphStore.getState().nodes[0].id;

      useGraphStore.getState().updateNodeInputType(id, 'vec2');
      const node = useGraphStore.getState().nodes.find(n => n.id === id)!;
      expect(node.data.inputDataType).toBe('vec2');
      expect(node.data.shaderCode).toContain('vec2');
      expect(node.data.uniforms).toEqual({});
    });

    it('does nothing for non-input nodes', () => {
      useGraphStore.getState().addNode('shader');
      const id = useGraphStore.getState().nodes[0].id;
      const codeBefore = useGraphStore.getState().nodes[0].data.shaderCode;

      useGraphStore.getState().updateNodeInputType(id, 'vec3');
      expect(useGraphStore.getState().nodes[0].data.shaderCode).toBe(codeBefore);
    });

    it('generates sampler2D shader when changing to sampler2D', () => {
      useGraphStore.getState().addInputNode('float');
      const id = useGraphStore.getState().nodes[0].id;

      useGraphStore.getState().updateNodeInputType(id, 'sampler2D');
      const node = useGraphStore.getState().nodes.find(n => n.id === id)!;
      expect(node.data.inputDataType).toBe('sampler2D');
      expect(node.data.shaderCode).toContain('sampler2D');
    });
  });

  describe('simple setters', () => {
    it('setSelectedNode', () => {
      useGraphStore.getState().setSelectedNode('node1');
      expect(useGraphStore.getState().selectedNodeId).toBe('node1');

      useGraphStore.getState().setSelectedNode(null);
      expect(useGraphStore.getState().selectedNodeId).toBeNull();
    });

    it('play / stop loopState transitions', () => {
      useGraphStore.getState().play();
      expect(useGraphStore.getState().loopState).toBe('playing');

      useGraphStore.getState().stop();
      expect(useGraphStore.getState().loopState).toBe('stopped');
    });

    it('setProjectName', () => {
      useGraphStore.getState().setProjectName('MyProject');
      expect(useGraphStore.getState().projectName).toBe('MyProject');
    });

    it('setSavedFilePath', () => {
      useGraphStore.getState().setSavedFilePath('/tmp/test.json');
      expect(useGraphStore.getState().savedFilePath).toBe('/tmp/test.json');

      useGraphStore.getState().setSavedFilePath(null);
      expect(useGraphStore.getState().savedFilePath).toBeNull();
    });
  });

  describe('setOutputPreview / clearOutputPreviews', () => {
    it('sets a preview for a node', () => {
      useGraphStore.getState().setOutputPreview('node1', 'data:image/png;base64,ABC');
      expect(useGraphStore.getState().outputPreviews['node1']).toBe('data:image/png;base64,ABC');
    });

    it('overwrites existing preview', () => {
      useGraphStore.getState().setOutputPreview('node1', 'old');
      useGraphStore.getState().setOutputPreview('node1', 'new');
      expect(useGraphStore.getState().outputPreviews['node1']).toBe('new');
    });

    it('clearOutputPreviews removes all', () => {
      useGraphStore.getState().setOutputPreview('node1', 'a');
      useGraphStore.getState().setOutputPreview('node2', 'b');
      useGraphStore.getState().clearOutputPreviews();
      expect(useGraphStore.getState().outputPreviews).toEqual({});
    });
  });

  describe('setNodeError / clearNodeErrors', () => {
    it('sets an error for a node', () => {
      useGraphStore.getState().setNodeError('node1', 'Shader compile failed');
      expect(useGraphStore.getState().nodeErrors['node1']).toBe('Shader compile failed');
    });

    it('clears a specific error with null', () => {
      useGraphStore.getState().setNodeError('node1', 'err');
      useGraphStore.getState().setNodeError('node1', null);
      expect(useGraphStore.getState().nodeErrors['node1']).toBeUndefined();
    });

    it('clearNodeErrors removes all errors', () => {
      useGraphStore.getState().setNodeError('node1', 'err1');
      useGraphStore.getState().setNodeError('node2', 'err2');
      useGraphStore.getState().clearNodeErrors();
      expect(useGraphStore.getState().nodeErrors).toEqual({});
    });
  });

  describe('pushHistory / undo / redo', () => {
    it('pushHistory saves current state to undoStack', () => {
      useGraphStore.getState().addNode('shader');
      // addNode already pushes history; undoStack should have the empty-state snapshot
      expect(useGraphStore.getState().undoStack.length).toBeGreaterThanOrEqual(1);
    });

    it('undo restores previous state', () => {
      useGraphStore.getState().addNode('shader');
      expect(useGraphStore.getState().nodes).toHaveLength(1);

      useGraphStore.getState().undo();
      expect(useGraphStore.getState().nodes).toHaveLength(0);
    });

    it('redo restores undone state', () => {
      useGraphStore.getState().addNode('shader');
      const nodeId = useGraphStore.getState().nodes[0].id;

      useGraphStore.getState().undo();
      expect(useGraphStore.getState().nodes).toHaveLength(0);

      useGraphStore.getState().redo();
      expect(useGraphStore.getState().nodes).toHaveLength(1);
      expect(useGraphStore.getState().nodes[0].id).toBe(nodeId);
    });

    it('undo with empty stack does nothing', () => {
      const stateBefore = { ...useGraphStore.getState() };
      useGraphStore.getState().undo();
      expect(useGraphStore.getState().nodes).toEqual(stateBefore.nodes);
      expect(useGraphStore.getState().edges).toEqual(stateBefore.edges);
    });

    it('redo with empty stack does nothing', () => {
      const stateBefore = { ...useGraphStore.getState() };
      useGraphStore.getState().redo();
      expect(useGraphStore.getState().nodes).toEqual(stateBefore.nodes);
      expect(useGraphStore.getState().edges).toEqual(stateBefore.edges);
    });

    it('undo clears selectedNodeId', () => {
      useGraphStore.getState().addNode('shader');
      useGraphStore.getState().setSelectedNode(useGraphStore.getState().nodes[0].id);
      useGraphStore.getState().undo();
      expect(useGraphStore.getState().selectedNodeId).toBeNull();
    });

    it('redo clears selectedNodeId', () => {
      useGraphStore.getState().addNode('shader');
      useGraphStore.getState().setSelectedNode(useGraphStore.getState().nodes[0].id);
      useGraphStore.getState().undo();
      useGraphStore.getState().redo();
      expect(useGraphStore.getState().selectedNodeId).toBeNull();
    });

    it('undo pushes to redoStack', () => {
      useGraphStore.getState().addNode('shader');
      useGraphStore.getState().undo();
      expect(useGraphStore.getState().redoStack.length).toBeGreaterThanOrEqual(1);
    });

    it('new action after undo clears redoStack', () => {
      useGraphStore.getState().addNode('shader');
      useGraphStore.getState().undo();
      expect(useGraphStore.getState().redoStack.length).toBeGreaterThanOrEqual(1);

      useGraphStore.getState().addNode('constant');
      expect(useGraphStore.getState().redoStack).toHaveLength(0);
    });
  });

  describe('loadGraph', () => {
    it('sets nodes and edges', () => {
      const nodes: Node<ShaderNodeData>[] = [{
        id: 'shader_5',
        type: 'shader',
        position: { x: 100, y: 200 },
        data: {
          type: 'shader', label: 'Test', shaderCode: '', inputs: [], outputs: [],
          uniforms: {},
        },
      }];
      const edges: Edge[] = [{
        id: 'e1', source: 'shader_5', target: 'out_1',
        sourceHandle: 'h1', targetHandle: 'h2',
      }];

      useGraphStore.getState().loadGraph(nodes, edges);
      expect(useGraphStore.getState().nodes).toHaveLength(1);
      expect(useGraphStore.getState().edges).toHaveLength(1);
      // loadGraph adds type: 'bezier' to edges
      expect(useGraphStore.getState().edges[0].type).toBe('bezier');
    });

    it('resets selectedNodeId', () => {
      useGraphStore.getState().setSelectedNode('old');
      useGraphStore.getState().loadGraph([], []);
      expect(useGraphStore.getState().selectedNodeId).toBeNull();
    });

    it('pushes history before loading', () => {
      useGraphStore.getState().loadGraph([], []);
      expect(useGraphStore.getState().undoStack.length).toBeGreaterThanOrEqual(1);
    });
  });

  describe('clearGraph', () => {
    it('resets nodes, edges, selectedNodeId, outputPreviews, savedFilePath, projectName', () => {
      useGraphStore.getState().addNode('shader');
      useGraphStore.getState().setProjectName('Test');
      useGraphStore.getState().setSavedFilePath('/tmp/test.json');
      useGraphStore.getState().setOutputPreview('n1', 'data');
      useGraphStore.getState().setSelectedNode('n1');

      useGraphStore.getState().clearGraph();
      const state = useGraphStore.getState();
      expect(state.nodes).toEqual([]);
      expect(state.edges).toEqual([]);
      expect(state.selectedNodeId).toBeNull();
      expect(state.outputPreviews).toEqual({});
      expect(state.savedFilePath).toBeNull();
      expect(state.projectName).toBe('Untitled');
    });

    it('pushes history before clearing', () => {
      useGraphStore.getState().addNode('shader');
      const undoLen = useGraphStore.getState().undoStack.length;
      useGraphStore.getState().clearGraph();
      expect(useGraphStore.getState().undoStack.length).toBeGreaterThan(undoLen);
    });
  });

  describe('onConnect type checking', () => {
    function setupConnectionTest(
      sourceType: ShaderNodeData['type'],
      sourceOutputDataType: DataType,
      targetType: ShaderNodeData['type'],
      targetInputDataType: DataType,
      sourceInputDataType?: DataType,
    ): { sourceId: string; targetId: string; connection: Connection } {
      const sourceNode: Node<ShaderNodeData> = {
        id: 'src_1', type: sourceType, position: { x: 0, y: 0 },
        data: {
          type: sourceType, label: 'Source', shaderCode: '',
          inputs: [],
          outputs: [{ id: 'out_p', label: 'out', dataType: sourceOutputDataType, direction: 'output' }],
          uniforms: {},
          inputDataType: sourceInputDataType,
        },
      };
      const targetNode: Node<ShaderNodeData> = {
        id: 'tgt_1', type: targetType, position: { x: 200, y: 0 },
        data: {
          type: targetType, label: 'Target', shaderCode: '',
          inputs: [{ id: 'in_p', label: 'in', dataType: targetInputDataType, direction: 'input' }],
          outputs: [],
          uniforms: {},
        },
      };
      useGraphStore.setState({ nodes: [sourceNode, targetNode], edges: [] });

      return {
        sourceId: 'src_1',
        targetId: 'tgt_1',
        connection: {
          source: 'src_1',
          target: 'tgt_1',
          sourceHandle: 'out_p',
          targetHandle: 'in_p',
        },
      };
    }

    it('allows sampler2D target from a shader source', () => {
      const { connection } = setupConnectionTest('shader', 'vec4', 'shader', 'sampler2D');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(1);
    });


    it('allows sampler2D target from an input-sampler2D source', () => {
      const { connection } = setupConnectionTest('input', 'sampler2D', 'shader', 'sampler2D', 'sampler2D');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(1);
    });

    it('rejects sampler2D target from a non-texture-producer input (float)', () => {
      const { connection } = setupConnectionTest('input', 'float', 'shader', 'sampler2D', 'float');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(0);
    });

    it('allows matching non-sampler types', () => {
      const { connection } = setupConnectionTest('shader', 'float', 'shader', 'float');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(1);
    });

    it('rejects mismatched non-sampler types', () => {
      const { connection } = setupConnectionTest('shader', 'float', 'shader', 'vec3');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(0);
    });

    it('allows connection when source/target nodes are not found (no type check)', () => {
      useGraphStore.setState({ nodes: [], edges: [] });
      const connection: Connection = {
        source: 'nonexistent_src',
        target: 'nonexistent_tgt',
        sourceHandle: 'h1',
        targetHandle: 'h2',
      };
      useGraphStore.getState().onConnect(connection);
      // When nodes not found, the type check is skipped and connection proceeds
      expect(useGraphStore.getState().edges).toHaveLength(1);
    });

    it('allows connection when ports are not found (no type check)', () => {
      const { connection } = setupConnectionTest('shader', 'float', 'shader', 'float');
      // Use handles that don't match any port
      connection.sourceHandle = 'nonexistent';
      connection.targetHandle = 'nonexistent';
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(1);
    });
  });

  describe('onNodesChange / onEdgesChange', () => {
    it('onNodesChange applies node changes', () => {
      useGraphStore.getState().addNode('shader');
      const id = useGraphStore.getState().nodes[0].id;

      useGraphStore.getState().onNodesChange([{ type: 'remove', id }]);
      expect(useGraphStore.getState().nodes).toHaveLength(0);
    });

    it('onEdgesChange applies edge changes', () => {
      useGraphStore.setState({
        edges: [{ id: 'e1', source: 'a', target: 'b', sourceHandle: null, targetHandle: null }],
      });

      useGraphStore.getState().onEdgesChange([{ type: 'remove', id: 'e1' }]);
      expect(useGraphStore.getState().edges).toHaveLength(0);
    });

    it('onNodesChange with non-remove change preserves nodes', () => {
      useGraphStore.getState().addNode('shader');
      const nodeCount = useGraphStore.getState().nodes.length;

      useGraphStore.getState().onNodesChange([{
        type: 'position',
        id: useGraphStore.getState().nodes[0].id,
        position: { x: 500, y: 500 },
      }]);
      expect(useGraphStore.getState().nodes).toHaveLength(nodeCount);
    });
  });
});
