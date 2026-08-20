import { describe, expect, it } from 'vitest';
import { OpenQuartzClient, Project } from '../../src/sdk';

const node = {
  id: 'color',
  type: 'shader',
  position: { x: 4, y: 8 },
  data: {
    type: 'shader' as const,
    label: 'Color',
    shaderCode: '',
    inputs: [{ id: 'color', label: 'color', dataType: 'vec4' as const, direction: 'input' as const }],
    outputs: [{ id: 'out', label: 'output', dataType: 'sampler2D' as const, direction: 'output' as const }],
    uniforms: {},
  },
};

describe('TypeScript public object proxies', () => {
  it('projects Project, Graph, Node, and Port identities without exposing transport objects', () => {
    const project = new Project('Proxy', [node], []);
    const projected = project.graph.node('color');

    expect(project.name).toBe('Proxy');
    expect(projected).toMatchObject({ id: 'color', label: 'Color', type: 'shader' });
    expect(projected?.inputs[0]).toMatchObject({
      nodeId: 'color', id: 'color', dataType: 'vec4', direction: 'input',
    });
    expect(projected?.outputs[0]).toMatchObject({
      nodeId: 'color', id: 'out', dataType: 'sampler2D', direction: 'output',
    });
    expect('runtime' in project).toBe(false);
  });

  it('owns graph snapshots and increments graph revisions on atomic replacement', () => {
    const project = new Project('Proxy', [node], []);
    const snapshot = project.graph.snapshot();
    snapshot.nodes[0].data.label = 'Mutated copy';
    expect(project.graph.node('color')?.label).toBe('Color');

    expect(project.graph.replace([], []).revision).toBe(1);
    expect(project.graph.nodes).toEqual([]);
  });

  it('maps stale revisions and restores the prior Rust snapshot on rollback', () => {
    const project = new Project('Revisioned', [node], []);
    expect(project.graph.replace([], [], 0).revision).toBe(1);
    expect(() => project.graph.replace([], [], 0)).toThrowError(
      expect.objectContaining({ code: 'stale-revision' }),
    );

    expect(project.graph.rollback().revision).toBe(2);
    expect(project.graph.node('color')?.label).toBe('Color');
  });

  it('rejects invalid edits through Rust domain validation', () => {
    const project = new Project('Invalid', [node], []);

    expect(() => project.graph.replace([node, node], [])).toThrowError(
      expect.objectContaining({ code: 'invalid-graph' }),
    );
    expect(project.graph.revision).toBe(0);
    expect(project.graph.node('color')?.label).toBe('Color');
  });

  it('applies typed graph commands through the Rust aggregate proxy', () => {
    const project = new Project('Commands');
    const input = project.graph.createNode({
      kind: 'input',
      dataType: 'float',
      inputMode: 'system',
    });
    const math = project.graph.createNode({ kind: 'math', op: 'add' });
    const source = input.outputs[0];
    const target = math.inputs[0];

    expect(project.graph.canConnect(input.id, source.id, math.id, target.id)).toBe(true);
    expect(project.graph.canConnect(input.id, 'missing', math.id, target.id)).toBe(false);

    const connected = project.graph.apply({
      kind: 'connect',
      source: { nodeId: input.id, portId: source.id },
      target: { nodeId: math.id, portId: target.id },
    });
    expect(connected.changedNodes).toEqual([input.id, math.id]);
    expect(project.graph.snapshot().edges).toHaveLength(1);

    expect(() => project.graph.apply({
      kind: 'connect',
      source: { nodeId: input.id, portId: 'missing' },
      target: { nodeId: math.id, portId: target.id },
    })).toThrowError(expect.objectContaining({ code: 'invalid-graph' }));
    expect(project.graph.snapshot().edges).toHaveLength(1);

    project.graph.rollback();
    expect(project.graph.snapshot().edges).toHaveLength(0);
    project.graph.redo();
    expect(project.graph.snapshot().edges).toHaveLength(1);
  });

  it('closes and disposes Rust aggregate handles deterministically', () => {
    const project = new Project('Lifecycle', [node], []);
    project.dispose();
    expect(() => project.toJSON()).toThrowError(expect.objectContaining({ code: 'disposed' }));
    expect(() => project.graph.snapshot()).toThrowError(
      expect.objectContaining({ code: 'disposed' }),
    );
  });

  it('opens and serializes a project through OpenQuartzClient', async () => {
    const source = new Project('Round trip', [node], []);
    const opened = await new OpenQuartzClient().openProject(source.toJSON());

    expect(opened.name).toBe('Round trip');
    expect(opened.graph.node('color')?.label).toBe('Color');
    expect(JSON.parse(opened.toJSON())).toMatchObject({
      version: '0.4.0',
      name: 'Round trip',
    });
  });
});
