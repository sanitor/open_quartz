import type { ShaderEntry } from './filter';

export const feedbackShaders: ShaderEntry[] = [
  {
    label: 'Gray-Scott Reaction-Diffusion',
    code: `uniform float feedRate = 0.040;
uniform float killRate = 0.060;
uniform float dA = 0.16;
uniform float dB = 0.08;
uniform float timestep = 0.2;
uniform float iFrame;
uniform float iTime;
uniform sampler2D previousFrame;

out vec4 fragColor;

void main() {
    vec2 uv = v_uv;

    if (iFrame < 0.5) {
        float a = 1.0;
        float b = 0.0;
        vec2 center = vec2(0.5);
        vec2 halfSeed = vec2(0.1, 0.1);
        if (all(lessThan(abs(uv - center), halfSeed))) {
            a = 0.0;
            b = 1.0;
        }
        fragColor = vec4(a, b, 0.0, 1.0);
        return;
    }

    vec2 px = 1.0 / vec2(textureSize(previousFrame, 0));
    vec4 c = texture(previousFrame, uv);
    float a = c.r;
    float b = c.g;

    // 5-point Laplacian
    float la = texture(previousFrame, uv + vec2(0.0, px.y)).r
             + texture(previousFrame, uv - vec2(0.0, px.y)).r
             + texture(previousFrame, uv - vec2(px.x, 0.0)).r
             + texture(previousFrame, uv + vec2(px.x, 0.0)).r - 4.0 * a;
    float lb = texture(previousFrame, uv + vec2(0.0, px.y)).g
             + texture(previousFrame, uv - vec2(0.0, px.y)).g
             + texture(previousFrame, uv - vec2(px.x, 0.0)).g
             + texture(previousFrame, uv + vec2(px.x, 0.0)).g - 4.0 * b;

    float rxn = a * b * b;
    a += (dA * la - rxn + feedRate * (1.0 - a)) * timestep;
    b += (dB * lb + rxn - (feedRate + killRate) * b) * timestep;

    fragColor = vec4(a, b, 0.0, 1.0);
}
// 5-point Laplacian — correct isotropic diffusion on square grid.
// Spots: F=0.035,k=0.065 | Labyrinth: F=0.055,k=0.062 | Coral: F=0.030,k=0.057
// Clear Color (R=1,G=0,B=0,A=0) for A=1,B=0 initial field. timestep=1.0 at 60fps ~ 60 steps/s.
`,
  },
  {
    label: 'Field Color Map',
    code: `uniform sampler2D inputImage;

out vec4 fragColor;

vec3 turbo(float t) {
    vec3 a = vec3(0.114, 0.056, 0.566);
    vec3 b = vec3(0.376, 0.763, 0.843);
    vec3 c = vec3(0.267, 0.472, 0.090);
    vec3 d = vec3(0.905, 0.811, 0.011);
    vec3 e = vec3(0.740, 0.080, 0.055);

    vec3 r;
    if (t < 0.25) r = mix(a, b, t / 0.25);
    else if (t < 0.5) r = mix(b, c, (t - 0.25) / 0.25);
    else if (t < 0.75) r = mix(c, d, (t - 0.5) / 0.25);
    else r = mix(d, e, (t - 0.75) / 0.25);
    return r;
}

void main() {
    vec4 raw = texture(inputImage, v_uv);
    float v = raw.g;  // Read chemical B from G channel
    vec3 color = turbo(clamp(v, 0.0, 1.0));
    fragColor = vec4(color, 1.0);
}`,
  },
];
