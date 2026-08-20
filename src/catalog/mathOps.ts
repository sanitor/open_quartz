import type { Port, DataType } from '../types';
import { catalogSnapshot } from '../sdk/catalog';

export interface MathOpDef {
  id: string;
  label: string;
  category: string;
  inputCount: number; // 1 = unary, 2 = binary, 3 = ternary
}

const snapshot = catalogSnapshot();

export const MATH_OPS: Record<string, MathOpDef> = Object.fromEntries(
  snapshot.mathOps.map(({ id, label, category, inputCount }) => [
    id,
    { id, label, category, inputCount },
  ]),
);

export const MATH_CATEGORIES: { category: string; ops: string[] }[] = snapshot.mathCategories;

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
