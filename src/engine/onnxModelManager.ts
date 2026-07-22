import type { CatalogEntry } from '../catalog/onnxCatalog';

// ---------------------------------------------------------------------------
// Model lifecycle status
// ---------------------------------------------------------------------------

export type OnnxModelStatus =
  | 'not-downloaded'
  | 'downloading'
  | 'downloaded'
  | 'introspecting'
  | 'ready'
  | 'error';

// ---------------------------------------------------------------------------
// Per-model runtime state
// ---------------------------------------------------------------------------

export interface ModelState {
  status: OnnxModelStatus;
  progress: number;        // 0-1 download progress
  error?: string;
  localPath?: string;      // Tauri: absolute path on disk
  modelBuffer?: ArrayBuffer; // in-memory after download/load
}

// ---------------------------------------------------------------------------
// Default state factory
// ---------------------------------------------------------------------------

function defaultState(): ModelState {
  return { status: 'not-downloaded', progress: 0 };
}

// ---------------------------------------------------------------------------
// Tauri detection
// ---------------------------------------------------------------------------

const isTauri = '__TAURI_INTERNALS__' in window;

// ---------------------------------------------------------------------------
// OnnxModelManager
// ---------------------------------------------------------------------------

export class OnnxModelManager {
  private states = new Map<string, ModelState>();
  private listeners = new Set<() => void>();
  private bufferCache = new Map<string, ArrayBuffer>();
  private progressCleanup: (() => void) | null = null;

  // -----------------------------------------------------------------------
  // Subscription (React-compatible external store pattern)
  // -----------------------------------------------------------------------

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  // -----------------------------------------------------------------------
  // State access
  // -----------------------------------------------------------------------

  getState(modelId: string): ModelState {
    return this.states.get(modelId) ?? defaultState();
  }

  // -----------------------------------------------------------------------
  // Download
  // -----------------------------------------------------------------------

  async downloadModel(entry: CatalogEntry): Promise<ArrayBuffer> {
    const cached = this.bufferCache.get(entry.id);
    if (cached) {
      this.updateState(entry.id, { status: 'downloaded', progress: 1, modelBuffer: cached });
      return cached;
    }

    this.updateState(entry.id, { status: 'downloading', progress: 0 });

    try {
      let buffer: ArrayBuffer;

      if (isTauri) {
        buffer = await this.downloadViaTauri(entry);
      } else {
        buffer = await this.downloadViaFetch(entry);
      }

      this.bufferCache.set(entry.id, buffer);
      this.updateState(entry.id, { status: 'downloaded', progress: 1, modelBuffer: buffer });
      return buffer;
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.updateState(entry.id, { status: 'error', progress: 0, error: message });
      throw err;
    }
  }

  // -----------------------------------------------------------------------
  // Tauri download: Rust-side reqwest, no CORS restrictions
  // -----------------------------------------------------------------------

  private async downloadViaTauri(entry: CatalogEntry): Promise<ArrayBuffer> {
    // Dynamic import: @tauri-apps/api only exists in Tauri runtime, not in
    // plain browser environments. This is a platform-specific exception.
    const { invoke } = await import('@tauri-apps/api/core');
    const { listen } = await import('@tauri-apps/api/event');

    // Listen for progress events from Rust side
    const unlisten = await listen<{ model_id: string; received: number; total: number }>(
      'model-download-progress',
      (event) => {
        if (event.payload.model_id === entry.id) {
          const progress = event.payload.total > 0
            ? event.payload.received / event.payload.total
            : 0;
          this.updateState(entry.id, { progress });
        }
      },
    );
    this.progressCleanup = unlisten;

    try {
      // Check if already downloaded on disk
      const isDownloaded = await invoke<boolean>('is_model_downloaded', { modelId: entry.id });
      if (isDownloaded) {
        this.updateState(entry.id, { progress: 1 });
        const bytes = await invoke<number[]>('read_model', { modelId: entry.id });
        return new Uint8Array(bytes).buffer;
      }

      // Download via Rust (reqwest, no CORS)
      await invoke<string>('download_model', {
        modelId: entry.id,
        url: entry.downloadUrl,
        expectedSize: entry.fileSize,
      });

      // Read the downloaded file into memory
      const bytes = await invoke<number[]>('read_model', { modelId: entry.id });
      return new Uint8Array(bytes).buffer;
    } finally {
      unlisten();
      this.progressCleanup = null;
    }
  }

  // -----------------------------------------------------------------------
  // Fetch download: browser-only, subject to CORS
  // -----------------------------------------------------------------------

  private async downloadViaFetch(entry: CatalogEntry): Promise<ArrayBuffer> {
    const response = await fetch(entry.downloadUrl);
    if (!response.ok) {
      throw new Error(`HTTP ${response.status}: ${response.statusText}`);
    }
    return this.readResponseWithProgress(response, entry.id, entry.fileSize);
  }

  // -----------------------------------------------------------------------
  // Local model loading (for Custom ONNX nodes)
  // -----------------------------------------------------------------------

  async loadLocalModel(filePath: string): Promise<ArrayBuffer> {
    if (isTauri) {
      // Dynamic import: platform-specific, only exists in Tauri runtime.
      const { invoke } = await import('@tauri-apps/api/core');
      const bytes = await invoke<number[]>('read_model', { modelId: filePath });
      return new Uint8Array(bytes).buffer;
    }
    const response = await fetch(filePath);
    if (!response.ok) {
      throw new Error(`Failed to load local model: HTTP ${response.status}`);
    }
    return response.arrayBuffer();
  }

  // -----------------------------------------------------------------------
  // Cache queries
  // -----------------------------------------------------------------------

  async isDownloaded(modelId: string): Promise<boolean> {
    if (this.bufferCache.has(modelId)) return true;
    if (isTauri) {
      // Dynamic import: platform-specific, only exists in Tauri runtime.
      const { invoke } = await import('@tauri-apps/api/core');
      return invoke<boolean>('is_model_downloaded', { modelId });
    }
    return false;
  }

  async loadCachedModel(modelId: string): Promise<ArrayBuffer | null> {
    const cached = this.bufferCache.get(modelId);
    if (cached) return cached;

    if (isTauri) {
      // Dynamic import: platform-specific, only exists in Tauri runtime.
      const { invoke } = await import('@tauri-apps/api/core');
      try {
        const isOnDisk = await invoke<boolean>('is_model_downloaded', { modelId });
        if (!isOnDisk) return null;
        const bytes = await invoke<number[]>('read_model', { modelId });
        const buffer = new Uint8Array(bytes).buffer;
        this.bufferCache.set(modelId, buffer);
        return buffer;
      } catch {
        return null;
      }
    }

    return null;
  }

  /** Manually cache a model buffer (for custom models loaded from local files). */
  cacheBuffer(modelId: string, buffer: ArrayBuffer): void {
    this.bufferCache.set(modelId, buffer);
  }

  // -----------------------------------------------------------------------
  // Teardown
  // -----------------------------------------------------------------------

  dispose(): void {
    this.states.clear();
    this.listeners.clear();
    this.bufferCache.clear();
    this.progressCleanup?.();
    this.progressCleanup = null;
  }

  // -----------------------------------------------------------------------
  // Internal helpers
  // -----------------------------------------------------------------------

  private updateState(modelId: string, patch: Partial<ModelState>): void {
    const prev = this.states.get(modelId) ?? defaultState();
    this.states.set(modelId, { ...prev, ...patch });
    this.notify();
  }

  private notify(): void {
    for (const fn of this.listeners) {
      fn();
    }
  }

  private async readResponseWithProgress(
    response: Response,
    modelId: string,
    expectedSize: number,
  ): Promise<ArrayBuffer> {
    const body = response.body;

    if (!body) {
      const buffer = await response.arrayBuffer();
      this.updateState(modelId, { progress: 1 });
      return buffer;
    }

    const reader = body.getReader();
    const chunks: Uint8Array[] = [];
    let received = 0;

    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;

      chunks.push(value);
      received += value.byteLength;

      const progress = expectedSize > 0 ? Math.min(received / expectedSize, 1) : 0;
      this.updateState(modelId, { progress });
    }

    const merged = new Uint8Array(received);
    let offset = 0;
    for (const chunk of chunks) {
      merged.set(chunk, offset);
      offset += chunk.byteLength;
    }

    return merged.buffer;
  }
}
