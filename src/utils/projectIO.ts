import type { ProjectFile } from '../types';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../types';
import { checkIsTauri, tauriConvertFileSrc } from './tauri';
import { SHADER_TEMPLATES } from '../catalog/predefinedShaders';

const CURRENT_VERSION = '0.4.0';

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
      nodes: nodes.map(n => {
        const data = { ...n.data };
        if (data.inputMode === 'video') {
          delete data.videoUrl;
        }
        // Prebuilt shaders: strip code from project file (resolved at runtime)
        if (data.shaderTemplateId) {
          data.shaderCode = '';
        }
        return {
          id: n.id,
          type: n.type ?? 'shader',
          position: { x: n.position.x, y: n.position.y },
          data,
        };
      }),
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

export async function deserializeProject(json: string): Promise<{
  project: ProjectFile;
  nodes: Node<ShaderNodeData>[];
  edges: Edge[];
}> {
  const project: ProjectFile = JSON.parse(json);
  if (!project.version) throw new Error('Invalid project file');
  if (project.version !== CURRENT_VERSION) {
    throw new Error(`Incompatible project version: expected ${CURRENT_VERSION}, got ${project.version}`);
  }

  const nodes: Node<ShaderNodeData>[] = project.graph.nodes.map((n) => {
    const data = { ...n.data };
    // Restore prebuilt shader code from catalog
    if (data.shaderTemplateId && !data.shaderCode) {
      const tpl = SHADER_TEMPLATES.get(data.shaderTemplateId);
      if (tpl) data.shaderCode = tpl.code;
    }
    return {
      id: n.id,
      type: n.type,
      position: n.position,
      data,
      selected: false,
      dragging: false,
    };
  });

  const tauri = await checkIsTauri();
  if (tauri) {
    for (const node of nodes) {
      if (node.data.inputMode === 'video' && node.data.videoFilePath && !node.data.videoUrl) {
        try {
          node.data.videoUrl = await tauriConvertFileSrc(node.data.videoFilePath);
        } catch {
          // File may have been moved; user sees reload prompt
        }
      }
    }
  }

  const edges: Edge[] = project.graph.edges.map((e) => ({
    id: e.id,
    source: e.source,
    sourceHandle: e.sourceHandle,
    target: e.target,
    targetHandle: e.targetHandle,
  }));

  return { project, nodes, edges };
}
