export type RuntimeLogLevel = 'debug' | 'info' | 'warn' | 'error';

export interface RuntimeLogRecord {
  source: 'browser-host' | 'browser-worker' | 'gpu' | 'wasm' | 'native';
  level: RuntimeLogLevel;
  event: string;
  fields?: Record<string, unknown>;
  at: number;
}

const MAX_TEXT = 320;

export function compactRuntimeText(value: unknown, max = MAX_TEXT): string {
  const text = value instanceof Error ? value.message : String(value);
  const compact = text.replace(/\s+/g, ' ').trim();
  return compact.length > max ? `${compact.slice(0, max)}…` : compact;
}

export function runtimeLog(
  source: RuntimeLogRecord['source'],
  level: RuntimeLogLevel,
  event: string,
  fields?: Record<string, unknown>,
): RuntimeLogRecord {
  const record: RuntimeLogRecord = { source, level, event, fields, at: Date.now() };
  const payload = JSON.stringify(record);
  const method = level === 'debug' ? console.debug : level === 'info' ? console.info : level === 'warn' ? console.warn : console.error;
  method(`[oq] ${payload}`);
  return record;
}
