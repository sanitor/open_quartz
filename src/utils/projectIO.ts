import type { ProjectFile } from '../types';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../types';

const CURRENT_VERSION = '0.1.0';

export function serializeProject(
  nodes: Node<ShaderNodeData>[],
  edges: Edge[],
  name: string = 'Untitled',
): ProjectFile {
  return {
    version: CURRENT_VERSION,
    name,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    graph: {
      nodes: nodes.map(n => ({
        id: n.id,
        type: n.type ?? 'shader',
        position: { x: n.position.x, y: n.position.y },
        data: n.data,
      })),
      edges: edges.map(e => ({
        id: e.id,
        source: e.source,
        sourceHandle: e.sourceHandle ?? '',
        target: e.target,
        targetHandle: e.targetHandle ?? '',
      })),
    },
  };
}

export function downloadProject(project: ProjectFile, filename?: string): string {
  const blob = new Blob([JSON.stringify(project, null, 2)], {
    type: 'application/json',
  });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename ?? `${project.name}.quartz.json`;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
  return a.download;
}

export function saveFileAs(project: ProjectFile, filename: string): void {
  downloadProject(project, filename);
}

export function saveFile(project: ProjectFile, filename: string): void {
  downloadProject(project, filename);
}

export function deserializeProject(json: string): {
  project: ProjectFile;
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
} {
  const project: ProjectFile = JSON.parse(json);
  if (!project.version) throw new Error('Invalid project file');

  const nodes: Node<ShaderNodeData>[] = project.graph.nodes.map((n) => ({
    id: n.id,
    type: n.type,
    position: n.position,
    data: n.data,
    selected: false,
    dragging: false,
  }));

  const edges: Edge[] = project.graph.edges.map((e) => ({
    id: e.id,
    source: e.source,
    sourceHandle: e.sourceHandle,
    target: e.target,
    targetHandle: e.targetHandle,
  }));

  return { project, nodes, edges };
}
