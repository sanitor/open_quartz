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
