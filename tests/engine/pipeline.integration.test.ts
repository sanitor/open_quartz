import { describe, it, expect, vi, beforeEach } from 'vitest';
import type { Node, Edge } from '@xyflow/react';
import type { ShaderNodeData } from '../../src/types';

// ---------------------------------------------------------------------------
// vi.hoisted — mock instances available inside vi.mock factories
// ---------------------------------------------------------------------------
const { mockRendererInstance, compileShaderMock } = vi.hoisted(() => {
  const inst = {
    setSize: vi.fn(),
    createTarget: vi.fn(() => ({
      texture: { type: 1 },
      width: 512,
      height: 512,
      dispose: vi.fn(),
    })),
    getContext: vi.fn(() => ({})),
    loadImageTexture: vi.fn(() => Promise.resolve({ type: 1, dispose: vi.fn() })),
    applyTextureSampling: vi.fn(),
    loadRawTexture: vi.fn(() => ({ type: 1, dispose: vi.fn() })),
    renderSampler2DInput: vi.fn(),
    renderWithMaterial: vi.fn(),
    readTargetToDataURL: vi.fn(() => 'data:image/png;base64,mock'),
    readTargetToCanvas: vi.fn(() => document.createElement('canvas')),
    renderToScreen: vi.fn(),
    getImageTexture: vi.fn(() => undefined),
    dispose: vi.fn(),
    clearResources: vi.fn(),
    clearTarget: vi.fn(),
    canvas: document.createElement('canvas'),
  };
  const compileMock = vi.fn(
    (_code: string, _inputs: unknown[], upstreamMap?: Map<string, string>) => ({
      material: { fragmentShader: 'compiled', uniforms: {} },
      upstreamSamplers: upstreamMap ? new Map(upstreamMap) : new Map<string, string>(),
      preambleLines: 5,
      needsFeedback: false,
    }),
  );
  return { mockRendererInstance: inst, compileShaderMock: compileMock };
});

// ---------------------------------------------------------------------------
// Module mocks
// ---------------------------------------------------------------------------
vi.mock('three', () => ({
  ShaderMaterial: class {},
  RawShaderMaterial: class {
    vertexShader = '';
    fragmentShader = '';
    uniforms: Record<string, unknown> = {};
    glslVersion = '';
    constructor(opts?: {
      vertexShader?: string;
      fragmentShader?: string;
      uniforms?: Record<string, unknown>;
      glslVersion?: string;
    }) {
      if (opts) {
        this.vertexShader = opts.vertexShader ?? '';
        this.fragmentShader = opts.fragmentShader ?? '';
        this.uniforms = opts.uniforms ?? {};
        this.glslVersion = opts.glslVersion ?? '';
      }
    }
  },
  GLSL3: 'GLSL3',
  WebGLRenderTarget: class {
    texture = { type: 1 };
    width = 0;
    height = 0;
    dispose = vi.fn();
  },
  Texture: class {
    dispose = vi.fn();
  },
}));

vi.mock('../../src/engine/webglRenderer', () => {
  const Ctor = vi.fn(function (this: Record<string, unknown>) {
    Object.assign(this, mockRendererInstance);
  });
  return { WebGLRenderer: Ctor };
});

vi.mock('../../src/engine/shaderCompiler', () => ({
  compileNodeShader: compileShaderMock,
  validateFragmentShader: vi.fn(() => null),
}));

vi.mock('../../src/engine/graphExecutor', () => ({
  topologicalSort: vi.fn((nodes: Array<{ id: string }>) => nodes.map((n) => n.id)),
}));

// Mock Image globally
class MockImage {
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  naturalWidth = 256;
  naturalHeight = 256;
  private _src = '';

  get src() {
    return this._src;
  }
  set src(val: string) {
    this._src = val;
    Promise.resolve().then(() => {
      if (val.includes('fail')) {
        this.onerror?.();
      } else {
        this.onload?.();
      }
    });
  }
}
vi.stubGlobal('Image', MockImage);

vi.stubGlobal('atob', (b64: string) => {
  const decoded = Buffer.from(b64, 'base64').toString('binary');
  return decoded;
});

// Mock onnx modules
vi.mock('../../src/catalog/onnxRegistry', () => ({
  ONNX_MODELS: {
    yolov8n: {
      id: 'yolov8n',
      label: 'YOLOv8n',
      modelUrl: '/m.onnx',
      targetSize: 640,
      scoreThreshold: 0.25,
      iouThreshold: 0.45,
      description: '',
      inputs: [],
      outputs: [],
    },
  },
  DEFAULT_ONNX_MODEL_ID: 'yolov8n',
}));
vi.mock('../../src/engine/onnxOverlay', () => ({
  drawDetectionOverlay: vi.fn(() => ({
    texture: { type: 1, dispose: vi.fn() },
    dataUrl: 'data:image/png;base64,overlay',
  })),
}));

// ---------------------------------------------------------------------------
// Imports (after mocks)
// ---------------------------------------------------------------------------
import { ExecutionEngine } from '../../src/engine/executionEngine';
import type { FrameInputs } from '../../src/engine/compositor';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function makeNode(id: string, data: Partial<ShaderNodeData>): Node<ShaderNodeData> {
  return {
    id,
    type: 'custom',
    position: { x: 0, y: 0 },
    data: {
      type: 'shader',
      label: id,
      shaderCode: 'void main() {}',
      inputs: [],
      outputs: [],
      uniforms: {},
      ...data,
    },
  };
}

function makeEdge(source: string, target: string, targetHandle: string, sourceHandle = 'output'): Edge {
  return {
    id: `${source}-${target}`,
    source,
    sourceHandle,
    target,
    targetHandle,
  };
}

function makeFrameInputs(overrides: Partial<FrameInputs> = {}): FrameInputs {
  return {
    time: 0,
    delta: 0.016,
    frame: 0,
    date: new Float32Array(4),
    mouse: new Float32Array(4),
    resolution: new Float32Array([512, 512, 1]),
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Pipeline Integration', () => {
  let engine: ExecutionEngine;

  beforeEach(() => {
    vi.clearAllMocks();
    engine = new ExecutionEngine(document.createElement('canvas'));
  });

  // =========================================================================
  // 1. Image → Shader → Renderer
  // =========================================================================
  describe('Image → Shader → Renderer pipeline', () => {
    const imageNode = makeNode('img1', {
      type: 'input',
      inputMode: 'image',
      inputDataType: 'sampler2D',
      imageDataUrl: 'data:image/png;base64,abc',
      imageWidth: 256,
      imageHeight: 256,
      shaderCode: '',
      inputs: [{ id: 'img1_value', label: 'value', dataType: 'sampler2D', direction: 'input' }],
      outputs: [{ id: 'img1_out', label: 'value', dataType: 'sampler2D', direction: 'output' }],
    });

    const shaderNode = makeNode('shader1', {
      type: 'shader',
      shaderCode: 'uniform sampler2D inputImage;\nout vec4 fragColor;\nvoid main() { fragColor = texture(inputImage, v_uv); }',
      inputs: [{ id: 'shader1_inputImage', label: 'inputImage', dataType: 'sampler2D', direction: 'input' }],
      outputs: [{ id: 'shader1_fragColor', label: 'fragColor', dataType: 'vec4', direction: 'output' }],
    });

    const rendererNode = makeNode('renderer1', {
      type: 'renderer',
      shaderCode: '',
      inputs: [{ id: 'renderer1_inputTexture', label: 'inputTexture', dataType: 'sampler2D', direction: 'input' }],
      outputs: [],
    });

    const nodes = [imageNode, shaderNode, rendererNode];
    const edges: Edge[] = [
      makeEdge('img1', 'shader1', 'shader1_inputImage', 'img1_out'),
      makeEdge('shader1', 'renderer1', 'renderer1_inputTexture', 'shader1_fragColor'),
    ];

    it('prepare() returns a valid plan with all nodes', () => {
      const plan = engine.prepare(nodes, edges);
      expect(plan).not.toBeNull();
      expect(plan!.sortedIds).toEqual(['img1', 'shader1', 'renderer1']);
    });

    it('plan.outputNodes contains the renderer', () => {
      const plan = engine.prepare(nodes, edges);
      expect(plan!.outputNodes).toEqual(['renderer1']);
    });

    it('shader upstream bindings point to the image input', () => {
      const plan = engine.prepare(nodes, edges);
      const shaderBindings = plan!.upstreamSamplerBindings.get('shader1');
      expect(shaderBindings).toBeDefined();
      expect(shaderBindings!.get('inputImage')).toBe('img1');
    });

    it('renderer upstream bindings point to the shader', () => {
      const plan = engine.prepare(nodes, edges);
      const rendererBindings = plan!.upstreamSamplerBindings.get('renderer1');
      expect(rendererBindings).toBeDefined();
      expect(rendererBindings!.get('inputTexture')).toBe('shader1');
    });

    it('loadImageTexture is called during prepare for the image input', () => {
      engine.prepare(nodes, edges);
      expect(mockRendererInstance.loadImageTexture).toHaveBeenCalledWith(
        'img1',
        'data:image/png;base64,abc',
      );
    });

    it('runFrame() executes the shader via renderWithMaterial', async () => {
      const plan = engine.prepare(nodes, edges);
      expect(plan).not.toBeNull();
      // Wait for async image load
      await Promise.all(plan!.pendingTextures);
      engine.runFrame(plan!, makeFrameInputs());
      expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalled();
    });

    it('renderRendererToScreen blits the shader output to screen', async () => {
      const plan = engine.prepare(nodes, edges);
      expect(plan).not.toBeNull();
      await Promise.all(plan!.pendingTextures);
      engine.runFrame(plan!, makeFrameInputs());
      engine.renderRendererToScreen(plan!, 'renderer1');
      expect(mockRendererInstance.renderToScreen).toHaveBeenCalled();
    });
  });

  // =========================================================================
  // 2. Fork/Join: A → B, A → C, B+C → D
  // =========================================================================
  describe('Fork/join graph (A→B, A→C, B+C→D)', () => {
    const nodeA = makeNode('A', {
      type: 'input',
      inputMode: 'image',
      inputDataType: 'sampler2D',
      imageDataUrl: 'data:image/png;base64,aaa',
      imageWidth: 128,
      imageHeight: 128,
      shaderCode: '',
      inputs: [{ id: 'A_value', label: 'value', dataType: 'sampler2D', direction: 'input' }],
      outputs: [{ id: 'A_out', label: 'value', dataType: 'sampler2D', direction: 'output' }],
    });

    const nodeB = makeNode('B', {
      type: 'shader',
      inputs: [{ id: 'B_inputImage', label: 'inputImage', dataType: 'sampler2D', direction: 'input' }],
      outputs: [{ id: 'B_fragColor', label: 'fragColor', dataType: 'vec4', direction: 'output' }],
    });

    const nodeC = makeNode('C', {
      type: 'shader',
      inputs: [{ id: 'C_inputImage', label: 'inputImage', dataType: 'sampler2D', direction: 'input' }],
      outputs: [{ id: 'C_fragColor', label: 'fragColor', dataType: 'vec4', direction: 'output' }],
    });

    const nodeD = makeNode('D', {
      type: 'shader',
      shaderCode: 'uniform sampler2D inputA;\nuniform sampler2D inputB;\nout vec4 fragColor;\nvoid main() { fragColor = mix(texture(inputA, v_uv), texture(inputB, v_uv), 0.5); }',
      inputs: [
        { id: 'D_inputA', label: 'inputA', dataType: 'sampler2D', direction: 'input' },
        { id: 'D_inputB', label: 'inputB', dataType: 'sampler2D', direction: 'input' },
      ],
      outputs: [{ id: 'D_fragColor', label: 'fragColor', dataType: 'vec4', direction: 'output' }],
    });

    const nodes = [nodeA, nodeB, nodeC, nodeD];
    const edges: Edge[] = [
      makeEdge('A', 'B', 'B_inputImage', 'A_out'),
      makeEdge('A', 'C', 'C_inputImage', 'A_out'),
      makeEdge('B', 'D', 'D_inputA', 'B_fragColor'),
      makeEdge('C', 'D', 'D_inputB', 'C_fragColor'),
    ];

    it('prepare() includes all 4 nodes in sorted order', () => {
      const plan = engine.prepare(nodes, edges);
      expect(plan).not.toBeNull();
      expect(plan!.sortedIds).toHaveLength(4);
    });

    it('blend shader D has both upstream bindings (inputA→B, inputB→C)', () => {
      const plan = engine.prepare(nodes, edges);
      const dBindings = plan!.upstreamSamplerBindings.get('D');
      expect(dBindings).toBeDefined();
      expect(dBindings!.get('inputA')).toBe('B');
      expect(dBindings!.get('inputB')).toBe('C');
    });

    it('runFrame() calls renderWithMaterial for B, C, and D', async () => {
      const plan = engine.prepare(nodes, edges);
      expect(plan).not.toBeNull();
      await Promise.all(plan!.pendingTextures);
      engine.runFrame(plan!, makeFrameInputs());
      // B, C, D are all shader nodes that get rendered
      expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalledTimes(3);
    });
  });

  // =========================================================================
  // 3. Feedback / Ping-pong
  // =========================================================================
  describe('Feedback / ping-pong', () => {
    beforeEach(() => {
      // Make compileNodeShader return needsFeedback: true for the feedback node
      compileShaderMock.mockImplementation(
        (_code: string, _inputs: unknown[], upstreamMap?: Map<string, string>) => ({
          material: { fragmentShader: 'compiled', uniforms: {} },
          upstreamSamplers: upstreamMap ? new Map(upstreamMap) : new Map<string, string>(),
          preambleLines: 5,
          needsFeedback: true,
        }),
      );
    });

    const fbNode = makeNode('fb1', {
      type: 'shader',
      shaderCode: 'uniform sampler2D previousFrame;\nout vec4 fragColor;\nvoid main() { fragColor = texture(previousFrame, v_uv) * 0.99; }',
      inputs: [],
      outputs: [{ id: 'fb1_fragColor', label: 'fragColor', dataType: 'vec4', direction: 'output' }],
    });

    it('prepare() allocates feedback targets for the node', () => {
      const plan = engine.prepare([fbNode], []);
      expect(plan).not.toBeNull();
      expect(plan!.feedbackTargets.has('fb1')).toBe(true);
      // Two targets (ping-pong pair)
      const targets = plan!.feedbackTargets.get('fb1')!;
      expect(targets).toHaveLength(2);
    });

    it('prepare() marks the node in feedbackFirstFrame', () => {
      const plan = engine.prepare([fbNode], []);
      expect(plan!.feedbackFirstFrame.has('fb1')).toBe(true);
    });

    it('createTarget is called twice for the ping-pong pair', () => {
      engine.prepare([fbNode], []);
      // Two feedback targets: fb1_fb0 and fb1_fb1
      const fbCalls = mockRendererInstance.createTarget.mock.calls.filter(
        (call: unknown[]) => typeof call[0] === 'string' && (call[0] as string).startsWith('fb1_fb'),
      );
      expect(fbCalls).toHaveLength(2);
    });

    it('first runFrame clears both targets then renders', () => {
      const plan = engine.prepare([fbNode], []);
      expect(plan).not.toBeNull();
      engine.runFrame(plan!, makeFrameInputs());
      // clearTarget should have been called for feedback initialization
      expect(mockRendererInstance.clearTarget).toHaveBeenCalled();
      expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalled();
    });

    it('second runFrame does not clear targets (feedbackFirstFrame consumed)', () => {
      const plan = engine.prepare([fbNode], []);
      expect(plan).not.toBeNull();

      // First frame
      engine.runFrame(plan!, makeFrameInputs());
      const clearCountAfterFirst = mockRendererInstance.clearTarget.mock.calls.length;

      // Second frame — no additional clears
      vi.clearAllMocks();
      engine.runFrame(plan!, makeFrameInputs({ frame: 1 }));
      expect(mockRendererInstance.clearTarget).not.toHaveBeenCalled();
      // But renderWithMaterial still called
      expect(mockRendererInstance.renderWithMaterial).toHaveBeenCalled();
    });

    it('feedbackReadIndex swaps after each frame', () => {
      const plan = engine.prepare([fbNode], []);
      expect(plan).not.toBeNull();
      const idx0 = plan!.feedbackReadIndex.get('fb1');

      engine.runFrame(plan!, makeFrameInputs());
      const idx1 = plan!.feedbackReadIndex.get('fb1');
      expect(idx1).not.toBe(idx0);

      engine.runFrame(plan!, makeFrameInputs({ frame: 1 }));
      const idx2 = plan!.feedbackReadIndex.get('fb1');
      expect(idx2).toBe(idx0); // Swapped back
    });
  });

  // =========================================================================
  // 4. Math → Shader propagation
  // =========================================================================
  describe('Math → Shader propagation', () => {
    beforeEach(() => {
      // Reset to non-feedback shader compilation
      compileShaderMock.mockImplementation(
        (_code: string, _inputs: unknown[], upstreamMap?: Map<string, string>) => ({
          material: { fragmentShader: 'compiled', uniforms: {} },
          upstreamSamplers: upstreamMap ? new Map(upstreamMap) : new Map<string, string>(),
          preambleLines: 5,
          needsFeedback: false,
        }),
      );
    });

    const floatInput = makeNode('float1', {
      type: 'input',
      inputDataType: 'float',
      shaderCode: 'uniform float value;\nout float outputValue;\nvoid main() { outputValue = value; }',
      inputs: [{ id: 'float1_value', label: 'value', dataType: 'float', direction: 'input' }],
      outputs: [{ id: 'float1_out', label: 'value', dataType: 'float', direction: 'output' }],
      uniforms: { value: 2.0 },
      inputMode: undefined,
    });

    const mathNode = makeNode('math1', {
      type: 'math',
      mathOp: 'multiply',
      shaderCode: '',
      inputs: [
        { id: 'math1_a', label: 'a', dataType: 'auto' as ShaderNodeData['inputs'][0]['dataType'], direction: 'input' },
        { id: 'math1_b', label: 'b', dataType: 'auto' as ShaderNodeData['inputs'][0]['dataType'], direction: 'input' },
      ],
      outputs: [{ id: 'math1_out', label: 'result', dataType: 'auto' as ShaderNodeData['inputs'][0]['dataType'], direction: 'output' }],
      uniforms: { b: 3.0 },
    });

    const shaderNode = makeNode('shader1', {
      type: 'shader',
      shaderCode: 'uniform float speed;\nout vec4 fragColor;\nvoid main() { fragColor = vec4(speed); }',
      inputs: [{ id: 'shader1_speed', label: 'speed', dataType: 'float', direction: 'input' }],
      outputs: [{ id: 'shader1_fragColor', label: 'fragColor', dataType: 'vec4', direction: 'output' }],
    });

    const nodes = [floatInput, mathNode, shaderNode];
    const edges: Edge[] = [
      makeEdge('float1', 'math1', 'math1_a', 'float1_out'),
      makeEdge('math1', 'shader1', 'shader1_speed', 'math1_out'),
    ];

    it('runFrame computes math result (2.0 * 3.0 = 6.0)', () => {
      const plan = engine.prepare(nodes, edges);
      expect(plan).not.toBeNull();
      engine.runFrame(plan!, makeFrameInputs());
      expect(plan!.mathValues.get('math1')).toBe(6);
    });

    it('math upstream bindings are recorded for the math node', () => {
      const plan = engine.prepare(nodes, edges);
      const mathBindings = plan!.upstreamSamplerBindings.get('math1');
      expect(mathBindings).toBeDefined();
      expect(mathBindings!.get('a')).toBe('float1');
    });
  });

  // =========================================================================
  // 5. ONNX node in pipeline
  // =========================================================================
  describe('ONNX node in pipeline', () => {
    beforeEach(() => {
      compileShaderMock.mockImplementation(
        (_code: string, _inputs: unknown[], upstreamMap?: Map<string, string>) => ({
          material: { fragmentShader: 'compiled', uniforms: {} },
          upstreamSamplers: upstreamMap ? new Map(upstreamMap) : new Map<string, string>(),
          preambleLines: 5,
          needsFeedback: false,
        }),
      );
    });

    const imageNode = makeNode('img1', {
      type: 'input',
      inputMode: 'image',
      inputDataType: 'sampler2D',
      imageDataUrl: 'data:image/png;base64,abc',
      imageWidth: 640,
      imageHeight: 640,
      shaderCode: '',
      inputs: [{ id: 'img1_value', label: 'value', dataType: 'sampler2D', direction: 'input' }],
      outputs: [{ id: 'img1_out', label: 'value', dataType: 'sampler2D', direction: 'output' }],
    });

    const onnxNode = makeNode('onnx1', {
      type: 'onnx',
      onnxModelId: 'yolov8n',
      onnxStatus: 'ready',
      shaderCode: '',
      inputs: [{ id: 'onnx1_inputImage', label: 'inputImage', dataType: 'sampler2D', direction: 'input' }],
      outputs: [{ id: 'onnx1_output', label: 'output', dataType: 'sampler2D', direction: 'output' }],
    });

    const rendererNode = makeNode('renderer1', {
      type: 'renderer',
      shaderCode: '',
      inputs: [{ id: 'renderer1_inputTexture', label: 'inputTexture', dataType: 'sampler2D', direction: 'input' }],
      outputs: [],
    });

    const nodes = [imageNode, onnxNode, rendererNode];
    const edges: Edge[] = [
      makeEdge('img1', 'onnx1', 'onnx1_inputImage', 'img1_out'),
      makeEdge('onnx1', 'renderer1', 'renderer1_inputTexture', 'onnx1_output'),
    ];

    it('prepare() skips shader compilation for ONNX nodes', () => {
      engine.prepare(nodes, edges);
      // compileNodeShader should NOT be called for the ONNX node
      // It may be called for other nodes but not onnx1
      const calls = compileShaderMock.mock.calls;
      // No shader compilation for ONNX — only for non-ONNX renderable nodes
      // In this pipeline, img1 is input (skipped), onnx1 is onnx (skipped),
      // renderer1 is renderer (skipped). So no calls.
      expect(calls).toHaveLength(0);
    });

    it('ONNX upstream bindings are recorded', () => {
      const plan = engine.prepare(nodes, edges);
      expect(plan).not.toBeNull();
      const onnxBindings = plan!.upstreamSamplerBindings.get('onnx1');
      expect(onnxBindings).toBeDefined();
      expect(onnxBindings!.get('inputImage')).toBe('img1');
    });

    it('runFrame reads upstream texture to canvas for ONNX inference', async () => {
      const plan = engine.prepare(nodes, edges);
      expect(plan).not.toBeNull();
      await Promise.all(plan!.pendingTextures);
      engine.runFrame(plan!, makeFrameInputs());
      // ONNX path: createTarget (scratch) → renderSampler2DInput → readTargetToCanvas
      expect(mockRendererInstance.readTargetToCanvas).toHaveBeenCalled();
    });

    it('ONNX in-flight guard prevents re-execution on next frame', async () => {
      const plan = engine.prepare(nodes, edges);
      expect(plan).not.toBeNull();
      await Promise.all(plan!.pendingTextures);

      // First frame: triggers inference
      engine.runFrame(plan!, makeFrameInputs());
      const firstCallCount = mockRendererInstance.readTargetToCanvas.mock.calls.length;

      // Second frame: in-flight, should be skipped
      engine.runFrame(plan!, makeFrameInputs({ frame: 1 }));
      expect(mockRendererInstance.readTargetToCanvas.mock.calls.length).toBe(firstCallCount);
    });

    it('ONNX node with status !== ready is skipped entirely', async () => {
      const downloadingNode = makeNode('onnx_dl', {
        type: 'onnx',
        onnxModelId: 'yolov8n',
        onnxStatus: 'downloading',
        shaderCode: '',
        inputs: [{ id: 'onnx_dl_in', label: 'inputImage', dataType: 'sampler2D', direction: 'input' }],
        outputs: [{ id: 'onnx_dl_out', label: 'output', dataType: 'sampler2D', direction: 'output' }],
      });
      const nodesWithDl = [imageNode, downloadingNode, rendererNode];
      const edgesWithDl: Edge[] = [
        makeEdge('img1', 'onnx_dl', 'onnx_dl_in', 'img1_out'),
        makeEdge('onnx_dl', 'renderer1', 'renderer1_inputTexture', 'onnx_dl_out'),
      ];
      const plan = engine.prepare(nodesWithDl, edgesWithDl);
      expect(plan).not.toBeNull();
      await Promise.all(plan!.pendingTextures);
      engine.runFrame(plan!, makeFrameInputs());
      // No readTargetToCanvas because ONNX node is still downloading
      expect(mockRendererInstance.readTargetToCanvas).not.toHaveBeenCalled();
    });
  });
});
