export type DataType =
  | 'float' | 'int' | 'uint' | 'bool'
  | 'vec2' | 'vec3' | 'vec4'
  | 'ivec2' | 'ivec3' | 'ivec4'
  | 'uvec2' | 'uvec3' | 'uvec4'
  | 'bvec2' | 'bvec3' | 'bvec4'
  | 'mat2' | 'mat3' | 'mat4'
  | 'sampler2D' | 'samplerCube';

export type InputMode = 'image' | 'framebuffer';

export type FramebufferFormat = 'rgba8' | 'rgba32f' | 'rg8' | 'rg32f' | 'r8' | 'r32f' | 'nv12';

export type TextureFilter = 'linear' | 'nearest';
export type TextureWrap = 'clamp' | 'repeat' | 'mirror';

export interface Port {
  id: string;
  label: string;
  dataType: DataType;
  direction: 'input' | 'output';
  defaultValue?: unknown;
}

export type NodeType = 'shader' | 'input' | 'constant';

export interface ShaderNodeData {
  type: NodeType;
  label: string;
  shaderCode: string;
  inputs: Port[];
  outputs: Port[];
  uniforms: Record<string, unknown>;
  collapsed?: boolean;
  inputDataType?: DataType;
  inputMode?: InputMode;
  imageDataUrl?: string;
  imageFileName?: string;
  imageWidth?: number;
  imageHeight?: number;
  fbFormat?: FramebufferFormat;
  fbWidth?: number;
  fbHeight?: number;
  fbStride?: number;
  rawDataUrl?: string;
  rawFileName?: string;
  texFilter?: TextureFilter;
  texWrap?: TextureWrap;
  width?: number;
  height?: number;
  autoSize?: boolean;
  resolvedWidth?: number;
  resolvedHeight?: number;
  outFormat?: FramebufferFormat;
  [key: string]: unknown;
}

export interface ProjectFile {
  version: string;
  name: string;
  createdAt: string;
  updatedAt: string;
  graph: {
    nodes: Array<{
      id: string;
      type: string;
      position: { x: number; y: number };
      data: ShaderNodeData;
    }>;
    edges: Array<{
      id: string;
      source: string;
      sourceHandle: string;
      target: string;
      targetHandle: string;
    }>;
  };
}

export const DATA_TYPE_COLORS: Record<DataType, string> = {
  float: '#4fc3f7',
  int: '#81c784',
  uint: '#66bb6a',
  bool: '#ffb74d',
  vec2: '#ba68c8',
  vec3: '#e57373',
  vec4: '#f06292',
  ivec2: '#a1887f',
  ivec3: '#90a4ae',
  ivec4: '#7986cb',
  uvec2: '#78909c',
  uvec3: '#8d6e63',
  uvec4: '#5c6bc0',
  bvec2: '#ffa726',
  bvec3: '#ff7043',
  bvec4: '#ab47bc',
  mat2: '#4db6ac',
  mat3: '#4dd0e1',
  mat4: '#4fc3f7',
  sampler2D: '#aed581',
  samplerCube: '#dce775',
};

export const GLSL_VALID_TYPES: DataType[] = [
  'float', 'int', 'uint', 'bool',
  'vec2', 'vec3', 'vec4',
  'ivec2', 'ivec3', 'ivec4',
  'uvec2', 'uvec3', 'uvec4',
  'bvec2', 'bvec3', 'bvec4',
  'mat2', 'mat3', 'mat4',
  'sampler2D', 'samplerCube',
];
