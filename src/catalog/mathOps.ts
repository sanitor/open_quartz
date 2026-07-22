import type { Port, DataType } from '../types';

export interface MathOpDef {
  id: string;
  label: string;
  category: string;
  inputCount: number; // 1 = unary, 2 = binary, 3 = ternary
  compute: (inputs: number[]) => number;
}

export const MATH_OPS: Record<string, MathOpDef> = {
  // Arithmetic
  add:      { id: 'add',      label: 'Add',      category: 'Arithmetic', inputCount: 2, compute: ([a, b]) => a + b },
  subtract: { id: 'subtract', label: 'Subtract', category: 'Arithmetic', inputCount: 2, compute: ([a, b]) => a - b },
  multiply: { id: 'multiply', label: 'Multiply', category: 'Arithmetic', inputCount: 2, compute: ([a, b]) => a * b },
  divide:   { id: 'divide',   label: 'Divide',   category: 'Arithmetic', inputCount: 2, compute: ([a, b]) => b !== 0 ? a / b : 0 },
  negate:   { id: 'negate',   label: 'Negate',   category: 'Arithmetic', inputCount: 1, compute: ([a]) => -a },
  modulo:   { id: 'modulo',   label: 'Modulo',   category: 'Arithmetic', inputCount: 2, compute: ([a, b]) => b !== 0 ? a % b : 0 },

  // Range
  min:        { id: 'min',        label: 'Min',        category: 'Range', inputCount: 2, compute: ([a, b]) => Math.min(a, b) },
  max:        { id: 'max',        label: 'Max',        category: 'Range', inputCount: 2, compute: ([a, b]) => Math.max(a, b) },
  clamp:      { id: 'clamp',      label: 'Clamp',      category: 'Range', inputCount: 3, compute: ([a, b, c]) => Math.min(Math.max(a, b), c) },
  saturate:   { id: 'saturate',   label: 'Saturate',   category: 'Range', inputCount: 1, compute: ([a]) => Math.min(Math.max(a, 0), 1) },
  step:       { id: 'step',       label: 'Step',       category: 'Range', inputCount: 2, compute: ([a, b]) => b >= a ? 1 : 0 },
  smoothstep: { id: 'smoothstep', label: 'Smoothstep', category: 'Range', inputCount: 3, compute: ([a, b, c]) => { const t = Math.min(Math.max((c - a) / (b - a), 0), 1); return t * t * (3 - 2 * t); } },
  abs:        { id: 'abs',        label: 'Abs',        category: 'Range', inputCount: 1, compute: ([a]) => Math.abs(a) },
  sign:       { id: 'sign',       label: 'Sign',       category: 'Range', inputCount: 1, compute: ([a]) => Math.sign(a) },

  // Trigonometry
  sin:  { id: 'sin',  label: 'Sin',  category: 'Trigonometry', inputCount: 1, compute: ([a]) => Math.sin(a) },
  cos:  { id: 'cos',  label: 'Cos',  category: 'Trigonometry', inputCount: 1, compute: ([a]) => Math.cos(a) },
  tan:  { id: 'tan',  label: 'Tan',  category: 'Trigonometry', inputCount: 1, compute: ([a]) => Math.tan(a) },
  asin: { id: 'asin', label: 'Asin', category: 'Trigonometry', inputCount: 1, compute: ([a]) => Math.asin(a) },
  acos: { id: 'acos', label: 'Acos', category: 'Trigonometry', inputCount: 1, compute: ([a]) => Math.acos(a) },
  atan: { id: 'atan', label: 'Atan', category: 'Trigonometry', inputCount: 1, compute: ([a]) => Math.atan(a) },

  // Exponential
  pow:  { id: 'pow',  label: 'Pow',  category: 'Exponential', inputCount: 2, compute: ([a, b]) => Math.pow(a, b) },
  sqrt: { id: 'sqrt', label: 'Sqrt', category: 'Exponential', inputCount: 1, compute: ([a]) => Math.sqrt(a) },
  exp:  { id: 'exp',  label: 'Exp',  category: 'Exponential', inputCount: 1, compute: ([a]) => Math.exp(a) },
  log:  { id: 'log',  label: 'Log',  category: 'Exponential', inputCount: 1, compute: ([a]) => Math.log(a) },

  // Interpolation
  mix: { id: 'mix', label: 'Mix', category: 'Interpolation', inputCount: 3, compute: ([a, b, c]) => a * (1 - c) + b * c },

  // Rounding
  floor: { id: 'floor', label: 'Floor', category: 'Rounding', inputCount: 1, compute: ([a]) => Math.floor(a) },
  ceil:  { id: 'ceil',  label: 'Ceil',  category: 'Rounding', inputCount: 1, compute: ([a]) => Math.ceil(a) },
  round: { id: 'round', label: 'Round', category: 'Rounding', inputCount: 1, compute: ([a]) => Math.round(a) },
  fract: { id: 'fract', label: 'Fract', category: 'Rounding', inputCount: 1, compute: ([a]) => a - Math.floor(a) },
};

export const MATH_CATEGORIES: { category: string; ops: string[] }[] = [
  { category: 'Arithmetic',    ops: ['add', 'subtract', 'multiply', 'divide', 'negate', 'modulo'] },
  { category: 'Range',         ops: ['min', 'max', 'clamp', 'saturate', 'step', 'smoothstep', 'abs', 'sign'] },
  { category: 'Trigonometry',  ops: ['sin', 'cos', 'tan', 'asin', 'acos', 'atan'] },
  { category: 'Exponential',   ops: ['pow', 'sqrt', 'exp', 'log'] },
  { category: 'Interpolation', ops: ['mix'] },
  { category: 'Rounding',      ops: ['floor', 'ceil', 'round', 'fract'] },
];

const PORT_LABELS = ['a', 'b', 'c'] as const;
const AUTO: DataType = 'auto';

export function getMathPorts(op: MathOpDef): { inputs: Port[]; outputs: Port[] } {
  const inputs: Port[] = [];
  for (let i = 0; i < op.inputCount; i++) {
    const label = PORT_LABELS[i];
    inputs.push({ id: `in_${label}`, label, dataType: AUTO, direction: 'input', defaultValue: 0 });
  }
  const outputs: Port[] = [
    { id: 'out_result', label: 'result', dataType: AUTO, direction: 'output' },
  ];
  return { inputs, outputs };
}
