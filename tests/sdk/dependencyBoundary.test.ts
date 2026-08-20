import { describe, expect, it } from 'vitest';
import * as sdk from '../../src/sdk';

const FORBIDDEN_PUBLIC_EXPORTS = [
  'Runtime',
  'Engine',
  'ExecutionPlan',
  'ExecutionEngine',
  'GpuExecutor',
  'BrowserHost',
  'NativeHost',
  'WasmSdkClient',
  'WasmRuntimeContract',
  'PipelineRuntime',
] as const;

describe('SDK dependency boundaries', () => {
  it('exports only public object proxies and value/error contracts', () => {
    expect(Object.keys(sdk).sort()).toEqual([
      'Catalog',
      'Graph',
      'Node',
      'OpenQuartzClient',
      'Output',
      'Player',
      'Port',
      'Project',
      'Resource',
      'SdkContractError',
      'Subscription',
    ]);
    for (const forbidden of FORBIDDEN_PUBLIC_EXPORTS) {
      expect(forbidden in sdk).toBe(false);
    }
  });
});
