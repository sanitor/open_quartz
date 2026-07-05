import * as THREE from 'three';

const VERTEX_SRC = `in vec3 position;
in vec2 uv;
out vec2 v_uv;
void main() {
  v_uv = uv;
  gl_Position = vec4(position, 1.0);
}`;

// Strip user declarations that will be injected by the system
function stripInjected(code: string): string {
  return code
    .replace(/#version\s+\d+\s*\w*\s*/g, '')
    .replace(/precision\s+\w+\s+\w+\s*;/g, '')
    .replace(/out\s+vec[234]\s+\w+\s*;/g, '');
}

export function compileNodeShader(
  nodeShaderCode: string,
  inputPorts: Array<{ label: string; dataType: string }>,
  upstreamMap: Map<string, string>, // uniformName -> upstream nodeId
): {
  material: THREE.ShaderMaterial;
  upstreamSamplers: Map<string, string>; // uniformName -> upstream nodeId
} {
  const upstreamSamplers = new Map<string, string>();
  let userCode = stripInjected(nodeShaderCode);

  let src = `precision highp float;\nin vec2 v_uv;\nout vec4 fragColor;\n`;

  // Inject uniforms for connected upstream nodes (keep original names)
  for (const [uniformName, sourceNodeId] of upstreamMap) {
    const port = inputPorts.find((p) => p.label === uniformName);
    if (port?.dataType === 'sampler2D') {
      src += `uniform sampler2D ${uniformName};\n`;
      upstreamSamplers.set(uniformName, sourceNodeId);
    } else if (port) {
      src += `uniform ${port.dataType} ${uniformName};\n`;
    }
    const re = new RegExp(`uniform\\s+\\w+\\s+${uniformName}\\s*;?`, 'g');
    userCode = userCode.replace(re, '');
  }

  // Inject non-sampler uniforms that aren't connected upstream
  for (const input of inputPorts) {
    if (!upstreamMap.has(input.label) && input.dataType !== 'sampler2D') {
      src += `uniform ${input.dataType} ${input.label};\n`;
      const re = new RegExp(`uniform\\s+${input.dataType}\\s+${input.label}\\s*;?`, 'g');
      userCode = userCode.replace(re, '');
    }
  }

  src += `\n${userCode}\n`;

  const material = new THREE.RawShaderMaterial({
    vertexShader: VERTEX_SRC,
    fragmentShader: src,
    uniforms: {},
    glslVersion: THREE.GLSL3,
  });

  return { material, upstreamSamplers };
}
