import catalog from './catalog.json';

export interface ShaderEntry {
  label: string;
  code: string;
}

export interface ShaderPreset extends ShaderEntry {
  templateName: string;
  shaderTemplateId: string | null;
}

export interface ShaderGroup {
  category: string;
  items: ShaderEntry[];
}

const catalogGroups: { category: string; items: ShaderPreset[] }[] = catalog.groups;

export const customShaderTemplates: ShaderPreset[] = catalogGroups.find(
  (group) => group.category === 'CUSTOM',
)!.items;

export const CUSTOM_SHADER_CODE = customShaderTemplates[0].code;
export const CUSTOM_2IN1_SHADER = customShaderTemplates[1].code;

export const shaderGroups: ShaderGroup[] = catalogGroups.filter(
  (group) => group.category !== 'CUSTOM',
);

function shadersIn(category: string): ShaderEntry[] {
  return shaderGroups.find((group) => group.category === category)!.items;
}

export const filterShaders = shadersIn('FILTER');
export const colorShaders = shadersIn('COLOR');
export const generatorShaders = shadersIn('GENERATOR');
export const blendShaders = shadersIn('BLEND');
export const distortionShaders = shadersIn('DISTORTION');
export const feedbackShaders = shadersIn('FEEDBACK');

export const predefinedShaders = shaderGroups.flatMap((group) => group.items);

export const SHADER_TEMPLATES: ReadonlyMap<string, ShaderEntry> = new Map(
  predefinedShaders.map((shader) => [shader.label, shader]),
);
