const DATA_URL_CHUNK_SIZE = 0x8000;

export function bytesToBase64DataUrl(bytes: Uint8Array, mimeType: string): string {
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += DATA_URL_CHUNK_SIZE) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + DATA_URL_CHUNK_SIZE));
  }
  return `data:${mimeType};base64,${btoa(binary)}`;
}

export async function blobToDataUrl(blob: Blob): Promise<string> {
  if (typeof FileReader === 'function') {
    return await new Promise<string>((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        if (typeof reader.result === 'string') {
          resolve(reader.result);
        } else {
          reject(new Error('Preview blob did not produce a data URL'));
        }
      };
      reader.onerror = () => reject(reader.error ?? new Error('Preview blob data URL read failed'));
      reader.readAsDataURL(blob);
    });
  }

  const bytes = new Uint8Array(await blob.arrayBuffer());
  return bytesToBase64DataUrl(bytes, blob.type || 'application/octet-stream');
}
