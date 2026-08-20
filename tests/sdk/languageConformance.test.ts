import { describe, expect, it } from 'vitest';
import { Project } from '../../src/sdk';

const EXPECTED_OBJECTS = [
  'OpenQuartzClient', 'Project', 'Graph', 'Node', 'Port', 'Player', 'Resource', 'Output', 'Catalog',
] as const;

const JAVA_SOURCE: Record<(typeof EXPECTED_OBJECTS)[number], string> = Object.fromEntries(
  EXPECTED_OBJECTS.map((name) => [name, `java/sdk/src/main/java/com/sanitor/openquartz/${name}.java`]),
) as Record<(typeof EXPECTED_OBJECTS)[number], string>;

describe('Rust TypeScript Java public proxy conformance', () => {
  it('keeps the language object graph complete and transport-free', async () => {
    const sdk = await import('../../src/sdk');
    for (const objectName of EXPECTED_OBJECTS) {
      expect(sdk[objectName]).toBeTypeOf('function');
      const source = await import(/* @vite-ignore */ `../../${JAVA_SOURCE[objectName]}?raw`)
        .then((module) => module.default as string);
      expect(source).toMatch(new RegExp(`(?:class|record) ${objectName}`));
      expect(source).not.toContain('PipelineRuntime');
      expect(source).not.toContain('ExecutionEngine');
    }
  });

  it('keeps the shared project file version and object behavior', () => {
    const project = new Project('Conformance');
    const file = project.toFile();
    expect(file).toMatchObject({ version: '0.4.0', name: 'Conformance' });
  });
});
