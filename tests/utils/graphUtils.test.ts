import { describe, it, expect } from 'vitest';
import {
  getUpstreamEdges,
  getDownstreamEdges,
  findUpstreamNodes,
  getConnectedTypeMap,
} from '../../src/utils/graphUtils';

// Minimal edge factory matching the shape these functions read
function makeEdge(source: string, target: string, id?: string, targetHandle?: string) {
  const edge: { id: string; source: string; target: string; targetHandle?: string } = {
    id: id ?? `${source}-${target}`,
    source,
    target,
  };
  if (targetHandle !== undefined) {
    edge.targetHandle = targetHandle;
  }
  return edge;
}

const sampleEdges = [
  makeEdge('A', 'B'),
  makeEdge('A', 'C'),
  makeEdge('B', 'C'),
  makeEdge('C', 'D'),
];

describe('getUpstreamEdges', () => {
  it('returns edges targeting the given node', () => {
    const result = getUpstreamEdges('C', sampleEdges);
    expect(result).toHaveLength(2);
    expect(result.map(e => e.source)).toContain('A');
    expect(result.map(e => e.source)).toContain('B');
  });

  it('returns empty array when no edges target the node', () => {
    const result = getUpstreamEdges('A', sampleEdges);
    expect(result).toEqual([]);
  });

  it('returns empty array for empty edges', () => {
    const result = getUpstreamEdges('A', []);
    expect(result).toEqual([]);
  });

  it('returns empty array for non-existent node', () => {
    const result = getUpstreamEdges('Z', sampleEdges);
    expect(result).toEqual([]);
  });
});

describe('getDownstreamEdges', () => {
  it('returns edges sourced from the given node', () => {
    const result = getDownstreamEdges('A', sampleEdges);
    expect(result).toHaveLength(2);
    expect(result.map(e => e.target)).toContain('B');
    expect(result.map(e => e.target)).toContain('C');
  });

  it('returns empty array when no edges source from the node', () => {
    const result = getDownstreamEdges('D', sampleEdges);
    expect(result).toEqual([]);
  });

  it('returns empty array for empty edges', () => {
    const result = getDownstreamEdges('A', []);
    expect(result).toEqual([]);
  });

  it('returns empty array for non-existent node', () => {
    const result = getDownstreamEdges('Z', sampleEdges);
    expect(result).toEqual([]);
  });

  it('returns single edge for a node with one downstream', () => {
    const result = getDownstreamEdges('C', sampleEdges);
    expect(result).toHaveLength(1);
    expect(result[0].target).toBe('D');
  });
});

describe('findUpstreamNodes', () => {
  it('returns source node ids for edges targeting the given node', () => {
    const result = findUpstreamNodes('C', sampleEdges);
    expect(result).toHaveLength(2);
    expect(result).toContain('A');
    expect(result).toContain('B');
  });

  it('returns empty array when no edges target the node', () => {
    const result = findUpstreamNodes('A', sampleEdges);
    expect(result).toEqual([]);
  });

  it('returns empty array for empty edges', () => {
    const result = findUpstreamNodes('A', []);
    expect(result).toEqual([]);
  });

  it('returns single upstream for a node with one incoming edge', () => {
    const result = findUpstreamNodes('B', sampleEdges);
    expect(result).toEqual(['A']);
  });

  it('returns empty array for non-existent node', () => {
    const result = findUpstreamNodes('Z', sampleEdges);
    expect(result).toEqual([]);
  });
});

describe('getConnectedTypeMap', () => {
  it('maps targetHandle → source for upstream edges with targetHandle', () => {
    const edges = [
      makeEdge('A', 'C', 'e1', 'inputImage'),
      makeEdge('B', 'C', 'e2', 'mask'),
    ];
    const map = getConnectedTypeMap('C', edges);
    expect(map.size).toBe(2);
    expect(map.get('inputImage')).toBe('A');
    expect(map.get('mask')).toBe('B');
  });

  it('skips edges without targetHandle', () => {
    const edges = [
      makeEdge('A', 'C'),  // no targetHandle
      makeEdge('B', 'C', 'e2', 'handle1'),
    ];
    const map = getConnectedTypeMap('C', edges);
    expect(map.size).toBe(1);
    expect(map.get('handle1')).toBe('B');
  });

  it('returns empty map when no edges target the node', () => {
    const edges = [makeEdge('A', 'B', 'e1', 'h1')];
    const map = getConnectedTypeMap('C', edges);
    expect(map.size).toBe(0);
  });

  it('returns empty map for empty edges', () => {
    const map = getConnectedTypeMap('C', []);
    expect(map.size).toBe(0);
  });

  it('only includes edges targeting the specified node', () => {
    const edges = [
      makeEdge('A', 'B', 'e1', 'h1'),
      makeEdge('A', 'C', 'e2', 'h2'),
      makeEdge('B', 'D', 'e3', 'h3'),
    ];
    const map = getConnectedTypeMap('C', edges);
    expect(map.size).toBe(1);
    expect(map.get('h2')).toBe('A');
  });

  it('last edge wins when multiple edges share the same targetHandle', () => {
    const edges = [
      makeEdge('A', 'C', 'e1', 'input'),
      makeEdge('B', 'C', 'e2', 'input'),
    ];
    const map = getConnectedTypeMap('C', edges);
    expect(map.size).toBe(1);
    // Map.set overwrites, so the last one (B) wins
    expect(map.get('input')).toBe('B');
  });
});
