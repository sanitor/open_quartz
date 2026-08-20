import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData, ProjectFile } from '../../src/types';
import {
  serializeProject,
  deserializeProject,
  downloadProject,
  saveFileAs,
  saveFile,
} from '../../src/utils/projectIO';

function makeNode(
  id: string,
  type: string,
  position = { x: 0, y: 0 },
  data?: Partial<ShaderNodeData>,
): Node<ShaderNodeData> {
  return {
    id,
    type,
    position,
    data: {
      type: (data?.type ?? 'shader') as ShaderNodeData['type'],
      label: data?.label ?? id,
      shaderCode: data?.shaderCode ?? '',
      inputs: data?.inputs ?? [],
      outputs: data?.outputs ?? [],
      uniforms: data?.uniforms ?? {},
      ...data,
    },
  };
}

function makeEdge(
  id: string,
  source: string,
  target: string,
  sourceHandle?: string | null,
  targetHandle?: string | null,
): Edge {
  return { id, source, target, sourceHandle: sourceHandle ?? null, targetHandle: targetHandle ?? null };
}

function sourceNode(id = 'source', handle = 'out'): Node<ShaderNodeData> {
  return makeNode(id, 'shader', { x: 0, y: 0 }, {
    outputs: [{ id: handle, label: handle, dataType: 'float', direction: 'output' }],
  });
}

function targetNode(id = 'target', handle = 'in'): Node<ShaderNodeData> {
  return makeNode(id, 'shader', { x: 120, y: 0 }, {
    inputs: [{ id: handle, label: handle, dataType: 'float', direction: 'input' }],
  });
}

describe('serializeProject', () => {
  it('creates a valid ProjectFile with version, name, timestamps, graph', () => {
    const nodes = [sourceNode('n1', 'out1'), targetNode('n2', 'in1')];
    const edges = [makeEdge('e1', 'n1', 'n2', 'out1', 'in1')];
    const result = serializeProject(nodes, edges, 'TestProject');

    expect(result.version).toBe('0.4.0');
    expect(result.name).toBe('TestProject');
    expect(result.createdAt).toBe('');
    expect(result.updatedAt).toBe('');
    expect(result.graph.nodes).toHaveLength(2);
    expect(result.graph.edges).toHaveLength(1);
  });

  it('defaults project name to "Untitled"', () => {
    const result = serializeProject([], []);
    expect(result.name).toBe('Untitled');
  });

  it('defaults node type to "shader" when node.type is undefined', () => {
    const node = makeNode('n1', 'shader');
    // Simulate undefined type on the Node object (not ShaderNodeData.type)
    delete (node as Record<string, unknown>).type;
    const result = serializeProject([node], []);
    expect(result.graph.nodes[0].type).toBe('shader');
  });

  it('rejects edges without typed port handles at the Rust boundary', () => {
    const edge = makeEdge('e1', 'source', 'target', null, null);
    expect(() => serializeProject([sourceNode(), targetNode()], [edge])).toThrow();
  });

  it('preserves defined sourceHandle/targetHandle', () => {
    const edge = makeEdge('e1', 'source', 'target', 'out_port', 'in_port');
    const result = serializeProject(
      [sourceNode('source', 'out_port'), targetNode('target', 'in_port')],
      [edge],
    );
    expect(result.graph.edges[0].sourceHandle).toBe('out_port');
    expect(result.graph.edges[0].targetHandle).toBe('in_port');
  });

  it('preserves node position', () => {
    const node = makeNode('n1', 'shader', { x: 42, y: 99 });
    const result = serializeProject([node], []);
    expect(result.graph.nodes[0].position).toEqual({ x: 42, y: 99 });
  });

  it('preserves node data', () => {
    const node = makeNode('n1', 'shader', undefined, {
      label: 'MyShader',
      shaderCode: 'void main() {}',
    });
    const result = serializeProject([node], []);
    expect(result.graph.nodes[0].data.label).toBe('MyShader');
    expect(result.graph.nodes[0].data.shaderCode).toBe('void main() {}');
  });
});

describe('deserializeProject', () => {
  function makeProjectJson(overrides?: Partial<ProjectFile>): string {
    const project: ProjectFile = {
      version: '0.4.0',
      name: 'Test',
      createdAt: '2024-01-01T00:00:00.000Z',
      updatedAt: '2024-01-01T00:00:00.000Z',
      graph: {
        nodes: [
          {
            id: 'n1',
            type: 'shader',
            position: { x: 10, y: 20 },
            data: {
              type: 'shader',
              label: 'n1',
              shaderCode: '',
              inputs: [],
              outputs: [{ id: 'out1', label: 'out1', dataType: 'float', direction: 'output' }],
              uniforms: {},
            },
          },
          {
            id: 'n2',
            type: 'shader',
            position: { x: 30, y: 40 },
            data: {
              type: 'shader',
              label: 'n2',
              shaderCode: '',
              inputs: [{ id: 'in1', label: 'in1', dataType: 'float', direction: 'input' }],
              outputs: [],
              uniforms: {},
            },
          },
        ],
        edges: [
          {
            id: 'e1',
            source: 'n1',
            sourceHandle: 'out1',
            target: 'n2',
            targetHandle: 'in1',
          },
        ],
      },
      ...overrides,
    };
    return JSON.stringify(project);
  }

  it('restores nodes with correct fields', async () => {
    const json = makeProjectJson();
    const { nodes } = await deserializeProject(json);
    expect(nodes).toHaveLength(2);
    expect(nodes[0].id).toBe('n1');
    expect(nodes[0].type).toBe('shader');
    expect(nodes[0].position).toEqual({ x: 10, y: 20 });
    expect(nodes[0].selected).toBe(false);
    expect(nodes[0].dragging).toBe(false);
  });

  it('restores edges with correct fields', async () => {
    const json = makeProjectJson();
    const { edges } = await deserializeProject(json);
    expect(edges).toHaveLength(1);
    expect(edges[0].id).toBe('e1');
    expect(edges[0].source).toBe('n1');
    expect(edges[0].target).toBe('n2');
    expect(edges[0].sourceHandle).toBe('out1');
    expect(edges[0].targetHandle).toBe('in1');
  });

  it('returns the parsed project object', async () => {
    const json = makeProjectJson();
    const { project } = await deserializeProject(json);
    expect(project.version).toBe('0.4.0');
    expect(project.name).toBe('Test');
  });

  it('throws on missing version', async () => {
    const obj = {
      name: 'Bad',
      createdAt: '',
      updatedAt: '',
      graph: { nodes: [], edges: [] },
    };
    await expect(deserializeProject(JSON.stringify(obj))).rejects.toThrow('Cannot decode project');
  });

  it('throws on invalid JSON', async () => {
    await expect(deserializeProject('not json {')).rejects.toThrow();
  });

  it('throws on empty string', async () => {
    await expect(deserializeProject('')).rejects.toThrow();
  });

  it('throws on incompatible version', async () => {
    const obj: ProjectFile = {
      version: '0.1.0',
      name: 'Old',
      createdAt: '',
      updatedAt: '',
      graph: { nodes: [], edges: [] },
    };
    await expect(deserializeProject(JSON.stringify(obj))).rejects.toThrow('Incompatible project version');
  });
});

describe('round-trip serialize → deserialize', () => {
  it('preserves nodes and edges through a round-trip', async () => {
    const nodes = [
      makeNode('s1', 'shader', { x: 100, y: 200 }, {
        label: 'Blur',
        shaderCode: 'uniform float x;',
        inputs: [{ id: 'p1', label: 'x', dataType: 'float', direction: 'input' }],
        outputs: [{ id: 'out_port', label: 'out', dataType: 'float', direction: 'output' }],
      }),
      makeNode('o1', 'shader', { x: 300, y: 200 }, {
        inputs: [{ id: 'in_port', label: 'in', dataType: 'float', direction: 'input' }],
      }),
    ];
    const edges = [makeEdge('e1', 's1', 'o1', 'out_port', 'in_port')];

    const project = serializeProject(nodes, edges, 'RoundTrip');
    const json = JSON.stringify(project);
    const { nodes: restoredNodes, edges: restoredEdges, project: restoredProject } =
      await deserializeProject(json);

    expect(restoredProject.name).toBe('RoundTrip');
    expect(restoredNodes).toHaveLength(2);
    expect(restoredNodes[0].id).toBe('s1');
    expect(restoredNodes[0].position).toEqual({ x: 100, y: 200 });
    expect(restoredNodes[0].data.label).toBe('Blur');
    expect(restoredEdges).toHaveLength(1);
    expect(restoredEdges[0].source).toBe('s1');
    expect(restoredEdges[0].target).toBe('o1');
    expect(restoredEdges[0].sourceHandle).toBe('out_port');
    expect(restoredEdges[0].targetHandle).toBe('in_port');
  });
});

describe('downloadProject', () => {
  let mockAnchor: {
    href: string;
    download: string;
    click: ReturnType<typeof vi.fn>;
  };

  beforeEach(() => {
    mockAnchor = { href: '', download: '', click: vi.fn() };
    vi.spyOn(document, 'createElement').mockReturnValue(mockAnchor as unknown as HTMLElement);
    vi.spyOn(document.body, 'appendChild').mockImplementation((el) => el);
    vi.spyOn(document.body, 'removeChild').mockImplementation((el) => el);
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock-url-123');
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
  });

  it('creates an anchor, clicks it, and cleans up', () => {
    const project = serializeProject([], [], 'Test');
    downloadProject(project);

    expect(document.createElement).toHaveBeenCalledWith('a');
    expect(mockAnchor.href).toBe('blob:mock-url-123');
    expect(mockAnchor.download).toBe('Test.quartz.json');
    expect(document.body.appendChild).toHaveBeenCalledWith(mockAnchor);
    expect(mockAnchor.click).toHaveBeenCalled();
    expect(document.body.removeChild).toHaveBeenCalledWith(mockAnchor);
    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:mock-url-123');
  });

  it('uses custom filename when provided', () => {
    const project = serializeProject([], [], 'Test');
    const name = downloadProject(project, 'custom.json');
    expect(mockAnchor.download).toBe('custom.json');
    expect(name).toBe('custom.json');
  });

  it('returns the download filename', () => {
    const project = serializeProject([], [], 'MyProject');
    const name = downloadProject(project);
    expect(name).toBe('MyProject.quartz.json');
  });
});

describe('saveFileAs / saveFile', () => {
  beforeEach(() => {
    const mockAnchor = { href: '', download: '', click: vi.fn() };
    vi.spyOn(document, 'createElement').mockReturnValue(mockAnchor as unknown as HTMLElement);
    vi.spyOn(document.body, 'appendChild').mockImplementation((el) => el);
    vi.spyOn(document.body, 'removeChild').mockImplementation((el) => el);
    vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:mock');
    vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {});
  });

  it('saveFileAs delegates to downloadProject', () => {
    const project = serializeProject([], [], 'Test');
    expect(() => saveFileAs(project, 'out.json')).not.toThrow();
    expect(URL.createObjectURL).toHaveBeenCalled();
  });

  it('saveFile delegates to downloadProject', () => {
    const project = serializeProject([], [], 'Test');
    expect(() => saveFile(project, 'out.json')).not.toThrow();
    expect(URL.createObjectURL).toHaveBeenCalled();
  });
});
