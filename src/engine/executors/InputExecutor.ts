import type { WebGLRenderer } from '../webglRenderer';
import type { ShaderNodeData, FramebufferFormat } from '../../types';
import type { Node } from '@xyflow/react';
import type { TextureSource } from './types';

export function prepareInputTexture(
  node: Node<ShaderNodeData>,
  renderer: WebGLRenderer,
  textureSources: Map<string, TextureSource>,
  onNodeError?: (nodeId: string, error: string) => void,
): Promise<void> | null {
  if (node.data.inputMode === 'framebuffer' && node.data.rawDataUrl && node.data.fbWidth && node.data.fbHeight) {
    try {
      const b64 = node.data.rawDataUrl.split(',')[1];
      const binary = atob(b64);
      const buf = new ArrayBuffer(binary.length);
      const view = new Uint8Array(buf);
      for (let i = 0; i < binary.length; i++) view[i] = binary.charCodeAt(i);
      const tex = renderer.loadRawTexture(
        node.id,
        buf,
        (node.data.fbFormat ?? 'rgba8') as FramebufferFormat,
        node.data.fbWidth,
        node.data.fbHeight,
        node.data.fbStride,
      );
      renderer.applyTextureSampling(tex, node.data.texFilter, node.data.texWrap);
      const target = renderer.createTarget(`raw_${node.id}`, node.data.fbWidth, node.data.fbHeight, true);
      renderer.renderSampler2DInput(tex, target);
      textureSources.set(node.id, { kind: 'fbo', target });
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      onNodeError?.(node.id, msg);
    }
    return null;
  }

  const cached = renderer.getImageTexture(node.id);
  if (cached) {
    textureSources.set(node.id, { kind: 'image', texture: cached });
    return null;
  }
  if (node.data.inputDataType === 'sampler2D' && node.data.imageDataUrl) {
    return renderer.loadImageTexture(node.id, node.data.imageDataUrl)
      .then((tex) => {
        renderer.applyTextureSampling(tex, node.data.texFilter, node.data.texWrap);
        textureSources.set(node.id, { kind: 'image', texture: tex });
      })
      .catch((e: unknown) => {
        const msg = e instanceof Error ? e.message : String(e);
        onNodeError?.(node.id, msg);
      });
  }
  return null;
}
