import { WasmSdkClient } from './WasmSdkClient';
import type { WasmModuleLoader } from './WasmSdkClient';

let client: WasmSdkClient | null = null;
let initialization: Promise<WasmSdkClient> | null = null;

export function initializeSdk(loader?: WasmModuleLoader): Promise<WasmSdkClient> {
  if (client) return Promise.resolve(client);
  if (initialization) return initialization;
  initialization = WasmSdkClient.load(loader).then((loaded) => {
    client = loaded;
    return loaded;
  });
  return initialization;
}

export function requireSdk(): WasmSdkClient {
  if (!client) {
    throw new Error('Rust SDK must be initialized before synchronous engine operations');
  }
  return client;
}
