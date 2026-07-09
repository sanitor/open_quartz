export type { ShaderEntry as ShaderTemplate } from './shaders';
export { CUSTOM_SHADER_CODE, CUSTOM_2IN1_SHADER } from './shaders';
export { shaderGroups } from './shaders';

import { shaderGroups } from './shaders';

export const predefinedShaders = shaderGroups.flatMap(g => g.items);
