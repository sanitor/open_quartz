/**
 * Shader bit-true tests — real WebGL2 in a headless browser via vitest browser mode.
 *
 * Each test creates a WebGL2 context, compiles a GLSL 300 es shader,
 * renders to an FBO, reads pixels back, and verifies exact or near-exact values.
 *
 * Run with: npm run test:shaders
 */
import { describe, it, expect } from 'vitest';

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Compile + link a GLSL 300 es program. Throws on failure. */
function createProgram(gl: WebGL2RenderingContext, fsSrc: string): WebGLProgram {
  const vs = `#version 300 es
    in vec2 position;
    out vec2 v_uv;
    void main() {
      v_uv = position * 0.5 + 0.5;
      gl_Position = vec4(position, 0.0, 1.0);
    }`;

  const program = gl.createProgram()!;
  for (const [type, src] of [[gl.VERTEX_SHADER, vs], [gl.FRAGMENT_SHADER, fsSrc]] as const) {
    const shader = gl.createShader(type)!;
    gl.shaderSource(shader, src);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      throw new Error(gl.getShaderInfoLog(shader) ?? 'shader compile failed');
    }
    gl.attachShader(program, shader);
  }
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(gl.getProgramInfoLog(program) ?? 'link failed');
  }
  return program;
}

/** Upload a flat RGBA Uint8Array as a NEAREST-filtered texture. */
function uploadTexture(gl: WebGL2RenderingContext, w: number, h: number, pixels: Uint8Array): WebGLTexture {
  const tex = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, tex);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  return tex;
}

/** Render a full-screen quad with the given program and read back RGBA pixels. */
function renderAndRead(
  gl: WebGL2RenderingContext, program: WebGLProgram, w: number, h: number,
  inputTex?: WebGLTexture,
): Uint8Array {
  // FBO
  const fbo = gl.createFramebuffer()!;
  const outTex = gl.createTexture()!;
  gl.bindTexture(gl.TEXTURE_2D, outTex);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
  gl.bindFramebuffer(gl.FRAMEBUFFER, fbo);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, outTex, 0);

  // Draw
  gl.viewport(0, 0, w, h);
  gl.useProgram(program);

  // Bind input texture AFTER FBO setup (which clobbers TEXTURE_2D binding)
  if (inputTex) {
    gl.activeTexture(gl.TEXTURE0);
    gl.bindTexture(gl.TEXTURE_2D, inputTex);
    gl.uniform1i(gl.getUniformLocation(program, 'inputImage'), 0);
  }

  const posLoc = gl.getAttribLocation(program, 'position');
  const buf = gl.createBuffer()!;
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]), gl.STATIC_DRAW);
  gl.enableVertexAttribArray(posLoc);
  gl.vertexAttribPointer(posLoc, 2, gl.FLOAT, false, 0, 0);
  gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);

  // Readback
  const output = new Uint8Array(w * h * 4);
  gl.readPixels(0, 0, w, h, gl.RGBA, gl.UNSIGNED_BYTE, output);
  return output;
}

/** Create a 4×4 WebGL2 context. */
function makeGl(): WebGL2RenderingContext {
  const canvas = document.createElement('canvas');
  canvas.width = 4;
  canvas.height = 4;
  const gl = canvas.getContext('webgl2');
  if (!gl) throw new Error('WebGL2 not available');
  return gl;
}

/** Make a solid-color 4×4 RGBA input texture. */
function solidInput(r: number, g: number, b: number, a = 255): Uint8Array {
  const pixels = new Uint8Array(4 * 4 * 4);
  for (let i = 0; i < 16; i++) {
    pixels[i * 4] = r;
    pixels[i * 4 + 1] = g;
    pixels[i * 4 + 2] = b;
    pixels[i * 4 + 3] = a;
  }
  return pixels;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Shader bit-true (WebGL2)', () => {

  it('Identity: output equals input pixel-exact', () => {
    const gl = makeGl();
    const fs = `#version 300 es
      precision highp float;
      uniform sampler2D inputImage;
      in vec2 v_uv;
      out vec4 fragColor;
      void main() { fragColor = texture(inputImage, v_uv); }`;

    const input = new Uint8Array([
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
      100, 150, 200, 255,  50, 100, 150, 255,  200, 50, 100, 255,  25, 75, 125, 255,
    ]);

    const program = createProgram(gl, fs);
    const tex = uploadTexture(gl, 4, 4, input);
    const output = renderAndRead(gl, program, 4, 4, tex);
    expect(Array.from(output)).toEqual(Array.from(input));
  });

  it('Invert: (255,0,0) → (0,255,255)', () => {
    const gl = makeGl();
    const fs = `#version 300 es
      precision highp float;
      uniform sampler2D inputImage;
      in vec2 v_uv;
      out vec4 fragColor;
      void main() {
        vec4 c = texture(inputImage, v_uv);
        fragColor = vec4(1.0 - c.rgb, c.a);
      }`;

    const program = createProgram(gl, fs);
    const tex = uploadTexture(gl, 4, 4, solidInput(255, 0, 0));
    const output = renderAndRead(gl, program, 4, 4, tex);
    // Every pixel should be (0, 255, 255, 255)
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(0);
      expect(output[i * 4 + 1]).toBe(255);
      expect(output[i * 4 + 2]).toBe(255);
      expect(output[i * 4 + 3]).toBe(255);
    }
  });

  it('Grayscale: pure green → luminance ≈ 150', () => {
    const gl = makeGl();
    const fs = `#version 300 es
      precision highp float;
      uniform sampler2D inputImage;
      in vec2 v_uv;
      out vec4 fragColor;
      void main() {
        vec4 c = texture(inputImage, v_uv);
        float gray = dot(c.rgb, vec3(0.299, 0.587, 0.114));
        fragColor = vec4(vec3(gray), c.a);
      }`;

    const program = createProgram(gl, fs);
    const tex = uploadTexture(gl, 4, 4, solidInput(0, 255, 0));
    const output = renderAndRead(gl, program, 4, 4, tex);
    const expected = Math.round(0.587 * 255);  // 150
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBeGreaterThanOrEqual(expected - 2);
      expect(output[i * 4]).toBeLessThanOrEqual(expected + 2);
      expect(output[i * 4 + 1]).toBe(output[i * 4]);  // R=G=B
      expect(output[i * 4 + 2]).toBe(output[i * 4]);
      expect(output[i * 4 + 3]).toBe(255);
    }
  });

  it('Constant output: shader ignoring input produces exact value', () => {
    const gl = makeGl();
    const fs = `#version 300 es
      precision highp float;
      out vec4 fragColor;
      void main() { fragColor = vec4(0.5, 0.25, 0.75, 1.0); }`;

    const program = createProgram(gl, fs);
    const output = renderAndRead(gl, program, 4, 4);
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(128);      // 0.5 * 255 = 127.5 → 128
      expect(output[i * 4 + 1]).toBe(64);   // 0.25 * 255 = 63.75 → 64
      expect(output[i * 4 + 2]).toBe(191);  // 0.75 * 255 = 191.25 → 191
      expect(output[i * 4 + 3]).toBe(255);
    }
  });

  it('Alpha passthrough: input alpha preserved', () => {
    const gl = makeGl();
    const fs = `#version 300 es
      precision highp float;
      uniform sampler2D inputImage;
      in vec2 v_uv;
      out vec4 fragColor;
      void main() { fragColor = texture(inputImage, v_uv); }`;

    const program = createProgram(gl, fs);
    const tex = uploadTexture(gl, 4, 4, solidInput(100, 200, 50, 128));
    gl.disable(gl.BLEND);
    const output = renderAndRead(gl, program, 4, 4, tex);
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(100);
      expect(output[i * 4 + 1]).toBe(200);
      expect(output[i * 4 + 2]).toBe(50);
      expect(output[i * 4 + 3]).toBe(128);
    }
  });

  it('Channel swap: RGB → BRG', () => {
    const gl = makeGl();
    const fs = `#version 300 es
      precision highp float;
      uniform sampler2D inputImage;
      in vec2 v_uv;
      out vec4 fragColor;
      void main() {
        vec4 c = texture(inputImage, v_uv);
        fragColor = vec4(c.b, c.r, c.g, c.a);
      }`;

    const program = createProgram(gl, fs);
    const tex = uploadTexture(gl, 4, 4, solidInput(200, 100, 50));
    const output = renderAndRead(gl, program, 4, 4, tex);
    for (let i = 0; i < 16; i++) {
      expect(output[i * 4]).toBe(50);
      expect(output[i * 4 + 1]).toBe(200);
      expect(output[i * 4 + 2]).toBe(100);
      expect(output[i * 4 + 3]).toBe(255);
    }
  });
});
