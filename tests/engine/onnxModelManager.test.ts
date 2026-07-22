import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import type { CatalogEntry } from '../../src/catalog/onnxCatalog';
import type { ModelState } from '../../src/engine/onnxModelManager';

// Ensure non-Tauri path: isTauri is `'__TAURI_INTERNALS__' in window`
// evaluated at module load. jsdom doesn't set it, but be explicit.
delete (window as Record<string, unknown>).__TAURI_INTERNALS__;

import { OnnxModelManager } from '../../src/engine/onnxModelManager';

function makeCatalogEntry(overrides: Partial<CatalogEntry> = {}): CatalogEntry {
  return {
    id: 'test-model',
    label: 'Test Model',
    task: 'detection',
    category: 'Detection',
    downloadUrl: 'https://example.com/model.onnx',
    fileSize: 1024,
    sha256: 'abc123',
    expectedIO: { inputs: [], outputs: [] },
    ...overrides,
  };
}

describe('OnnxModelManager', () => {
  let mgr: OnnxModelManager;

  beforeEach(() => {
    mgr = new OnnxModelManager();
  });

  afterEach(() => {
    mgr.dispose();
    vi.restoreAllMocks();
  });

  // -----------------------------------------------------------------------
  // getState
  // -----------------------------------------------------------------------

  it('getState returns default state for unknown model', () => {
    const state = mgr.getState('nonexistent');
    expect(state).toEqual({ status: 'not-downloaded', progress: 0 });
  });

  // -----------------------------------------------------------------------
  // subscribe / notify
  // -----------------------------------------------------------------------

  describe('subscribe', () => {
    it('listener is called when state updates', async () => {
      const listener = vi.fn();
      mgr.subscribe(listener);

      // Trigger a state update via downloadModel with a cached buffer
      const entry = makeCatalogEntry();
      const buf = new ArrayBuffer(8);
      // Manually seed the cache by downloading once with mocked fetch
      // Instead, use downloadModel with a pre-cached buffer approach:
      // First call triggers a fetch, but let's use the simpler path.
      // Seed buffer via a successful download, then check listener was called.

      // Mock fetch to return a simple response without body stream
      const mockResponse = {
        ok: true,
        status: 200,
        body: null,
        arrayBuffer: vi.fn().mockResolvedValue(buf),
      } as unknown as Response;
      vi.spyOn(globalThis, 'fetch').mockResolvedValue(mockResponse);

      await mgr.downloadModel(entry);

      // updateState is called multiple times: 'downloading' then 'downloaded'
      expect(listener).toHaveBeenCalledTimes(3); // downloading, progress=1 (no-body fallback), downloaded
    });

    it('returns an unsubscribe function that removes the listener', async () => {
      const listener = vi.fn();
      const unsub = mgr.subscribe(listener);
      unsub();

      // Trigger state update — listener should NOT be called
      const buf = new ArrayBuffer(4);
      const mockResponse = {
        ok: true, status: 200, body: null,
        arrayBuffer: vi.fn().mockResolvedValue(buf),
      } as unknown as Response;
      vi.spyOn(globalThis, 'fetch').mockResolvedValue(mockResponse);

      await mgr.downloadModel(makeCatalogEntry());
      expect(listener).not.toHaveBeenCalled();
    });
  });

  // -----------------------------------------------------------------------
  // downloadModel
  // -----------------------------------------------------------------------

  describe('downloadModel', () => {
    it('returns cached buffer immediately without fetching', async () => {
      const entry = makeCatalogEntry();
      const buf = new ArrayBuffer(16);

      // First download to populate cache
      const mockResponse = {
        ok: true, status: 200, body: null,
        arrayBuffer: vi.fn().mockResolvedValue(buf),
      } as unknown as Response;
      const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(mockResponse);

      await mgr.downloadModel(entry);
      expect(fetchSpy).toHaveBeenCalledTimes(1);

      // Second call — should use cache, no additional fetch
      const result = await mgr.downloadModel(entry);
      expect(fetchSpy).toHaveBeenCalledTimes(1);
      expect(result).toBe(buf);

      const state = mgr.getState(entry.id);
      expect(state.status).toBe('downloaded');
      expect(state.progress).toBe(1);
      expect(state.modelBuffer).toBe(buf);
    });

    it('downloads via fetch, caches buffer, and updates state', async () => {
      const entry = makeCatalogEntry({ fileSize: 8 });
      const payload = new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8]);

      // Build a ReadableStream that emits two chunks
      const stream = new ReadableStream<Uint8Array>({
        start(controller) {
          controller.enqueue(payload.slice(0, 4));
          controller.enqueue(payload.slice(4));
          controller.close();
        },
      });

      const mockResponse = {
        ok: true,
        status: 200,
        body: stream,
        arrayBuffer: vi.fn(),
      } as unknown as Response;
      vi.spyOn(globalThis, 'fetch').mockResolvedValue(mockResponse);

      const listener = vi.fn();
      mgr.subscribe(listener);

      const result = await mgr.downloadModel(entry);

      // Verify the merged buffer matches the payload
      expect(new Uint8Array(result)).toEqual(payload);

      // Verify state is 'downloaded' with progress 1
      const state = mgr.getState(entry.id);
      expect(state.status).toBe('downloaded');
      expect(state.progress).toBe(1);
      expect(state.modelBuffer).toBe(result);

      // fetch was called with the download URL
      expect(globalThis.fetch).toHaveBeenCalledWith(entry.downloadUrl);

      // Listener was called for: downloading(0), progress(0.5), progress(1), downloaded
      expect(listener.mock.calls.length).toBeGreaterThanOrEqual(3);
    });

    it('sets error state and rethrows on fetch failure', async () => {
      const entry = makeCatalogEntry();
      const fetchError = new Error('Network failure');
      vi.spyOn(globalThis, 'fetch').mockRejectedValue(fetchError);

      await expect(mgr.downloadModel(entry)).rejects.toThrow('Network failure');

      const state = mgr.getState(entry.id);
      expect(state.status).toBe('error');
      expect(state.progress).toBe(0);
      expect(state.error).toBe('Network failure');
    });

    it('sets error state on non-ok HTTP response', async () => {
      const entry = makeCatalogEntry();
      const mockResponse = {
        ok: false,
        status: 404,
        statusText: 'Not Found',
      } as unknown as Response;
      vi.spyOn(globalThis, 'fetch').mockResolvedValue(mockResponse);

      await expect(mgr.downloadModel(entry)).rejects.toThrow('HTTP 404: Not Found');

      const state = mgr.getState(entry.id);
      expect(state.status).toBe('error');
      expect(state.error).toBe('HTTP 404: Not Found');
    });

    it('converts non-Error throw to string in error state', async () => {
      const entry = makeCatalogEntry();
      vi.spyOn(globalThis, 'fetch').mockRejectedValue('string-error');

      await expect(mgr.downloadModel(entry)).rejects.toBe('string-error');

      const state = mgr.getState(entry.id);
      expect(state.status).toBe('error');
      expect(state.error).toBe('string-error');
    });
  });

  // -----------------------------------------------------------------------
  // loadCachedModel
  // -----------------------------------------------------------------------

  describe('loadCachedModel', () => {
    it('returns cached buffer for a previously downloaded model', async () => {
      const entry = makeCatalogEntry();
      const buf = new ArrayBuffer(8);
      const mockResponse = {
        ok: true, status: 200, body: null,
        arrayBuffer: vi.fn().mockResolvedValue(buf),
      } as unknown as Response;
      vi.spyOn(globalThis, 'fetch').mockResolvedValue(mockResponse);

      await mgr.downloadModel(entry);

      const cached = await mgr.loadCachedModel(entry.id);
      expect(cached).toBe(buf);
    });

    it('returns null for unknown model in non-Tauri env', async () => {
      const result = await mgr.loadCachedModel('never-downloaded');
      expect(result).toBeNull();
    });
  });

  // -----------------------------------------------------------------------
  // isDownloaded
  // -----------------------------------------------------------------------

  describe('isDownloaded', () => {
    it('returns true when model is in buffer cache', async () => {
      const entry = makeCatalogEntry();
      const buf = new ArrayBuffer(4);
      const mockResponse = {
        ok: true, status: 200, body: null,
        arrayBuffer: vi.fn().mockResolvedValue(buf),
      } as unknown as Response;
      vi.spyOn(globalThis, 'fetch').mockResolvedValue(mockResponse);

      await mgr.downloadModel(entry);

      expect(await mgr.isDownloaded(entry.id)).toBe(true);
    });

    it('returns false for unknown model in non-Tauri env', async () => {
      expect(await mgr.isDownloaded('unknown')).toBe(false);
    });
  });

  // -----------------------------------------------------------------------
  // dispose
  // -----------------------------------------------------------------------

  it('dispose clears all internal state', async () => {
    const entry = makeCatalogEntry();
    const buf = new ArrayBuffer(4);
    const mockResponse = {
      ok: true, status: 200, body: null,
      arrayBuffer: vi.fn().mockResolvedValue(buf),
    } as unknown as Response;
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(mockResponse);

    const listener = vi.fn();
    mgr.subscribe(listener);
    await mgr.downloadModel(entry);
    listener.mockClear();

    mgr.dispose();

    // State reverts to default
    expect(mgr.getState(entry.id)).toEqual({ status: 'not-downloaded', progress: 0 });
    // Cache cleared
    expect(await mgr.isDownloaded(entry.id)).toBe(false);
    // Listener no longer called (was removed by dispose)
  });
});
