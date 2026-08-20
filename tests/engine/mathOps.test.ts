import { describe, it, expect } from 'vitest';
import { MATH_OPS, MATH_CATEGORIES, getMathPorts } from '../../src/catalog/mathOps';

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
