import type * as THREE from 'three';

export type TextureSource =
  | { kind: 'fbo'; target: THREE.WebGLRenderTarget }
  | { kind: 'image'; texture: THREE.Texture };

export const BUILTIN_UNIFORMS = new Set([
  'iTime', 'iTimeDelta', 'iFrame', 'iDate', 'iMouse', 'iResolution', 'previousFrame',
]);

export function setUniform(material: THREE.ShaderMaterial, key: string, value: unknown): void {
  const uniform = material.uniforms[key];
  if (uniform) {
    uniform.value = value;
  } else {
    material.uniforms[key] = { value };
  }
}

export function normalizeUniformValue(value: unknown): unknown {
  if (typeof value === 'number' || typeof value === 'boolean') return value;
  if (typeof value === 'string') {
    const num = Number(value);
    return Number.isNaN(num) ? value : num;
  }
  return value;
}

export function formatShaderError(msg: string, preambleLines: number): string {
  const lines = msg.split('\n');
  const relevant = lines.filter(
    (l) => l.includes('ERROR:') || l.includes('WARNING:')
  );
  const result = relevant.length > 0 ? relevant : lines.filter(
    (l) => l.includes('Shader Error') || l.includes('getProgramInfoLog')
  );
  if (result.length === 0) return msg;
  if (preambleLines <= 0) return result.join('\n');
  return result.map((line) =>
    line.replace(/(\d+):(\d+):/g, (_match, strNum, lineNum) => {
      const adjusted = parseInt(lineNum, 10) - preambleLines;
      return `${strNum}:${adjusted > 0 ? adjusted : 1}:`;
    })
  ).join('\n');
}

export function isRenderableNode(node: { data: { type: string } }): boolean {
  return node.data.type === 'shader' || node.data.type === 'constant' || node.data.type === 'renderer' || node.data.type === 'onnx';
}
