import { describe, it, expect } from 'vitest';
import { MATH_OPS, MATH_CATEGORIES, getMathPorts } from '../../src/catalog/mathOps';

describe('MATH_OPS compute', () => {
  // Table-driven: each row names the op, inputs, and the expected output.
  // Approximate results use toBeCloseTo; exact results use toBe.
  const cases: { op: string; inputs: number[]; expected: number; approx?: boolean }[] = [
    // Arithmetic
    { op: 'add',      inputs: [3, 5],    expected: 8 },
    { op: 'subtract', inputs: [5, 3],    expected: 2 },
    { op: 'multiply', inputs: [3, 4],    expected: 12 },
    { op: 'divide',   inputs: [10, 2],   expected: 5 },
    { op: 'negate',   inputs: [-3],      expected: 3 },
    { op: 'negate',   inputs: [7],       expected: -7 },
    { op: 'modulo',   inputs: [7, 3],    expected: 1 },
    { op: 'modulo',   inputs: [10, 5],   expected: 0 },

    // Range
    { op: 'min',      inputs: [3, 5],    expected: 3 },
    { op: 'min',      inputs: [-1, 2],   expected: -1 },
    { op: 'max',      inputs: [3, 5],    expected: 5 },
    { op: 'max',      inputs: [-4, -1],  expected: -1 },
    { op: 'clamp',    inputs: [5, 0, 1], expected: 1 },
    { op: 'clamp',    inputs: [-1, 0, 1],expected: 0 },
    { op: 'clamp',    inputs: [0.5, 0, 1], expected: 0.5 },
    { op: 'saturate', inputs: [1.5],     expected: 1 },
    { op: 'saturate', inputs: [-0.5],    expected: 0 },
    { op: 'saturate', inputs: [0.4],     expected: 0.4 },
    { op: 'step',     inputs: [0.5, 0.3],expected: 0 },
    { op: 'step',     inputs: [0.5, 0.7],expected: 1 },
    { op: 'step',     inputs: [0.5, 0.5],expected: 1 }, // equal → 1 (b >= a)
    { op: 'smoothstep', inputs: [0, 1, 0.5], expected: 0.5, approx: true },
    { op: 'smoothstep', inputs: [0, 1, 0],   expected: 0 },
    { op: 'smoothstep', inputs: [0, 1, 1],   expected: 1 },
    { op: 'abs',      inputs: [-3],      expected: 3 },
    { op: 'abs',      inputs: [3],       expected: 3 },
    { op: 'sign',     inputs: [-5],      expected: -1 },
    { op: 'sign',     inputs: [0],       expected: 0 },
    { op: 'sign',     inputs: [42],      expected: 1 },

    // Trigonometry
    { op: 'sin',  inputs: [0],              expected: 0 },
    { op: 'sin',  inputs: [Math.PI / 2],    expected: 1, approx: true },
    { op: 'cos',  inputs: [0],              expected: 1 },
    { op: 'cos',  inputs: [Math.PI],        expected: -1, approx: true },
    { op: 'tan',  inputs: [0],              expected: 0 },
    { op: 'asin', inputs: [1],              expected: Math.PI / 2, approx: true },
    { op: 'asin', inputs: [0],              expected: 0 },
    { op: 'acos', inputs: [1],              expected: 0, approx: true },
    { op: 'atan', inputs: [0],              expected: 0 },
    { op: 'atan', inputs: [1],              expected: Math.PI / 4, approx: true },

    // Exponential
    { op: 'pow',  inputs: [2, 3],  expected: 8 },
    { op: 'pow',  inputs: [5, 0],  expected: 1 },
    { op: 'sqrt', inputs: [4],     expected: 2 },
    { op: 'sqrt', inputs: [0],     expected: 0 },
    { op: 'exp',  inputs: [0],     expected: 1 },
    { op: 'exp',  inputs: [1],     expected: Math.E, approx: true },
    { op: 'log',  inputs: [1],     expected: 0 },
    { op: 'log',  inputs: [Math.E],expected: 1, approx: true },

    // Interpolation
    { op: 'mix', inputs: [0, 10, 0.3], expected: 3, approx: true },
    { op: 'mix', inputs: [0, 10, 0],   expected: 0 },
    { op: 'mix', inputs: [0, 10, 1],   expected: 10 },

    // Rounding
    { op: 'floor', inputs: [3.7],  expected: 3 },
    { op: 'floor', inputs: [-1.2], expected: -2 },
    { op: 'ceil',  inputs: [3.2],  expected: 4 },
    { op: 'ceil',  inputs: [-1.8], expected: -1 },
    { op: 'round', inputs: [3.5],  expected: 4 },
    { op: 'round', inputs: [3.4],  expected: 3 },
    { op: 'fract', inputs: [3.7],  expected: 0.7, approx: true },
    { op: 'fract', inputs: [1.0],  expected: 0 },
    { op: 'fract', inputs: [-0.3], expected: 0.7, approx: true }, // -0.3 - floor(-0.3) = -0.3 - (-1) = 0.7
  ];

  it.each(cases)('$op($inputs) = $expected', ({ op, inputs, expected, approx }) => {
    const result = MATH_OPS[op].compute(inputs);
    if (approx) {
      expect(result).toBeCloseTo(expected, 5);
    } else {
      expect(result).toBe(expected);
    }
  });

  describe('safe division by zero', () => {
    it('divide(1, 0) returns 0 instead of Infinity', () => {
      expect(MATH_OPS.divide.compute([1, 0])).toBe(0);
    });

    it('divide(0, 0) returns 0 instead of NaN', () => {
      expect(MATH_OPS.divide.compute([0, 0])).toBe(0);
    });

    it('modulo(1, 0) returns 0 instead of NaN', () => {
      expect(MATH_OPS.modulo.compute([1, 0])).toBe(0);
    });

    it('modulo(0, 0) returns 0 instead of NaN', () => {
      expect(MATH_OPS.modulo.compute([0, 0])).toBe(0);
    });
  });

  describe('smoothstep Hermite interpolation', () => {
    it('produces monotonically increasing values across [0,1]', () => {
      let prev = -1;
      for (let i = 0; i <= 10; i++) {
        const t = i / 10;
        const result = MATH_OPS.smoothstep.compute([0, 1, t]);
        expect(result).toBeGreaterThanOrEqual(prev);
        prev = result;
      }
    });

    it('clamps below the edge to 0', () => {
      expect(MATH_OPS.smoothstep.compute([0, 1, -0.5])).toBe(0);
    });

    it('clamps above the edge to 1', () => {
      expect(MATH_OPS.smoothstep.compute([0, 1, 1.5])).toBe(1);
    });
  });
});

describe('getMathPorts', () => {
  it('returns 1 input + 1 output for a unary op', () => {
    const { inputs, outputs } = getMathPorts(MATH_OPS.negate);
    expect(inputs).toHaveLength(1);
    expect(outputs).toHaveLength(1);
    expect(inputs[0].id).toBe('in_a');
    expect(outputs[0].id).toBe('out_result');
  });

  it('returns 2 inputs + 1 output for a binary op', () => {
    const { inputs, outputs } = getMathPorts(MATH_OPS.add);
    expect(inputs).toHaveLength(2);
    expect(outputs).toHaveLength(1);
    expect(inputs[0].id).toBe('in_a');
    expect(inputs[1].id).toBe('in_b');
  });

  it('returns 3 inputs + 1 output for a ternary op', () => {
    const { inputs, outputs } = getMathPorts(MATH_OPS.clamp);
    expect(inputs).toHaveLength(3);
    expect(outputs).toHaveLength(1);
    expect(inputs[0].id).toBe('in_a');
    expect(inputs[1].id).toBe('in_b');
    expect(inputs[2].id).toBe('in_c');
  });

  it('all ports have dataType "auto"', () => {
    const { inputs, outputs } = getMathPorts(MATH_OPS.mix);
    for (const p of [...inputs, ...outputs]) {
      expect(p.dataType).toBe('auto');
    }
  });

  it('input ports have direction "input", output has "output"', () => {
    const { inputs, outputs } = getMathPorts(MATH_OPS.smoothstep);
    for (const p of inputs) expect(p.direction).toBe('input');
    for (const p of outputs) expect(p.direction).toBe('output');
  });

  it('input ports have defaultValue 0', () => {
    const { inputs } = getMathPorts(MATH_OPS.add);
    for (const p of inputs) {
      expect(p.defaultValue).toBe(0);
    }
  });
});

describe('MATH_CATEGORIES', () => {
  it('contains exactly 6 categories', () => {
    expect(MATH_CATEGORIES).toHaveLength(6);
  });

  it('every op referenced in categories exists in MATH_OPS', () => {
    for (const cat of MATH_CATEGORIES) {
      for (const opId of cat.ops) {
        expect(MATH_OPS).toHaveProperty(opId);
      }
    }
  });

  it('every op in MATH_OPS appears in exactly one category', () => {
    const allCategorized = MATH_CATEGORIES.flatMap(c => c.ops);
    const opKeys = Object.keys(MATH_OPS);
    // Every op is listed
    for (const key of opKeys) {
      expect(allCategorized).toContain(key);
    }
    // No duplicates
    expect(new Set(allCategorized).size).toBe(allCategorized.length);
    // Covers all ops
    expect(allCategorized.length).toBe(opKeys.length);
  });

  it('each op category field matches its MATH_CATEGORIES grouping', () => {
    for (const cat of MATH_CATEGORIES) {
      for (const opId of cat.ops) {
        expect(MATH_OPS[opId].category).toBe(cat.category);
      }
    }
  });
});
