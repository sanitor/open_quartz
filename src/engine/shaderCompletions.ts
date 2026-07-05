import { CompletionContext, type CompletionResult } from '@codemirror/autocomplete';
import { syntaxTree } from '@codemirror/language';
import type { EditorState } from '@codemirror/state';

const GLSL_KEYWORDS = [
  'break', 'continue', 'do', 'for', 'while', 'if', 'else',
  'discard', 'return', 'switch', 'case', 'default',
  'struct', 'void',
  'const', 'uniform', 'buffer', 'shared',
  'coherent', 'writeonly', 'readonly',
  'highp', 'mediump', 'lowp', 'precision',
  'invariant', 'flat', 'smooth',
  'in', 'out', 'inout',
  'true', 'false',
];

const GLSL_TYPES = [
  'float', 'int', 'uint', 'bool',
  'vec2', 'vec3', 'vec4',
  'ivec2', 'ivec3', 'ivec4',
  'uvec2', 'uvec3', 'uvec4',
  'bvec2', 'bvec3', 'bvec4',
  'mat2', 'mat3', 'mat4',
  'mat2x2', 'mat2x3', 'mat2x4',
  'mat3x2', 'mat3x3', 'mat3x4',
  'mat4x2', 'mat4x3', 'mat4x4',
  'sampler2D', 'sampler3D', 'samplerCube',
  'sampler2DShadow', 'samplerCubeShadow',
  'sampler2DArray', 'sampler2DArrayShadow',
  'isampler2D', 'isampler3D', 'isamplerCube',
  'usampler2D', 'usampler3D', 'usamplerCube',
  'sampler2DMS', 'isampler2DMS', 'usampler2DMS',
];

const GLSL_FUNCTIONS = [
  'radians', 'degrees', 'sin', 'cos', 'tan', 'asin', 'acos', 'atan',
  'sinh', 'cosh', 'tanh', 'asinh', 'acosh', 'atanh',
  'pow', 'exp', 'log', 'exp2', 'log2', 'sqrt', 'inversesqrt',
  'abs', 'sign', 'floor', 'ceil', 'fract', 'mod', 'min', 'max', 'clamp',
  'mix', 'step', 'smoothstep', 'length', 'distance', 'dot', 'cross',
  'normalize', 'reflect', 'refract', 'faceforward',
  'matrixCompMult', 'outerProduct', 'transpose', 'determinant', 'inverse',
  'lessThan', 'lessThanEqual', 'greaterThan', 'greaterThanEqual',
  'equal', 'notEqual', 'any', 'all', 'not',
  'texture', 'textureProj', 'textureLod', 'textureSize', 'texelFetch',
  'dFdx', 'dFdy', 'fwidth',
  'packSnorm2x16', 'unpackSnorm2x16', 'packUnorm2x16', 'unpackUnorm2x16',
  'packHalf2x16', 'unpackHalf2x16',
  'intBitsToFloat', 'uintBitsToFloat', 'floatBitsToInt', 'floatBitsToUint',
  'isnan', 'isinf',
  'trunc', 'round', 'roundEven',
];

const GLSL_BUILTINS = [
  'gl_Position', 'gl_PointSize',
  'gl_FragCoord', 'gl_FrontFacing', 'gl_PointCoord',
  'gl_FragDepth',
  'gl_VertexID', 'gl_InstanceID',
  'gl_GlobalInvocationID', 'gl_LocalInvocationID',
  'gl_WorkGroupSize', 'gl_WorkGroupID',
  'gl_NumWorkGroups',
  'gl_MaxVertexAttribs', 'gl_MaxVertexUniformVectors',
  'gl_MaxVaryingVectors', 'gl_MaxVertexOutputVectors',
  'gl_MaxFragmentInputVectors', 'gl_MaxTextureImageUnits',
  'gl_MaxFragmentUniformVectors', 'gl_MaxDrawBuffers',
];

const builtinSet = new Set([
  ...GLSL_KEYWORDS, ...GLSL_TYPES, ...GLSL_FUNCTIONS, ...GLSL_BUILTINS,
]);

function extractUserVariables(state: EditorState): string[] {
  const tree = syntaxTree(state);
  const names = new Set<string>();
  const cursor = tree.cursor();
  cursor.iterate((node) => {
    if (node.name === 'IdentifierDefinition') {
      const name = state.sliceDoc(node.from, node.to);
      if (name && !builtinSet.has(name)) {
        names.add(name);
      }
    }
  });
  return [...names];
}

export function glslCompletions(context: CompletionContext): CompletionResult | null {
  const word = context.matchBefore(/\w+/);
  if (!word && !context.explicit) return null;

  const options: { label: string; type: string }[] = [
    ...GLSL_KEYWORDS.map(k => ({ label: k, type: 'keyword' })),
    ...GLSL_TYPES.map(t => ({ label: t, type: 'type' })),
    ...GLSL_FUNCTIONS.map(f => ({ label: f, type: 'function' })),
    ...GLSL_BUILTINS.map(b => ({ label: b, type: 'constant' })),
  ];

  const userVars = extractUserVariables(context.state);
  for (const name of userVars) {
    options.push({ label: name, type: 'variable' });
  }

  return {
    from: word ? word.from : context.pos,
    options,
    validFor: /^\w*$/,
  };
}
