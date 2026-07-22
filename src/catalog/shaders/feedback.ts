import type { ShaderEntry } from './filter';

export const feedbackShaders: ShaderEntry[] = [
  {
    label: 'Gray-Scott Reaction-Diffusion',
    code: `@fragment
fn main(@location(0) v_uv: vec2f) -> @location(0) vec4f {
    let uv = v_uv;

    if (iFrame < 0.5) {
        var a = 1.0;
        var b = 0.0;
        let center = vec2f(0.5);
        let halfSeed = vec2f(0.06);
        let d = abs(uv - center);
        if (d.x < halfSeed.x && d.y < halfSeed.y) {
            a = 0.5;
            b = 0.25;
        }
        return vec4f(a, b, 0.0, 1.0);
    }

    let px = vec2f(1.0) / vec2f(textureDimensions(previousFrame));
    let c = textureSample(previousFrame, previousFrameSampler, uv);
    var a = c.r;
    var b = c.g;

    let up    = textureSample(previousFrame, previousFrameSampler, uv + vec2f(0.0, px.y)).rg;
    let down  = textureSample(previousFrame, previousFrameSampler, uv - vec2f(0.0, px.y)).rg;
    let left  = textureSample(previousFrame, previousFrameSampler, uv - vec2f(px.x, 0.0)).rg;
    let right = textureSample(previousFrame, previousFrameSampler, uv + vec2f(px.x, 0.0)).rg;

    let la = up.r + down.r + left.r + right.r - 4.0 * a;
    let lb = up.g + down.g + left.g + right.g - 4.0 * b;

    let rxn = a * b * b;
    a += (dA * la - rxn + feedRate * (1.0 - a)) * timestep;
    b += (dB * lb + rxn - (feedRate + killRate) * b) * timestep;

    return vec4f(clamp(a, 0.0, 1.0), clamp(b, 0.0, 1.0), 0.0, 1.0);
}
// 5-point Laplacian stencil on pixel grid.
// Labyrinth: F=0.055,k=0.062 | Coral: F=0.062,k=0.061 | Spots: F=0.035,k=0.065
// Mitosis: F=0.028,k=0.062 | Worms: F=0.078,k=0.061
// Clear Color (R=1,G=0,B=0,A=0) for A=1,B=0 initial field.
`,

  },
];
