/**
 * Tauri platform helpers.
 * All imports are dynamic because these modules only exist in the Tauri runtime,
 * not in plain browser environments. This is a platform-specific exception.
 */

let _isTauri: boolean | null = null;

export async function checkIsTauri(): Promise<boolean> {
  if (_isTauri !== null) return _isTauri;
  try {
    // dynamic: module only exists in Tauri runtime
    const { isTauri } = await import('@tauri-apps/api/core');
    _isTauri = isTauri();
  } catch {
    _isTauri = false;
  }
  return _isTauri;
}

export async function tauriConvertFileSrc(filePath: string): Promise<string> {
  // dynamic: module only exists in Tauri runtime
  const { convertFileSrc } = await import('@tauri-apps/api/core');
  return convertFileSrc(filePath);
}

export async function tauriReadVideoThumbnail(filePath: string): Promise<string> {
  const { invoke } = await import('@tauri-apps/api/core');
  const bytes = Uint8Array.from(await invoke<number[]>('native_video_thumbnail', { path: filePath }));
  let binary = '';
  for (let offset = 0; offset < bytes.length; offset += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(offset, offset + 0x8000));
  }
  return `data:image/jpeg;base64,${btoa(binary)}`;
}

export async function tauriOpenVideoFile(): Promise<string | null> {
  // dynamic: plugin only available in Tauri runtime
  const { open } = await import('@tauri-apps/plugin-dialog');
  const selected = await open({
    multiple: false,
    filters: [{ name: 'Video', extensions: ['mp4', 'webm', 'mov', 'avi', 'mkv', 'ogg'] }],
  });
  if (!selected || typeof selected !== 'string') return null;
  return selected;
}
