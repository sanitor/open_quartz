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

export async function saveFileAs(
  project: ProjectFile,
): Promise<{ name: string; handle: FileSystemFileHandle | null } | null> {
  if (typeof window !== 'undefined' && 'showSaveFilePicker' in window) {
    try {
      const handle = await (window as any).showSaveFilePicker({
        suggestedName: `${project.name}.quartz.json`,
        types: [{
          description: 'Open Quartz Project',
          accept: { 'application/json': ['.quartz.json'] },
        }],
      });
      const writable = await handle.createWritable();
      await writable.write(JSON.stringify(project, null, 2));
      await writable.close();
      return { name: (handle as FileSystemFileHandle).name, handle: handle as FileSystemFileHandle };
    } catch (err) {
      if ((err as DOMException)?.name === 'AbortError') return null;
      throw err;
    }
  }

  const fallbackName = downloadProject(project);
  return { name: fallbackName, handle: null };
}

export async function saveFile(
  project: ProjectFile,
  handle: FileSystemFileHandle | null,
  fallbackName?: string,
): Promise<void> {
  if (handle) {
    const writable = await handle.createWritable();
    await writable.write(JSON.stringify(project, null, 2));
    await writable.close();
    return;
  }

  if (fallbackName) {
    downloadProject(project, fallbackName);
  }
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
