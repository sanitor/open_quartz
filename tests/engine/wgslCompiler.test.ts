import { describe, expect, it } from 'vitest';
import catalog from '../../src/shaders/catalog.json';
import { validateWgslEdit } from '../../src/engine/gpu/wgslCompiler';

function recordingDevice(): { device: GPUDevice; sources: string[] } {
  const sources: string[] = [];
  const device = {
    createShaderModule: ({ code }: { code: string }) => {
      sources.push(code);
      return { getCompilationInfo: async () => ({ messages: [] }) };
    },
  } as unknown as GPUDevice;
  return { device, sources };
}

describe('edit-time WGSL builtins', () => {
  it('injects the Gray-Scott catalog builtin and feedback alongside parsed ports', async () => {
    const shader = catalog.groups.flatMap((group) => group.items)
      .find((item) => item.label === 'Gray-Scott Reaction-Diffusion')!;
    const { device, sources } = recordingDevice();
    const inputs = ['dA', 'dB', 'feedRate', 'killRate', 'timestep']
      .map((label) => ({ label, dataType: 'float' }));
    expect(await validateWgslEdit(shader.code, inputs, device)).toEqual([]);
    expect(sources[0]).toMatch(/var<uniform> iFrame: f32;/);
    expect(sources[0]).toMatch(/var previousFrame: texture_2d<f32>;/);
    expect(sources[0]).toMatch(/var previousFrameSampler: sampler;/);
  });

  it('uses full identifiers, ignores comments and does not duplicate explicit ports', async () => {
    const { device, sources } = recordingDevice();
    const code = '// iTime iDate\n/* iMouse iResolution */\n@fragment fn main() -> @location(0) vec4f { let iFrameExtra = 0.0; return vec4f(iFrame, iTimeDelta, iFrameExtra, 1.0); }';
    await validateWgslEdit(code, [{ label: 'iFrame', dataType: 'float' }], device);
    const preamble = sources[0].slice(0, sources[0].indexOf(code));
    expect(preamble.match(/var<uniform> iFrame:/g)).toHaveLength(1);
    expect(preamble).toContain('var<uniform> iTimeDelta: f32;');
    expect(preamble).not.toMatch(/var<uniform> (iTime|iDate|iMouse|iResolution):/);
  });
});
