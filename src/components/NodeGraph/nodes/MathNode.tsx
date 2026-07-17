import { Handle, Position, type NodeProps, type Node } from '@xyflow/react';
import type { ShaderNodeData, DataType } from '../../../types';
import { DATA_TYPE_COLORS } from '../../../types';
import { useGraphStore } from '../../../store/useGraphStore';
import { MATH_OPS } from '../../../engine/mathOps';

const MATH_ACCENT = '#f59e0b';
const ROW_H = 22;
const HEADER_H = 26;

const OP_SYMBOLS: Record<string, string> = {
  add: '+', subtract: '−', multiply: '×', divide: '÷',
  negate: '−x', modulo: '%',
  min: 'min', max: 'max', clamp: 'clamp', saturate: 'sat',
  step: 'step', smoothstep: 'sstep', abs: '|x|', sign: '±',
  sin: 'sin', cos: 'cos', tan: 'tan',
  asin: 'asin', acos: 'acos', atan: 'atan',
  pow: 'xⁿ', sqrt: '√', exp: 'eˣ', log: 'ln',
  mix: 'mix',
  floor: '⌊x⌋', ceil: '⌈x⌉', round: '≈', fract: 'frac',
};

type MathNodeType = Node<ShaderNodeData>;

export function MathNode({ data, selected }: NodeProps<MathNodeType>) {
  const edges = useGraphStore((s) => s.edges);
  const nodes = useGraphStore((s) => s.nodes);
  const op = data.mathOp ? MATH_OPS[data.mathOp] : undefined;
  const symbol = data.mathOp ? (OP_SYMBOLS[data.mathOp] ?? data.label) : '?';

  // Infer actual type from connected peer
  function inferType(portId: string, isInput: boolean): DataType {
    if (isInput) {
      const edge = edges.find((e) => e.targetHandle === portId);
      if (edge) {
        const srcNode = nodes.find((n) => n.id === edge.source);
        if (srcNode) {
          const srcPort = srcNode.data.outputs.find((p) => p.id === edge.sourceHandle);
          if (srcPort && srcPort.dataType !== 'auto') return srcPort.dataType;
        }
      }
    } else {
      // Output = widest inferred input type
      const widthOrder: DataType[] = ['bool', 'int', 'uint', 'float', 'vec2', 'vec3', 'vec4'];
      let widest: DataType = 'auto';
      for (const inp of data.inputs) {
        const t = inferType(inp.id, true);
        if (t !== 'auto' && widthOrder.indexOf(t) > widthOrder.indexOf(widest)) widest = t;
      }
      return widest;
    }
    return 'auto';
  }

  function portColor(portId: string, isInput: boolean): string {
    const inferred = inferType(portId, isInput);
    if (inferred !== 'auto') return DATA_TYPE_COLORS[inferred] ?? MATH_ACCENT;
    return MATH_ACCENT;
  }

  return (
    <div
      className={`rounded-xl border bg-white shadow-sm ${
        selected ? 'border-[#007aff] shadow-md' : 'border-[#d2d2d7]'
      }`}
      style={{ minWidth: 120 }}
    >
      {/* Header */}
      <div
        className="flex items-center px-2 rounded-t-[11px]"
        style={{ height: HEADER_H, backgroundColor: MATH_ACCENT }}
      >
        <span className="text-[9px] font-semibold text-white">MATH</span>
        <span className="ml-auto text-[10px] text-white/80 font-medium">{op?.label ?? data.label}</span>
      </div>

      {/* Body: symbol + ports */}
      <div className="flex items-stretch">
        {/* Input ports column */}
        <div style={{ paddingTop: 2, paddingBottom: 2, minWidth: 36 }}>
          {data.inputs.map((port) => {
            const connected = edges.some((e) => e.targetHandle === port.id);
            const color = portColor(port.id, true);
            return (
              <div
                key={port.id}
                className="flex items-center text-[10px] text-[#1d1d1f] pl-2 pr-1"
                style={{ height: ROW_H, position: 'relative' }}
              >
                <Handle
                  type="target"
                  position={Position.Left}
                  id={port.id}
                  className="!w-2.5 !h-2.5 !border-2"
                  style={{
                    borderColor: color,
                    backgroundColor: connected ? color : 'transparent',
                  }}
                />
                <span className="ml-3 text-[#6e6e73]">{port.label}</span>
              </div>
            );
          })}
        </div>

        {/* Center symbol */}
        <div className="flex items-center justify-center px-2" style={{ minHeight: ROW_H }}>
          <span className="text-[16px] font-bold text-[#f59e0b] select-none">{symbol}</span>
        </div>

        {/* Output ports column */}
        <div style={{ paddingTop: 2, paddingBottom: 2, minWidth: 36 }}>
          {data.outputs.map((port) => {
            const color = portColor(port.id, false);
            return (
              <div
                key={port.id}
                className="flex items-center justify-end text-[10px] text-[#1d1d1f] pr-2 pl-1"
                style={{ height: ROW_H, position: 'relative' }}
              >
                <span className="mr-3 text-[#6e6e73]">{port.label}</span>
                <Handle
                  type="source"
                  position={Position.Right}
                  id={port.id}
                  className="!w-2.5 !h-2.5 !border-2 !border-white"
                  style={{ backgroundColor: color }}
                />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
