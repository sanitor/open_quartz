import { describe, it, expect } from 'vitest';
import { topologicalSort } from '../../src/engine/graphExecutor';

// Minimal node/edge factories matching the shapes topologicalSort reads:
// Node needs { id: string }, Edge needs { source: string, target: string }
function makeNode(id: string): { id: string; position: { x: number; y: number }; data: Record<string, unknown> } {
  return { id, position: { x: 0, y: 0 }, data: {} };
}

function makeEdge(source: string, target: string): { id: string; source: string; target: string } {
  return { id: `${source}-${target}`, source, target };
}

describe('topologicalSort', () => {
  it('sorts a linear DAG A→B→C as [A, B, C]', () => {
    const nodes = [makeNode('A'), makeNode('B'), makeNode('C')];
    const edges = [makeEdge('A', 'B'), makeEdge('B', 'C')];
    const order = topologicalSort(nodes, edges);
    expect(order).toEqual(['A', 'B', 'C']);
  });

  it('sorts a diamond DAG (A→B, A→C, B→D, C→D)', () => {
    const nodes = [makeNode('A'), makeNode('B'), makeNode('C'), makeNode('D')];
    const edges = [
      makeEdge('A', 'B'),
      makeEdge('A', 'C'),
      makeEdge('B', 'D'),
      makeEdge('C', 'D'),
    ];
    const order = topologicalSort(nodes, edges);

    // A must come first, D must come last
    expect(order[0]).toBe('A');
    expect(order[order.length - 1]).toBe('D');
    expect(order).toHaveLength(4);

    // B and C must both appear before D
    expect(order.indexOf('B')).toBeLessThan(order.indexOf('D'));
    expect(order.indexOf('C')).toBeLessThan(order.indexOf('D'));
  });

  it('returns single node when there are no edges', () => {
    const nodes = [makeNode('X')];
    const order = topologicalSort(nodes, []);
    expect(order).toEqual(['X']);
  });

  it('handles multiple disconnected subgraphs', () => {
    const nodes = [
      makeNode('A'), makeNode('B'),
      makeNode('X'), makeNode('Y'),
    ];
    const edges = [makeEdge('A', 'B'), makeEdge('X', 'Y')];
    const order = topologicalSort(nodes, edges);
    expect(order).toHaveLength(4);

    // Within each subgraph, order is preserved
    expect(order.indexOf('A')).toBeLessThan(order.indexOf('B'));
    expect(order.indexOf('X')).toBeLessThan(order.indexOf('Y'));
  });

  it('returns partial order for cyclic graph (only nodes with 0 in-degree)', () => {
    // A→B→C→A (full cycle, no node has 0 in-degree... unless A is never targeted)
    // Actually A→B, B→C, C→A means all have in-degree 1 → none have 0 → empty
    const nodes = [makeNode('A'), makeNode('B'), makeNode('C')];
    const edges = [
      makeEdge('A', 'B'),
      makeEdge('B', 'C'),
      makeEdge('C', 'A'),
    ];
    const order = topologicalSort(nodes, edges);
    // Pure cycle: every node has in-degree ≥ 1, so none enter the queue
    expect(order).toEqual([]);
  });

  it('returns reachable nodes for a partial cycle (some nodes have 0 in-degree)', () => {
    // S→A→B→A (S has 0 in-degree, A and B form a cycle)
    const nodes = [makeNode('S'), makeNode('A'), makeNode('B')];
    const edges = [
      makeEdge('S', 'A'),
      makeEdge('A', 'B'),
      makeEdge('B', 'A'),
    ];
    const order = topologicalSort(nodes, edges);
    // S is the only node with in-degree 0, but decrementing A's in-degree from S
    // still leaves it at 1 (from B→A), so only S is emitted
    expect(order).toEqual(['S']);
  });

  it('handles empty nodes and edges', () => {
    const order = topologicalSort([], []);
    expect(order).toEqual([]);
  });

  it('handles edges referencing non-existent nodes gracefully', () => {
    // Edge source "Z" not in nodes → adj.get(e.source) returns undefined → skipped
    const nodes = [makeNode('A')];
    const edges = [makeEdge('Z', 'A')];
    const order = topologicalSort(nodes, edges);
    // A has in-degree 0 (edge source Z not in adj map, so push is skipped)
    expect(order).toEqual(['A']);
  });

  it('preserves correct order for a longer chain', () => {
    const ids = ['1', '2', '3', '4', '5'];
    const nodes = ids.map(makeNode);
    const edges = [
      makeEdge('1', '2'),
      makeEdge('2', '3'),
      makeEdge('3', '4'),
      makeEdge('4', '5'),
    ];
    const order = topologicalSort(nodes, edges);
    expect(order).toEqual(['1', '2', '3', '4', '5']);
  });

  it('handles fan-out (one source, multiple targets)', () => {
    const nodes = [makeNode('S'), makeNode('A'), makeNode('B'), makeNode('C')];
    const edges = [
      makeEdge('S', 'A'),
      makeEdge('S', 'B'),
      makeEdge('S', 'C'),
    ];
    const order = topologicalSort(nodes, edges);
    expect(order[0]).toBe('S');
    expect(order).toHaveLength(4);
    expect(order).toContain('A');
    expect(order).toContain('B');
    expect(order).toContain('C');
  });

  it('handles fan-in (multiple sources, one target)', () => {
    const nodes = [makeNode('A'), makeNode('B'), makeNode('C'), makeNode('T')];
    const edges = [
      makeEdge('A', 'T'),
      makeEdge('B', 'T'),
      makeEdge('C', 'T'),
    ];
    const order = topologicalSort(nodes, edges);
    expect(order[order.length - 1]).toBe('T');
    expect(order).toHaveLength(4);
  });
});
