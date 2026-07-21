import { Handle, Position } from '@xyflow/react';
import type { Port } from '../../../types';
import { DATA_TYPE_COLORS } from '../../../types';

// ─── Layout constants ────────────────────────────────────────────
export const ROW_H = 26;
export const HEADER_H = 28;
const HEADER_BG = '#1e293b';           // unified dark-blue caption
const PORT_COLOR = '#8e8e93';

// ─── Menu-aligned SVG icons (match Header toolbar) ──────────────
const ICON_SIZE = 12;
const iconProps = { width: ICON_SIZE, height: ICON_SIZE, viewBox: '0 0 16 16', fill: 'none', stroke: 'currentColor' } as const;

/** First-level menu icons as JSX. */
export const MENU_ICONS: Record<string, React.ReactNode> = {
  source: (
    <svg {...iconProps} strokeWidth="1.3" strokeLinecap="round">
      <circle cx="4.5" cy="8" r="3" /><line x1="7.5" y1="8" x2="14" y2="8" />
    </svg>
  ),
  math: (
    <svg {...iconProps} strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round">
      <text x="8" y="12" textAnchor="middle" fontSize="12" fill="currentColor" stroke="none" fontWeight="bold">±</text>
    </svg>
  ),
  shader: (
    <svg {...iconProps} strokeWidth="1.2" strokeLinecap="round">
      <rect x="3.5" y="3.5" width="9" height="9" rx="1" />
      <line x1="5" y1="3.5" x2="5" y2="1.5" /><line x1="8" y1="3.5" x2="8" y2="1" /><line x1="11" y1="3.5" x2="11" y2="1.5" />
      <line x1="5" y1="12.5" x2="5" y2="14.5" /><line x1="8" y1="12.5" x2="8" y2="15" /><line x1="11" y1="12.5" x2="11" y2="14.5" />
      <line x1="3.5" y1="5" x2="1.5" y2="5" /><line x1="3.5" y1="8" x2="1" y2="8" /><line x1="3.5" y1="11" x2="1.5" y2="11" />
      <line x1="12.5" y1="5" x2="14.5" y2="5" /><line x1="12.5" y1="8" x2="15" y2="8" /><line x1="12.5" y1="11" x2="14.5" y2="11" />
    </svg>
  ),
  onnx: (
    <svg {...iconProps} strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="8" cy="8" r="6" /><path d="M5 8 L8 5 L11 8 L8 11 Z" />
    </svg>
  ),
  renderer: (
    <svg {...iconProps} strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round">
      <rect x="1.5" y="2.5" width="13" height="9" rx="1" />
      <line x1="5" y1="14" x2="11" y2="14" /><line x1="8" y1="11.5" x2="8" y2="14" />
    </svg>
  ),
};

/** Second-level Source sub-menu icons (for input nodes). */
export const SOURCE_ICONS: Record<string, React.ReactNode> = {
  system: (
    <svg {...iconProps} strokeWidth="1.3" strokeLinecap="round">
      <circle cx="8" cy="8" r="6" /><polyline points="8,4 8,8 11,10" />
    </svg>
  ),
  constants: (
    <svg {...iconProps} strokeWidth="1.3" strokeLinecap="round">
      <circle cx="8" cy="8" r="3" fill="currentColor" />
    </svg>
  ),
  external: (
    <svg {...iconProps} strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2" y="3" width="12" height="10" rx="1" /><circle cx="5.5" cy="6.5" r="1.5" /><polyline points="2,12 6,8 9,11 11,9 14,12" />
    </svg>
  ),
};

// ─── Status LED ──────────────────────────────────────────────────
export type NodeStatus = 'not-ready' | 'ready' | 'error';

const STATUS_COLORS: Record<NodeStatus, string> = {
  'not-ready': '#8e8e93',   // gray
  'ready':     '#34c759',   // green
  'error':     '#ff3b30',   // red
};

function StatusLed({ status }: { status: NodeStatus }) {
  const color = STATUS_COLORS[status];
  return (
    <span
      className="inline-block rounded-full flex-shrink-0"
      style={{
        width: 7, height: 7,
        backgroundColor: color,
        boxShadow: status === 'error' ? `0 0 4px ${color}` : undefined,
      }}
    />
  );
}

// ─── Port rows ───────────────────────────────────────────────────
export interface InputPortRowProps {
  port: Port;
  connected: boolean;
  error: boolean;
  color?: string;
  rowHeight?: number;
}

export function InputPortRow({ port, connected, error, color, rowHeight = ROW_H }: InputPortRowProps) {
  const c = color ?? PORT_COLOR;
  return (
    <div
      key={port.id}
      className="flex items-center text-[11px] text-[#1d1d1f] px-3"
      style={{ height: rowHeight, position: 'relative' }}
    >
      <Handle
        type="target"
        position={Position.Left}
        id={port.id}
        className="!w-3 !h-3 !border-2"
        style={{
          borderColor: error ? '#ff3b30' : c,
          backgroundColor: error ? '#ff3b30' : connected ? c : 'transparent',
        }}
      />
      <span className={`ml-4 ${error ? 'text-[#ff3b30] font-medium' : ''}`}>{port.label}</span>
      <span className="ml-auto text-[9px] text-[#aeaeb2]">{port.dataType}</span>
    </div>
  );
}

export interface OutputPortRowProps {
  port: Port;
  color?: string;
  rowHeight?: number;
}

export function OutputPortRow({ port, color, rowHeight = ROW_H }: OutputPortRowProps) {
  const c = color ?? (DATA_TYPE_COLORS[port.dataType] || PORT_COLOR);
  return (
    <div
      key={port.id}
      className="flex items-center justify-end text-[11px] text-[#1d1d1f] px-3"
      style={{ height: rowHeight, position: 'relative' }}
    >
      <span className="text-[9px] text-[#aeaeb2]">{port.dataType}</span>
      <span className="ml-1.5 mr-4">{port.label}</span>
      <Handle
        type="source"
        position={Position.Right}
        id={port.id}
        className="!w-3 !h-3 !border-2 !border-white"
        style={{ backgroundColor: c }}
      />
    </div>
  );
}

// ─── Divider ─────────────────────────────────────────────────────
export function PortDivider() {
  return <div className="mx-3 border-t border-[#f0f0f0]" />;
}

// ─── NodeShell ───────────────────────────────────────────────────
export interface NodeShellProps {
  /** Icon element shown before typeName. */
  icon: React.ReactNode;
  /** Node type name shown UPPERCASE in the caption. */
  typeName: string;
  /** Instance label shown lowercase in the caption. */
  label: string;
  /** Current node status drives the LED. */
  status: NodeStatus;
  /** Whether the node is selected in the graph. */
  selected: boolean;
  /** Minimum width of the node card. */
  minWidth?: number;
  /** Optional extra element in the header (e.g. expand toggle). */
  headerExtra?: React.ReactNode;
  children: React.ReactNode;
}

export function NodeShell({
  icon,
  typeName,
  label,
  status,
  selected,
  minWidth = 200,
  headerExtra,
  children,
}: NodeShellProps) {
  return (
    <div
      className={`rounded-xl border bg-white shadow-sm ${
        selected ? 'border-[#007aff] shadow-md' : 'border-[#d2d2d7]'
      }`}
      style={{ minWidth }}
    >
      {/* Caption */}
      <div
        className="flex items-center gap-1.5 px-3 rounded-t-[11px]"
        style={{ height: HEADER_H, backgroundColor: HEADER_BG }}
      >
        {/* Left: type icon + type name (UPPERCASE) */}
        <span className="text-white/60 leading-none flex-shrink-0">{icon}</span>
        <span className="text-[10px] font-semibold text-white leading-none">{typeName.toUpperCase()}</span>

        <span className="flex-1" />

        {/* Right: instance label (lowercase) + status LED */}
        <span className="text-[10px] text-white/50 font-medium truncate max-w-[80px]">{label.toLowerCase()}</span>
        {headerExtra}
        <StatusLed status={status} />
      </div>

      {children}
    </div>
  );
}
