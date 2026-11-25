import { execFileSync } from 'node:child_process';
import { copyFile, readdir, stat } from 'node:fs/promises';
import { join } from 'node:path';

export async function copyWindowsCrt(destination) {
  const vswhere = join(process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)',
    'Microsoft Visual Studio', 'Installer', 'vswhere.exe');
  const vs = execFileSync(vswhere, ['-latest', '-products', '*', '-requires',
    'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath'],
    { encoding: 'utf8' }).trim();
  if (!vs) throw new Error('Visual C++ x64 build tools/redist are required for portable Windows packaging');
  const redistRoot = join(vs, 'VC', 'Redist', 'MSVC');
  const versions = (await readdir(redistRoot)).filter((name) => /^\d+\.\d+\.\d+$/.test(name))
    .sort((a, b) => b.localeCompare(a, undefined, { numeric: true }));
  if (!versions.length) throw new Error(`No MSVC redistributable found in ${redistRoot}`);
  const redistArch = join(redistRoot, versions[0], 'x64');
  const crt = (await readdir(redistArch)).filter((name) => /^Microsoft\.VC\d+\.CRT$/.test(name))
    .sort((a, b) => b.localeCompare(a, undefined, { numeric: true }))[0];
  if (!crt) throw new Error(`No app-local VC++ CRT found in ${redistArch}`);
  const redist = join(redistArch, crt);
  for (const file of await readdir(redist)) {
    if (!file.toLowerCase().endsWith('.dll')) continue;
    const source = join(redist, file);
    const info = await stat(source);
    if (!info.isFile() || !info.size) throw new Error(`Invalid VC++ runtime file: ${source}`);
    await copyFile(source, join(destination, file));
  }
  for (const file of ['msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll']) {
    const path = join(destination, file);
    const info = await stat(path);
    if (!info.isFile() || !info.size) throw new Error(`Invalid VC++ runtime file: ${path}`);
  }
}
