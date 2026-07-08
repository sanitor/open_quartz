import type { Port } from '../../types';
import { DATA_TYPE_COLORS } from '../../types';

const BUILTIN_UNIFORMS: Record<string, true> = {
  iTime: true,
  iTimeDelta: true,
  iFrame: true,
  iDate: true,
  iMouse: true,
  iResolution: true,
};

interface PortInspectorProps {
  inputs: Port[];
  outputs: Port[];
  uniforms: Record<string, unknown>;
  onUniformChange: (label: string, value: unknown) => void;
  showOutputs?: boolean;
}

export function PortInspector({ inputs, outputs, uniforms, onUniformChange, showOutputs = true }: PortInspectorProps) {
  return (
    <div className="space-y-4">
      {/* Inputs */}
      <div>
        <h4 className="text-[11px] font-semibold text-[#86868b] uppercase tracking-wider mb-2">Inputs</h4>
        {inputs.length === 0 && (
          <p className="text-[11px] text-[#aeaeb2] italic">Add uniforms to your shader to create inputs</p>
        )}
        <div className="space-y-1.5">
          {inputs.map((port) => (
            <PortRow
              key={port.id}
              port={port}
              value={uniforms[port.label] ?? port.defaultValue ?? ''}
              onChange={(v) => onUniformChange(port.label, v)}
            />
          ))}
        </div>
      </div>

      {/* Outputs */}
      {showOutputs && (
        <div>
          <h4 className="text-[11px] font-semibold text-[#86868b] uppercase tracking-wider mb-2">Outputs</h4>
          {outputs.length === 0 && (
            <p className="text-[11px] text-[#aeaeb2] italic">Add out variables to your shader to create outputs</p>
          )}
          <div className="space-y-1">
            {outputs.map((port) => (
              <div key={port.id} className="flex items-center gap-2 text-[11px] text-[#1d1d1f]">
                <span
                  className="w-2 h-2 rounded-full inline-block"
                  style={{ backgroundColor: DATA_TYPE_COLORS[port.dataType] }}
                />
                <span className="font-medium">{port.label}</span>
                <span className="text-[9px] text-[#aeaeb2]">{port.dataType}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function PortRow({
  port,
  value,
  onChange,
}: {
  port: Port;
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const isBuiltin = BUILTIN_UNIFORMS[port.label] === true;
  if (port.dataType === 'sampler2D') {
    return (
      <div className="flex items-center gap-2 text-[11px]">
        <span
          className="w-2 h-2 rounded-full inline-block flex-shrink-0"
          style={{ backgroundColor: DATA_TYPE_COLORS[port.dataType] }}
        />
        <span className="text-[#1d1d1f] font-medium w-20 truncate">{port.label}</span>
        <span className="text-[9px] text-[#aeaeb2] w-12">{port.dataType}</span>
        {isBuiltin && <span className="text-[8px] text-[#007aff] bg-[#e8f2ff] rounded px-1">AUTO</span>}
        <span className="flex-1 text-[10px] text-[#aeaeb2] italic text-right">
          ← connect upstream
        </span>
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2 text-[11px]">
      <span
        className="w-2 h-2 rounded-full inline-block flex-shrink-0"
        style={{ backgroundColor: DATA_TYPE_COLORS[port.dataType] }}
      />
      <span className="text-[#1d1d1f] font-medium w-20 truncate">{port.label}</span>
      {isBuiltin && <span className="text-[8px] text-[#007aff] bg-[#e8f2ff] rounded px-1">AUTO</span>}
      <span className="text-[9px] text-[#aeaeb2] w-12">{port.dataType}</span>
      <VectorInput dataType={port.dataType} value={value} onChange={onChange} />
    </div>
  );
}

const VEC_COMPONENTS: Record<string, string[]> = {
  vec2: ['x', 'y'],
  vec3: ['x', 'y', 'z'],
  vec4: ['x', 'y', 'z', 'w'],
  ivec2: ['x', 'y'],
  ivec3: ['x', 'y', 'z'],
  ivec4: ['x', 'y', 'z', 'w'],
  uvec2: ['x', 'y'],
  uvec3: ['x', 'y', 'z'],
  uvec4: ['x', 'y', 'z', 'w'],
  bvec2: ['x', 'y'],
  bvec3: ['x', 'y', 'z'],
  bvec4: ['x', 'y', 'z', 'w'],
};

const MAT_DIMS: Record<string, number> = {
  mat2: 2,
  mat3: 3,
  mat4: 4,
};

function VectorInput({
  dataType,
  value,
  onChange,
}: {
  dataType: string;
  value: unknown;
  onChange: (v: unknown) => void;
}) {
  const comps = VEC_COMPONENTS[dataType];
  const matDim = MAT_DIMS[dataType];

  if (matDim) {
    const total = matDim * matDim;
    const arr: number[] = Array.isArray(value) ? value : new Array(total).fill(0);
    return (
      <div className="flex-1 flex flex-col gap-0.5">
        {Array.from({ length: matDim }, (_, row) => (
          <div key={row} className="flex gap-0.5">
            {Array.from({ length: matDim }, (_, col) => {
              const idx = col * matDim + row;
              return (
                <input
                  key={col}
                  type="text"
                  value={String(arr[idx] ?? 0)}
                  onChange={(e) => {
                    const next = Array.from({ length: total }, (_, k) => arr[k] ?? 0);
                    const parsed = parseFloat(e.target.value);
                    next[idx] = isNaN(parsed) ? 0 : parsed;
                    onChange(next);
                  }}
                  className="flex-1 min-w-0 bg-white border border-[#d2d2d7] rounded px-0.5 py-0.5 text-center text-[#1d1d1f] text-[10px] outline-none focus:border-[#007aff]"
                />
              );
            })}
          </div>
        ))}
      </div>
    );
  }

  if (!comps) {
    return (
      <input
        type="text"
        value={String(value ?? '')}
        onChange={(e) => onChange(e.target.value)}
        className="flex-1 bg-white border border-[#d2d2d7] rounded px-2 py-0.5 text-[#1d1d1f] text-[11px] outline-none focus:border-[#007aff]"
        placeholder="value"
      />
    );
  }

  const arr: number[] = Array.isArray(value) ? value : [0, 0, 0, 0];

  return (
    <div className="flex-1 flex flex-col gap-0.5">
      {comps.map((c, i) => (
        <div key={c} className="flex items-center gap-1">
          <span className="text-[9px] text-[#aeaeb2] font-mono w-2">{c}</span>
          <input
            type="text"
            value={String(arr[i] ?? 0)}
            onChange={(e) => {
              const next = [...comps.map((_, j) => arr[j] ?? 0)];
              const parsed = parseFloat(e.target.value);
              next[i] = isNaN(parsed) ? 0 : parsed;
              onChange(next);
            }}
            className="flex-1 bg-white border border-[#d2d2d7] rounded px-1 py-0.5 text-[#1d1d1f] text-[11px] outline-none focus:border-[#007aff]"
          />
        </div>
      ))}
    </div>
  );
}
