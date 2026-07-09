import type { ShaderEntry } from './filter';
import { filterShaders } from './filter';
import { colorShaders } from './color';
import { generatorShaders } from './generator';
import { blendShaders } from './blend';
import { distortionShaders } from './distortion';

export type { ShaderEntry } from './filter';

export interface ShaderGroup {
  category: string;
  items: ShaderEntry[];
}

export { CUSTOM_SHADER_CODE, CUSTOM_2IN1_SHADER } from './templates';
export { filterShaders } from './filter';
export { colorShaders } from './color';
export { generatorShaders } from './generator';
export { blendShaders } from './blend';
export { distortionShaders } from './distortion';

export const shaderGroups: ShaderGroup[] = [
  { category: 'FILTER', items: filterShaders },
  { category: 'COLOR', items: colorShaders },
  { category: 'GENERATOR', items: generatorShaders },
  { category: 'BLEND', items: blendShaders },
  { category: 'DISTORTION', items: distortionShaders },
];
