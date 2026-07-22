import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { Node, Edge, NodeChange, EdgeChange, Connection } from '@xyflow/react';
import type { ShaderNodeData, DataType } from '../../src/types';

// Mock @xyflow/react before importing the store
vi.mock('@xyflow/react', () => ({
  applyNodeChanges: (changes: NodeChange[], nodes: Node[]) => {
    const removeIds = new Set(
      changes.filter((c): c is NodeChange & { type: 'remove'; id: string } => c.type === 'remove').map(c => c.id),
    );
    if (removeIds.size > 0) return nodes.filter(n => !removeIds.has(n.id));
    return [...nodes];
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
import { MATH_OPS, getMathPorts } from '../../src/catalog/mathOps';

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

describe('Math and System nodes in useGraphStore', () => {
  beforeEach(() => {
    resetStore();
  });

  describe('addMathNode', () => {
    it('creates a node with type math and the requested mathOp', () => {
      useGraphStore.getState().addMathNode('add');
      const nodes = useGraphStore.getState().nodes;
      expect(nodes).toHaveLength(1);
      const node = nodes[0];
      expect(node.data.type).toBe('math');
      expect(node.type).toBe('math');
      expect(node.data.mathOp).toBe('add');
    });

    it('sets the label from MATH_OPS definition', () => {
      useGraphStore.getState().addMathNode('multiply');
      const node = useGraphStore.getState().nodes[0];
      expect(node.data.templateName).toBe(MATH_OPS['multiply'].label);
      expect(node.data.label).toMatch(/^multiply_\d+$/);
    });

    it.each([
      { op: 'add', expectedInputCount: 2 },
      { op: 'negate', expectedInputCount: 1 },
      { op: 'clamp', expectedInputCount: 3 },
    ])('creates correct port count for $op (inputCount=$expectedInputCount)', ({ op, expectedInputCount }) => {
      useGraphStore.getState().addMathNode(op);
      const node = useGraphStore.getState().nodes[0];
      expect(node.data.inputs).toHaveLength(expectedInputCount);
      expect(node.data.outputs).toHaveLength(1);
    });

    it('prefixes port ids with the node id', () => {
      useGraphStore.getState().addMathNode('add');
      const node = useGraphStore.getState().nodes[0];
      const nodeId = node.id;
      for (const port of node.data.inputs) {
        expect(port.id).toMatch(new RegExp(`^${nodeId}_`));
      }
      for (const port of node.data.outputs) {
        expect(port.id).toMatch(new RegExp(`^${nodeId}_`));
      }
    });

    it('preserves port labels and dataType from getMathPorts', () => {
      useGraphStore.getState().addMathNode('mix');
      const node = useGraphStore.getState().nodes[0];
      const expectedPorts = getMathPorts(MATH_OPS['mix']);
      // Input labels match
      expect(node.data.inputs.map(p => p.label)).toEqual(expectedPorts.inputs.map(p => p.label));
      // All ports are auto-typed
      for (const port of [...node.data.inputs, ...node.data.outputs]) {
        expect(port.dataType).toBe('auto');
      }
      // Output label
      expect(node.data.outputs[0].label).toBe('result');
    });

    it('does not create a node for an unknown mathOp', () => {
      useGraphStore.getState().addMathNode('nonexistent_op_xyz');
      expect(useGraphStore.getState().nodes).toHaveLength(0);
    });

    it('assigns distinct ids to consecutively added math nodes', () => {
      useGraphStore.getState().addMathNode('add');
      useGraphStore.getState().addMathNode('sin');
      const nodes = useGraphStore.getState().nodes;
      expect(nodes).toHaveLength(2);
      expect(nodes[0].id).not.toBe(nodes[1].id);
    });
  });

  describe('addSystemNode', () => {
    const SYSTEM_EXPECTATIONS: {
      source: NonNullable<ShaderNodeData['systemSource']>;
      dataType: DataType;
    }[] = [
      { source: 'time', dataType: 'float' },
      { source: 'timeDelta', dataType: 'float' },
      { source: 'frame', dataType: 'int' },
      { source: 'mouse', dataType: 'vec4' },
      { source: 'resolution', dataType: 'vec3' },
    ];

    it.each(SYSTEM_EXPECTATIONS)(
      'creates a system input node for $source with dataType $dataType',
      ({ source, dataType }) => {
        useGraphStore.getState().addSystemNode(source);
        const nodes = useGraphStore.getState().nodes;
        expect(nodes).toHaveLength(1);
        const node = nodes[0];
        expect(node.data.type).toBe('input');
        expect(node.data.inputMode).toBe('system');
        expect(node.data.systemSource).toBe(source);
        expect(node.data.inputDataType).toBe(dataType);
      },
    );

    it('creates all 5 system source types without collision', () => {
      const sources: NonNullable<ShaderNodeData['systemSource']>[] = [
        'time', 'timeDelta', 'frame', 'mouse', 'resolution',
      ];
      for (const src of sources) {
        useGraphStore.getState().addSystemNode(src);
      }
      const nodes = useGraphStore.getState().nodes;
      expect(nodes).toHaveLength(5);
      const ids = new Set(nodes.map(n => n.id));
      expect(ids.size).toBe(5);
    });
  });

  describe('onConnect auto-type rules', () => {
    function setupConnectionTest(
      sourceType: ShaderNodeData['type'],
      sourceOutputDataType: DataType,
      targetType: ShaderNodeData['type'],
      targetInputDataType: DataType,
      sourceInputDataType?: DataType,
    ): { connection: Connection } {
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
        connection: {
          source: 'src_1',
          target: 'tgt_1',
          sourceHandle: 'out_p',
          targetHandle: 'in_p',
        },
      };
    }

    it.each([
      { srcType: 'float' as DataType, desc: 'float→auto' },
      { srcType: 'vec3' as DataType, desc: 'vec3→auto' },
      { srcType: 'int' as DataType, desc: 'int→auto' },
    ])('allows $desc connections', ({ srcType }) => {
      const { connection } = setupConnectionTest('shader', srcType, 'math', 'auto');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(1);
    });

    it('allows auto→float connections', () => {
      const { connection } = setupConnectionTest('math', 'auto', 'shader', 'float');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(1);
    });

    it('allows auto→auto connections', () => {
      const { connection } = setupConnectionTest('math', 'auto', 'math', 'auto');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(1);
    });

    it('rejects sampler2D→auto connections', () => {
      const { connection } = setupConnectionTest('input', 'sampler2D', 'math', 'auto', 'sampler2D');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(0);
    });

    it('rejects auto→sampler2D connections', () => {
      const { connection } = setupConnectionTest('math', 'auto', 'shader', 'sampler2D');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(0);
    });

    it('still allows float→float (existing behavior)', () => {
      const { connection } = setupConnectionTest('shader', 'float', 'shader', 'float');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(1);
    });

    it('still rejects float→int (existing behavior)', () => {
      const { connection } = setupConnectionTest('shader', 'float', 'shader', 'int');
      useGraphStore.getState().onConnect(connection);
      expect(useGraphStore.getState().edges).toHaveLength(0);
    });
  });
});
