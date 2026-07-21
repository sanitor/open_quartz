export type { ShaderEntry as ShaderTemplate } from './shaders';
export { CUSTOM_SHADER_CODE, CUSTOM_2IN1_SHADER } from './shaders';
export { shaderGroups } from './shaders';

import { shaderGroups } from './shaders';
import type { ShaderEntry } from './shaders';

export const predefinedShaders = shaderGroups.flatMap(g => g.items);

/** Lookup map: templateId (label) → ShaderEntry. */
export const SHADER_TEMPLATES: ReadonlyMap<string, ShaderEntry> = new Map(
  predefinedShaders.map(s => [s.label, s]),
);
