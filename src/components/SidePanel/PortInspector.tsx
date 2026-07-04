import type { Port } from '../../types';
import { DATA_TYPE_COLORS } from '../../types';

interface PortInspectorProps {
  inputs: Port[];
  outputs: Port[];
  uniforms: Record<string, unknown>;
  onUniformChange: (label: string, value: unknown) => void;
}

export function PortInspector({ inputs, outputs, uniforms, onUniformChange }: PortInspectorProps) {
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
  if (port.dataType === 'sampler2D') {
    return (
      <div className="flex items-center gap-2 text-[11px]">
        <span
          className="w-2 h-2 rounded-full inline-block flex-shrink-0"
          style={{ backgroundColor: DATA_TYPE_COLORS[port.dataType] }}
        />
        <span className="text-[#1d1d1f] font-medium w-20 truncate">{port.label}</span>
        <span className="text-[9px] text-[#aeaeb2] w-12">{port.dataType}</span>
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
      <span className="text-[9px] text-[#aeaeb2] w-12">{port.dataType}</span>
      <input
        type="text"
        value={String(value ?? '')}
        onChange={(e) => onChange(e.target.value)}
        className="flex-1 bg-white border border-[#d2d2d7] rounded px-2 py-0.5 text-[#1d1d1f] text-[11px] outline-none focus:border-[#007aff]"
        placeholder="value"
      />
    </div>
  );
}
