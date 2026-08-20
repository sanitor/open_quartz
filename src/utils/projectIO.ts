import type { ProjectFile } from '../types';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import { OpenQuartzClient, Project as SdkProject } from '../sdk';

export function serializeProject(
  nodes: Node<ShaderNodeData>[],
  edges: Edge[],
  name: string = 'Untitled',
): ProjectFile {
  return new SdkProject(name, nodes, edges).toFile();
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

export async function deserializeProject(json: string): Promise<{
  project: ProjectFile;
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
}> {
  const opened = await new OpenQuartzClient().openProject(json);
  const project = opened.toFile();
  const graph = opened.graph.snapshot();
  const nodes: Node<ShaderNodeData>[] = graph.nodes.map((n) => ({
    id: n.id,
    type: n.type,
    position: n.position,
    data: n.data,
    selected: false,
    dragging: false,
  }));

  const edges: Edge[] = graph.edges.map((e) => ({
    id: e.id,
    source: e.source,
    sourceHandle: e.sourceHandle,
    target: e.target,
    targetHandle: e.targetHandle,
  }));

  return { project, nodes, edges };
}
