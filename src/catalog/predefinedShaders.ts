import { catalogSnapshot } from '../sdk/catalog';
import type { ShaderGroupDescriptor, ShaderTemplateDescriptor } from '../sdk/catalog';

export type ShaderTemplate = ShaderTemplateDescriptor;

const snapshot = catalogSnapshot();

export const shaderGroups: ShaderGroupDescriptor[] = snapshot.shaderGroups;
export const predefinedShaders: ShaderTemplateDescriptor[] = shaderGroups.flatMap((group) => group.items);

export const SHADER_TEMPLATES: ReadonlyMap<string, ShaderTemplateDescriptor> = new Map(
  predefinedShaders.map((shader) => [shader.label, shader]),
);
