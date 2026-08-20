import { afterEach, describe, expect, it, vi } from 'vitest';
import { blobToDataUrl, bytesToBase64DataUrl } from '../../src/sdk/internal/browserPreviewEncoding';

describe('browser preview encoding', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('matches the existing byte-string data URL contract', () => {
    const bytes = new Uint8Array([0, 1, 2, 253, 254, 255]);

    expect(bytesToBase64DataUrl(bytes, 'image/png')).toBe('data:image/png;base64,AAEC/f7/');
  });

  it('falls back to byte-string encoding when FileReader is unavailable', async () => {
    vi.stubGlobal('FileReader', undefined);
    const blob = new Blob([new Uint8Array([10, 20, 30, 40])], { type: 'image/png' });

    await expect(blobToDataUrl(blob)).resolves.toBe('data:image/png;base64,ChQeKA==');
  });

  it('uses worker-native FileReader data URLs when available', async () => {
    const readAsDataURL = vi.fn(function (
      this: { result: string | null; onload: (() => void) | null },
    ) {
      this.result = 'data:image/png;base64,native';
      queueMicrotask(() => this.onload?.());
    });
    vi.stubGlobal('FileReader', class {
      result: string | ArrayBuffer | null = null;
      error: DOMException | null = null;
      onload: (() => void) | null = null;
      onerror: (() => void) | null = null;
      readAsDataURL = readAsDataURL;
    });

    await expect(blobToDataUrl(new Blob([new Uint8Array([1])], { type: 'image/png' })))
      .resolves.toBe('data:image/png;base64,native');
    expect(readAsDataURL).toHaveBeenCalledOnce();
  });
});
