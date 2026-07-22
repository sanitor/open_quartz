import type { ShaderEntry } from './filter';

export const feedbackShaders: ShaderEntry[] = [
  {
    label: 'Gray-Scott Reaction-Diffusion',
    code: `uniform float feedRate = 0.055;
uniform float killRate = 0.062;
uniform float dA = 0.21;
uniform float dB = 0.105;
uniform float timestep = 1.0;
uniform float iFrame;
uniform sampler2D previousFrame;

out vec4 fragColor;

void main() {
    vec2 uv = v_uv;

    if (iFrame < 0.5) {
        float a = 1.0;
        float b = 0.0;
        vec2 center = vec2(0.5);
        vec2 halfSeed = vec2(0.06);
        if (all(lessThan(abs(uv - center), halfSeed))) {
            a = 0.5;
            b = 0.25;
        }
        fragColor = vec4(a, b, 0.0, 1.0);
        return;
    }

    vec2 px = 1.0 / vec2(textureSize(previousFrame, 0));
    vec4 c = texture(previousFrame, uv);
    float a = c.r;
    float b = c.g;

    // 5-point Laplacian — sample each neighbor once, read both channels
    vec2 up    = texture(previousFrame, uv + vec2(0.0, px.y)).rg;
    vec2 down  = texture(previousFrame, uv - vec2(0.0, px.y)).rg;
    vec2 left  = texture(previousFrame, uv - vec2(px.x, 0.0)).rg;
    vec2 right = texture(previousFrame, uv + vec2(px.x, 0.0)).rg;

    float la = up.r + down.r + left.r + right.r - 4.0 * a;
    float lb = up.g + down.g + left.g + right.g - 4.0 * b;

    float rxn = a * b * b;
    a += (dA * la - rxn + feedRate * (1.0 - a)) * timestep;
    b += (dB * lb + rxn - (feedRate + killRate) * b) * timestep;

    fragColor = vec4(clamp(a, 0.0, 1.0), clamp(b, 0.0, 1.0), 0.0, 1.0);
}
// 5-point Laplacian stencil on pixel grid.
// Labyrinth: F=0.055,k=0.062 | Coral: F=0.062,k=0.061 | Spots: F=0.035,k=0.065
// Mitosis: F=0.028,k=0.062 | Worms: F=0.078,k=0.061
// Clear Color (R=1,G=0,B=0,A=0) for A=1,B=0 initial field.
`,

  },
];