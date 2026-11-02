import assert from 'node:assert/strict';
import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { basename, join, resolve } from 'node:path';
import { decodePng, encodePng, makeReference, REFERENCE_LABELS } from './shader-e2e-reference.mjs';

const CATALOG_PATH = fileURLToPath(new URL('../src/shaders/catalog.json', import.meta.url));
const PHYSICAL_FIXTURES = fileURLToPath(new URL('../tests/fixtures/shaders/', import.meta.url));
const WIDTH = 64;
const HEIGHT = 64;
const BACKGROUND = [245, 245, 247];
const slug = (label) => label.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
const descriptionFor = (entry) => {
  const firstLine = entry.source.split(/\r?\n/).map((line) => line.trim()).find((line) => line.startsWith('//'));
  return `${entry.category} shader: ${firstLine?.replace(/^\/\/\s*/, '') ?? entry.label}.`;
};

function parseShaderInputs(source) {
  const declarations = [];
  const texture = /@group\(\d+\)\s*@binding\(\d+\)\s*var\s+([A-Za-z_]\w*)\s*:\s*texture_2d\s*</g;
  const uniform = /@group\(\d+\)\s*@binding\(\d+\)\s*var<uniform>\s+([A-Za-z_]\w*)\s*:\s*([A-Za-z_]\w*)/g;
  for (const match of source.matchAll(texture)) {
    if (match[1] === 'previousFrame' || match[1] === 'iChannel0') continue;
    declarations.push({ order: match.index, label: match[1], dataType: 'sampler2D', kind: 'texture' });
  }
  const typeMap = { f32: 'float', i32: 'int', u32: 'uint', bool: 'bool', vec2f: 'vec2', vec3f: 'vec3', vec4f: 'vec4', vec2i: 'ivec2', vec3i: 'ivec3', vec4i: 'ivec4', vec2u: 'uvec2', vec3u: 'uvec3', vec4u: 'uvec4' };
  for (const match of source.matchAll(uniform)) {
    if (/^(iFrame|iTime|iTimeDelta|iResolution|iMouse|iDate|iFrameRate|previousFrameSampler|iChannel\d+)$/.test(match[1])) continue;
    const dataType = typeMap[match[2]];
    assert.ok(dataType, `Unsupported shader uniform type ${match[2]} (${match[1]})`);
    declarations.push({ order: match.index, label: match[1], dataType, kind: 'uniform' });
  }
  declarations.sort((a, b) => a.order - b.order);
  assert.equal(new Set(declarations.map((port) => port.label)).size, declarations.length, 'Shader port labels must be unique');
  return declarations.map(({ order: _order, ...port }) => port);
}

function parseTemplateIdentity(item) {
  return { templateName: item.templateName, shaderTemplateId: item.shaderTemplateId ?? null };
}

function readCatalog() {
  const parsed = JSON.parse(requireCatalogText());
  assert.ok(Array.isArray(parsed.groups), 'Shader catalog must contain groups');
  const items = parsed.groups.flatMap((group) => group.items.map((item) => ({
    id: `shader-${slug(item.label)}`,
    label: item.label,
    category: group.category,
    source: item.code,
    templateName: item.templateName,
    shaderTemplateId: item.shaderTemplateId ?? null,
    inputs: parseShaderInputs(item.code),
    outputs: [{ label: 'fragColor', dataType: 'vec4' }],
  })));
  assert.equal(items.length, 34, `Expected exactly 34 catalog shader definitions; found ${items.length}`);
  assert.equal(new Set(items.map((item) => item.label)).size, items.length, 'Catalog shader labels must be unique');
  assert.equal(new Set(items.map((item) => item.id)).size, items.length, 'Catalog shader fixture IDs must be unique');
  const references = new Set(REFERENCE_LABELS);
  for (const item of items) assert.ok(references.has(item.label), `New catalog shader ${item.label} has no CPU image reference`);
  for (const label of references) assert.ok(items.some((item) => item.label === label), `CPU reference ${label} has no catalog shader`);
  return items;
}

function requireCatalogText() {
  return readCatalog.text;
}

readCatalog.text = '';
// Keep catalog JSON as the sole source of shader code; never duplicate WGSL here.
const catalogTextPromise = readFile(CATALOG_PATH, 'utf8');
const catalogText = await catalogTextPromise;
readCatalog.text = catalogText;
export const SHADER_CATALOG = Object.freeze(readCatalog().map((entry) => Object.freeze(entry)));

export function readShaderCatalog() {
  return SHADER_CATALOG.map((entry) => structuredClone(entry));
}

const UNIFORMS = Object.freeze({
  'CUSTOM SHADER': { intensity: 1.37 },
  'CUSTOM 2IN-1': { mixFactor: 0.37 },
  'Sobel Edge Detection': { intensity: 1.6 },
  Sharpen: { strength: 1.4 },
  // Binary-exact block sizes avoid nearest-sampler ties perturbed by division.
  Pixelate: { blockSize: [8, 4] },
  'Brightness/Contrast': { brightness: 0.08, contrast: 1.35 },
  'Hue Rotate': { angle: 1.1 },
  Threshold: { threshold: 0.53 },
  'Solid Color': { color: [0.22, 0.48, 0.79, 1] },
  Gradient: { colorA: [0.13, 0.25, 0.82, 1], colorB: [0.91, 0.31, 0.16, 1] },
  Checkerboard: { gridSize: [7, 9], color1: [0.12, 0.34, 0.82, 1], color2: [0.91, 0.55, 0.18, 1] },
  Noise: { scale: 13.7 },
  Circle: { circle: [0.43, 0.56, 0.29, 0.13] },
  Twirl: { radius: 0.73, angle: 2.7 },
  Ripple: { frequency: 17.3, amplitude: 0.037 },
  Displacement: { strength: 0.24 },
  Barrel: { k1: -0.19, k2: 0.035 },
  Pinch: { radius: 0.67, strength: 0.8 },
  'Gray-Scott Reaction-Diffusion': { dA: 0.21, dB: 0.105, feedRate: 0.055, killRate: 0.062, timestep: 1.0 },
});

function createChart(variant) {
  const pixels = Buffer.alloc(WIDTH * HEIGHT * 4);
  for (let y = 0; y < HEIGHT; y++) for (let x = 0; x < WIDTH; x++) {
    const offset = (y * WIDTH + x) * 4;
    let r, g, b;
    if (variant === 'a') {
      const band = Math.floor(x / 8);
      const gradient = Math.round(20 + 210 * y / 63);
      const colors = [[235, 58, 48], [255, 166, 38], [245, 223, 62], [53, 177, 94], [45, 142, 220], [101, 83, 199], [214, 73, 171], [237, 237, 237]];
      [r, g, b] = colors[band];
      const checker = ((x >> 2) + (y >> 2)) % 2;
      const circle = (x - 32) ** 2 + (y - 33) ** 2 < 144;
      const edge = x === 0 || y === 0 || x === 63 || y === 63;
      const scale = circle ? 1.08 : checker ? 0.73 : 0.88;
      r = Math.min(255, Math.round(r * scale * gradient / 230));
      g = Math.min(255, Math.round(g * scale * gradient / 230));
      b = Math.min(255, Math.round(b * scale * gradient / 230));
      if (edge) [r, g, b] = [20, 25, 35];
    } else {
      const band = Math.floor(y / 8);
      const colors = [[31, 88, 188], [243, 188, 51], [30, 155, 166], [177, 58, 101], [222, 232, 238], [96, 167, 67], [167, 93, 194], [41, 44, 53]];
      [r, g, b] = colors[band];
      const diagonal = (x + y) % 13 < 5;
      const box = x >= 12 && x < 30 && y >= 15 && y < 34;
      const circle = (x - 46) ** 2 + (y - 45) ** 2 < 90;
      const ramp = Math.round(30 + 205 * x / 63);
      const scale = box ? 1.12 : diagonal ? 0.68 : circle ? 0.82 : 0.96;
      r = Math.min(255, Math.round(r * scale * ramp / 235));
      g = Math.min(255, Math.round(g * scale * ramp / 235));
      b = Math.min(255, Math.round(b * scale * ramp / 235));
      if (x === 1 || y === 1 || x === 62 || y === 62) [r, g, b] = [248, 248, 250];
    }
    pixels[offset] = r;
    pixels[offset + 1] = g;
    pixels[offset + 2] = b;
    pixels[offset + 3] = 255;
  }
  return pixels;
}

export const FIXTURE_IMAGES = Object.freeze([
  Object.freeze({ fileName: 'fixture-a.png', pixels: createChart('a') }),
  Object.freeze({ fileName: 'fixture-b.png', pixels: createChart('b') }),
]);

function dataUrl(png) { return `data:image/png;base64,${png.toString('base64')}`; }
function sha256(buffer) { return createHash('sha256').update(buffer).digest('hex'); }
function port(data, nodeId) { return { id: `${nodeId}_${data.label}`, label: data.label, dataType: data.dataType, direction: 'input' }; }
function imageNode(texture, index, png) {
  const id = texture.imageNodeId;
  return {
    id,
    type: 'input',
    position: { x: 0, y: index === 0 ? 0 : 260 },
    data: {
      type: 'input',
      label: index === 0 ? 'Fixture A' : 'Fixture B',
      shaderCode: '',
      inputs: [],
      outputs: [{ id: `${id}_image`, label: 'image', dataType: 'sampler2D', direction: 'output' }],
      uniforms: {},
      inputDataType: 'sampler2D',
      inputMode: 'image',
      imageDataUrl: dataUrl(png),
      imageFileName: basename(texture.fixturePath),
      imageWidth: WIDTH,
      imageHeight: HEIGHT,
      texFilter: 'nearest',
      texWrap: 'clamp',
    },
  };
}
function assertUniforms(entry, uniforms) {
  const uniformInputs = entry.inputs.filter((input) => input.kind === 'uniform');
  assert.equal(Object.keys(uniforms).length, uniformInputs.length, `${entry.label} must specify every uniform exactly once`);
  for (const input of uniformInputs) {
    assert.ok(Object.hasOwn(uniforms, input.label), `${entry.label} has no explicit fixture value for ${input.label}`);
    const value = uniforms[input.label];
    assert.ok(Array.isArray(value) ? value.some((item) => item !== 0) : value !== 0, `${entry.label}.${input.label} must not be all-zero`);
  }
}

/** Build 34 runnable shader projects and export input/expected PNGs for reports. */
export async function prepareShaderFixtures(outputDir) {
  assert.ok(outputDir, 'prepareShaderFixtures(outputDir) requires an output directory');
  const output = resolve(outputDir);
  await mkdir(output, { recursive: true });
  const inputPaths = [];
  const sourceBytes = [];
  const sourceHashes = [];
  for (const image of FIXTURE_IMAGES) {
    const physicalPath = join(PHYSICAL_FIXTURES, image.fileName);
    const physical = await readFile(physicalPath);
    const decoded = decodePng(physical);
    assert.equal(decoded.width, WIDTH, `${image.fileName} width must be 64`);
    assert.equal(decoded.height, HEIGHT, `${image.fileName} height must be 64`);
    assert.deepEqual(decoded.pixels, image.pixels, `${image.fileName} pixels differ from reproducible fixture generator`);
    const outputPath = join(output, image.fileName);
    await writeFile(outputPath, physical);
    inputPaths.push(outputPath);
    sourceBytes.push(physical);
    sourceHashes.push(sha256(physical));
  }
  assert.notEqual(sourceHashes[0], sourceHashes[1], 'Dual-input fixture images must differ');
  const cases = [];
  for (const entry of SHADER_CATALOG) {
    const texturePorts = entry.inputs.filter((input) => input.kind === 'texture');
    const uniforms = structuredClone(UNIFORMS[entry.label] ?? {});
    assertUniforms(entry, uniforms);
    assert.equal(texturePorts.length, texturePorts.length > 1 ? 2 : texturePorts.length, `${entry.label}: texture fixture count is unsupported`);
    const textureInputs = texturePorts.map((input, index) => ({
      label: input.label,
      imageNodeId: index === 0 ? 'image_a' : 'image_b',
      fixturePath: inputPaths[index],
      sourcePath: join(PHYSICAL_FIXTURES, basename(inputPaths[index])),
      sha256: sourceHashes[index],
    }));
    const images = texturePorts.map((_, index) => ({ width: WIDTH, height: HEIGHT, pixels: FIXTURE_IMAGES[index].pixels }));
    const reference = makeReference(entry.label, images, uniforms);
    const id = entry.id;
    const expectedPath = join(output, `${id}-expected.png`);
    await writeFile(expectedPath, encodePng(reference));
    const projectPath = join(output, `${id}.quartz.json`);
    const inputNodes = textureInputs.map((texture, index) => imageNode(texture, index, sourceBytes[index]));
    const shaderNode = {
      id: 'shader_1',
      type: 'shader',
      position: { x: 280, y: 80 },
      data: {
        type: 'shader',
        label: entry.label,
        templateName: entry.templateName,
        shaderTemplateId: entry.shaderTemplateId,
        shaderCode: entry.source,
        inputs: entry.inputs.map((input) => port(input, 'shader_1')),
        outputs: entry.outputs.map((outputPort) => ({ id: `shader_1_${outputPort.label}`, label: outputPort.label, dataType: outputPort.dataType, direction: 'output' })),
        uniforms,
        autoSize: false,
        width: WIDTH,
        height: HEIGHT,
        texFilter: 'nearest',
        texWrap: 'clamp',
      },
    };
    const rendererNode = {
      id: 'renderer_1',
      type: 'renderer',
      position: { x: 560, y: 80 },
      data: {
        type: 'renderer',
        label: 'Renderer',
        shaderCode: '',
        inputs: [{ id: 'renderer_1_inputTexture', label: 'inputTexture', dataType: 'sampler2D', direction: 'input' }],
        outputs: [],
        uniforms: {},
      },
    };
    const project = {
      version: '0.4.0',
      name: `Shader E2E — ${entry.label}`,
      createdAt: '',
      updatedAt: '',
      graph: { nodes: [...inputNodes, shaderNode, rendererNode], edges: [] },
    };
    await writeFile(projectPath, `${JSON.stringify(project, null, 2)}\n`);
    const fixturePaths = textureInputs.map((texture) => texture.fixturePath);
    const result = {
      id,
      title: entry.label,
      label: entry.label,
      category: entry.category,
      description: descriptionFor(entry),
      source: entry.source,
      templateName: entry.templateName,
      shaderTemplateId: entry.shaderTemplateId,
      templateIdentity: parseTemplateIdentity({ templateName: entry.templateName, shaderTemplateId: entry.shaderTemplateId }),
      projectPath,
      uniforms,
      textureInputs,
      expectedPath,
      fixturePaths,
      fixtureImageCount: fixturePaths.length,
      shaderInputPorts: entry.inputs.map(({ label, dataType, kind }) => ({ label, dataType, kind })),
      outputPort: entry.outputs[0],
      inputPngs: textureInputs.map(({ fixturePath, sha256: digest, label }) => ({ label, path: fixturePath, sha256: digest })),
      oracle: reference.oracle,
    };
    Object.defineProperty(result, 'expected', { value: reference, enumerable: false });
    cases.push(result);
  }
  assert.equal(cases.length, SHADER_CATALOG.length, 'Every catalog shader must have exactly one fixture case');
  assert.equal(new Set(cases.map((entry) => entry.id)).size, cases.length, 'Shader fixture case IDs must be unique');
  for (const entry of SHADER_CATALOG) assert.equal(cases.filter((fixture) => fixture.label === entry.label).length, 1, `Expected exactly one case for ${entry.label}`);
  return cases;
}

/** Write only the two committed physical PNGs; preparation itself copies them to its report output. */
export async function writePhysicalFixtures() {
  await mkdir(PHYSICAL_FIXTURES, { recursive: true });
  for (const image of FIXTURE_IMAGES) {
    await writeFile(join(PHYSICAL_FIXTURES, image.fileName), encodePng({ width: WIDTH, height: HEIGHT, pixels: image.pixels }));
  }
  return FIXTURE_IMAGES.map((image) => join(PHYSICAL_FIXTURES, image.fileName));
}

export const shaderFixtureAssets = Object.freeze({
  directory: PHYSICAL_FIXTURES,
  width: WIDTH,
  height: HEIGHT,
  images: FIXTURE_IMAGES.map(({ fileName, pixels }) => ({ fileName, width: WIDTH, height: HEIGHT, pixelSha256: sha256(pixels) })),
  background: BACKGROUND,
});

export async function writeFixtureAssets(outputDir) {
  return prepareShaderFixtures(outputDir);
}

export { copyFile, readFile };
