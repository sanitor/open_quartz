#!/usr/bin/env node
// Native desktop entry point. Web/WASM preparation is deliberately separate.
import { execFileSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmod, copyFile, cp, mkdir, readFile, readdir, rename, rm, stat, writeFile } from 'node:fs/promises';
import { delimiter, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const [mode = 'dev', ...appArgs] = process.argv.slice(2);
if (!['dev', 'build', 'prepare', 'test'].includes(mode) || (mode !== 'dev' && appArgs.length)) {
  throw new Error('Usage: node scripts/desktop.mjs dev [application arguments] | build | prepare | test');
}
const platform = process.platform;
const arch = process.arch;
const triples = {
  'darwin-arm64': 'aarch64-apple-darwin', 'darwin-x64': 'x86_64-apple-darwin',
  'win32-x64': 'x86_64-pc-windows-msvc',
  'linux-x64': 'x86_64-unknown-linux-gnu', 'linux-arm64': 'aarch64-unknown-linux-gnu',
};
const triple = triples[`${platform}-${arch}`];
if (!triple) throw new Error(`Unsupported native desktop platform: ${platform}/${arch}`);
if (process.env.CARGO_BUILD_TARGET && process.env.CARGO_BUILD_TARGET !== triple) {
  throw new Error('desktop.mjs packages the host architecture only; run on a matching native host');
}
const cache = join(root, 'node_modules', '.cache', 'open-quartz');
const target = join(root, 'apps', 'open_quartz_slint', 'target');
const manifest = join(root, 'apps', 'open_quartz_slint', 'Cargo.toml');
const helperRuntime = join(root, 'src-tauri', 'runtime');
const runtime = join(cache, `desktop-runtime-${platform}-${arch}`);
const exe = `open_quartz_slint${platform === 'win32' ? '.exe' : ''}`;
const ortName = platform === 'win32' ? 'onnxruntime.dll'
  : platform === 'darwin' ? 'libonnxruntime.1.21.0.dylib' : 'libonnxruntime.so.1.21.0';

async function run(command, args, options = {}) {
  await new Promise((accept, reject) => {
    const child = spawn(command, args, { cwd: root, stdio: 'inherit', ...options });
    child.once('error', reject);
    child.once('exit', (code, signal) => code === 0 ? accept()
      : reject(new Error(`${command} failed (${signal || code})`)));
  });
}
async function requireFile(path) {
  const info = await stat(path).catch(() => null);
  if (!info?.isFile() || info.size === 0) throw new Error(`Required desktop resource missing or empty: ${path}`);
}
async function download(url, name, sha256) {
  const path = join(cache, name);
  let bytes = await readFile(path).catch(() => null);
  if (!bytes) {
    const response = await fetch(url, { signal: AbortSignal.timeout(600_000) });
    if (!response.ok) throw new Error(`Runtime download failed (${response.status}): ${url}`);
    bytes = Buffer.from(await response.arrayBuffer());
  }
  if (sha256 && createHash('sha256').update(bytes).digest('hex') !== sha256) {
    throw new Error(`Runtime checksum mismatch: remove ${path} and retry`);
  }
  await writeFile(path, bytes);
  return path;
}
async function unpack(archive, directory) {
  await rm(directory, { recursive: true, force: true });
  await mkdir(directory, { recursive: true });
  await run(platform === 'win32' ? 'tar.exe' : 'tar', ['-xf', archive, '-C', directory]);
}
async function copyRequired(source, destination) {
  await requireFile(source);
  await copyFile(source, destination);
}
async function prepareRuntime() {
  await mkdir(cache, { recursive: true });
  await rm(runtime, { recursive: true, force: true });
  await mkdir(runtime, { recursive: true });
  await run(process.execPath, ['scripts/copy-ffmpeg.mjs']);
  const ffmpeg = platform === 'win32' ? 'ffmpeg.exe' : 'ffmpeg';
  await copyRequired(join(helperRuntime, ffmpeg), join(runtime, ffmpeg));
  if (platform !== 'win32') await chmod(join(runtime, ffmpeg), 0o755);
  await copyRequired(join(helperRuntime, 'FFMPEG-NOTICE.txt'), join(runtime, 'FFMPEG-NOTICE.txt'));
  if (platform === 'win32') {
    for (const name of await readdir(join(helperRuntime, 'ffmpeg-shared', 'bin'))) {
      if (name.toLowerCase().endsWith('.dll')) {
        await copyRequired(join(helperRuntime, 'ffmpeg-shared', 'bin', name), join(runtime, name));
      }
    }
    for (const name of ['avcodec-63.dll', 'avformat-63.dll', 'avutil-61.dll', 'swresample-7.dll']) {
      await requireFile(join(runtime, name));
    }
    await cp(join(helperRuntime, 'ffmpeg-shared', 'doc'), join(runtime, 'ffmpeg-doc'), { recursive: true });
    await cp(join(helperRuntime, 'ffmpeg-shared', 'LICENSE.txt'), join(runtime, 'FFMPEG-LICENSE.txt'));
    const url = 'https://github.com/microsoft/onnxruntime/releases/download/v1.21.0/Microsoft.ML.OnnxRuntime.DirectML.1.21.0.nupkg';
    const archive = await download(url, 'ort-directml-1.21.0.nupkg', 'f1f7dd0b6a568ff6e79be3822d591faffb3ae0f7f0aaf58ee3d7bd1d9f0e6edc');
    const extracted = join(cache, 'ort-directml-1.21.0');
    await unpack(archive, extracted);
    await copyRequired(join(extracted, 'runtimes', 'win-x64', 'native', ortName), join(runtime, ortName));
    await copyRequired(join(extracted, 'LICENSE'), join(runtime, 'ONNXRUNTIME-LICENSE.txt'));
    await copyRequired(join(extracted, 'ThirdPartyNotices.txt'), join(runtime, 'ONNXRUNTIME-NOTICES.txt'));
    // This version is the native dependency declared by Microsoft's ORT 1.21.0 package.
    const dmlUrl = 'https://api.nuget.org/v3-flatcontainer/microsoft.ai.directml/1.15.4/microsoft.ai.directml.1.15.4.nupkg';
    const dml = join(cache, 'directml-1.15.4');
    await unpack(await download(dmlUrl, 'directml-1.15.4.nupkg'), dml);
    await copyRequired(join(dml, 'bin', 'x64-win', 'DirectML.dll'), join(runtime, 'DirectML.dll'));
    // Keep the full dependency distribution, including its redistributable license.
    await cp(dml, join(runtime, 'directml-distribution'), { recursive: true });
    await writeFile(join(runtime, 'ONNXRUNTIME-SOURCE.txt'), `${url}\nDirectML dependency: ${dmlUrl}\n`);
    // FFMPEG_DIR is also required by ffmpeg-sys-next when Windows D3D12 is enabled.
    await requireFile(join(helperRuntime, 'ffmpeg-shared', 'include', 'libavcodec', 'avcodec.h'));
    await requireFile(join(helperRuntime, 'ffmpeg-shared', 'lib', 'avcodec.lib'));
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
      if (file.toLowerCase().endsWith('.dll')) await copyRequired(join(redist, file), join(runtime, file));
    }
    for (const file of ['msvcp140.dll', 'vcruntime140.dll', 'vcruntime140_1.dll']) {
      await requireFile(join(runtime, file));
    }
  } else {
    const assets = {
      'darwin-arm64': ['osx-arm64', '5c3f2064ee97eb7774e87f396735c8eada7287734f1bb7847467ad30d4036115'],
      'darwin-x64': ['osx-x86_64', '8305afd2d75ee5702844a23b099d41885af30ad3d1b4cf3d8d795e3d8c1f9396'],
      'linux-x64': ['linux-x64', '7485c7e7aac6501b27e353dcbe068e45c61ab51fbaf598d13970dfae669d20bf'],
      'linux-arm64': ['linux-aarch64', '4508084bde1232ee1ab4b6fad2155be0ea2ccab1c1aae9910ddb3fb68a60805e'],
    };
    const [asset, digest] = assets[`${platform}-${arch}`];
    const name = `onnxruntime-${asset}-1.21.0`;
    const url = `https://github.com/microsoft/onnxruntime/releases/download/v1.21.0/${name}.tgz`;
    const extracted = join(cache, name);
    await unpack(await download(url, `${name}.tgz`, digest), extracted);
    const source = join(extracted, name);
    for (const file of await readdir(join(source, 'lib'))) {
      if (file.endsWith('.dylib') || /\.so(?:\.[\d.]+)?$/.test(file)) {
        await copyRequired(join(source, 'lib', file), join(runtime, file));
      }
    }
    await copyRequired(join(source, 'LICENSE'), join(runtime, 'ONNXRUNTIME-LICENSE.txt'));
    await copyRequired(join(source, 'ThirdPartyNotices.txt'), join(runtime, 'ONNXRUNTIME-NOTICES.txt'));
    await writeFile(join(runtime, 'ONNXRUNTIME-SOURCE.txt'), `${url}\nSHA-256: ${digest}\n`);
  }
  await requireFile(join(runtime, ortName));
}
function nativeEnv() {
  const env = {
    ...process.env,
    CARGO_TARGET_DIR: target,
    ORT_DYLIB_PATH: join(runtime, ortName),
    OPEN_QUARTZ_FFMPEG_PATH: join(runtime, platform === 'win32' ? 'ffmpeg.exe' : 'ffmpeg'),
    PATH: `${runtime}${delimiter}${process.env.PATH || ''}`,
  };
  // Do not let Cargo put a host-target build in a nested cross-target directory.
  delete env.CARGO_BUILD_TARGET;
  if (platform === 'win32') env.FFMPEG_DIR = join(helperRuntime, 'ffmpeg-shared');
  return env;
}
async function installRuntime(directory) {
  await rm(join(directory, 'runtime'), { recursive: true, force: true });
  await cp(runtime, join(directory, 'runtime'), { recursive: true });
  if (platform === 'win32') {
    // ffmpeg-next imports DLLs before main; PATH setup inside Rust is too late.
    for (const file of await readdir(runtime)) {
      if (file.toLowerCase().endsWith('.dll')) await copyRequired(join(runtime, file), join(directory, file));
    }
  }
}
async function verifyStartup(binary) {
  const env = { ...process.env };
  delete env.ORT_DYLIB_PATH;
  delete env.OPEN_QUARTZ_FFMPEG_PATH;
  await run(binary, ['--help'], { env });
}
async function packageDesktop(binary) {
  const pkg = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
  const config = JSON.parse(await readFile(join(root, 'src-tauri', 'tauri.conf.json'), 'utf8'));
  const output = join(root, 'dist-desktop');
  const label = `OpenQuartz-${pkg.version}-${platform === 'win32' ? 'windows' : platform === 'darwin' ? 'macos' : 'linux'}-${arch}`;
  const stagingRoot = join(output, `${label}.staging`);
  const staging = join(stagingRoot, label);
  await mkdir(output, { recursive: true });
  await rm(stagingRoot, { recursive: true, force: true });
  await mkdir(staging, { recursive: true });
  const extension = platform === 'linux' ? '.tar.gz' : '.zip';
  const archive = join(output, `${label}${extension}`);
  const partialArchive = join(output, `${label}.partial${extension}`);
  try {
    if (platform === 'darwin') {
      const app = join(staging, 'Open Quartz.app');
      const contents = join(app, 'Contents');
      const resources = join(contents, 'Resources');
      await mkdir(join(contents, 'MacOS'), { recursive: true });
      await mkdir(resources, { recursive: true });
      await copyRequired(binary, join(contents, 'MacOS', exe));
      await chmod(join(contents, 'MacOS', exe), 0o755);
      await cp(runtime, join(resources, 'runtime'), { recursive: true });
      await copyRequired(join(root, 'src-tauri', 'icons', 'icon.icns'), join(resources, 'icon.icns'));
      await copyRequired(join(root, 'LICENSE'), join(resources, 'LICENSE.txt'));
      await writeFile(join(contents, 'Info.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>Open Quartz</string>
<key>CFBundleDisplayName</key><string>Open Quartz</string>
<key>CFBundleIdentifier</key><string>com.openquartz.app</string>
<key>CFBundleExecutable</key><string>${exe}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>icon.icns</string>
<key>CFBundleShortVersionString</key><string>${config.version}</string>
<key>CFBundleVersion</key><string>${config.version}</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
<key>LSBackgroundOnly</key><false/>
<key>LSUIElement</key><false/>
<key>NSHighResolutionCapable</key><true/>
<key>NSCameraUsageDescription</key><string>Open Quartz uses the selected camera as a video input in your project.</string>
</dict></plist>\n`);
      // Ad-hoc signing makes the assembled bundle internally consistent, not notarized.
      for (const file of await readdir(join(resources, 'runtime'))) {
        if (file === 'ffmpeg' || file.endsWith('.dylib')) {
          await run('codesign', ['--force', '--sign', '-', join(resources, 'runtime', file)]);
        }
      }
      await run('codesign', ['--force', '--deep', '--sign', '-', app]);
      await verifyStartup(join(contents, 'MacOS', exe));
      await run('ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', app, partialArchive]);
    } else {
      await copyRequired(binary, join(staging, exe));
      await installRuntime(staging);
      await copyRequired(join(root, 'LICENSE'), join(staging, 'LICENSE.txt'));
      await copyRequired(join(root, 'src-tauri', 'icons', platform === 'win32' ? 'icon.ico' : 'icon.png'),
        join(staging, platform === 'win32' ? 'icon.ico' : 'icon.png'));
      await verifyStartup(join(staging, exe));
      if (platform === 'win32') {
        await requireFile(join(staging, 'runtime', 'open-quartz-screensaver-stub.exe'));
        const quote = (value) => `'${value.replaceAll("'", "''")}'`;
        await run('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command',
          `$shell = New-Object -ComObject WScript.Shell; ` +
          `$link = $shell.CreateShortcut(${quote(join(staging, 'Open Quartz.lnk'))}); ` +
          `$link.TargetPath = ${quote(join(staging, exe))}; ` +
          `$link.IconLocation = ${quote(join(staging, 'icon.ico'))}; ` +
          `$link.Description = 'Open Quartz native desktop'; $link.Save()`]);
        await run('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command',
          `Compress-Archive -LiteralPath ${quote(staging)} -DestinationPath ${quote(partialArchive)} -Force`]);
      } else {
        await chmod(join(staging, exe), 0o755);
        await run('tar', ['-czf', partialArchive, '-C', stagingRoot, label]);
      }
    }
    await rm(archive, { force: true });
    await rename(partialArchive, archive);
    const directory = join(output, label);
    await rm(directory, { recursive: true, force: true });
    await rename(staging, directory);
    await rm(stagingRoot, { recursive: true, force: true });
    console.log(`[desktop] Packaged ${directory}\n[desktop] Archive ${archive}`);
  } catch (error) {
    await rm(stagingRoot, { recursive: true, force: true });
    await rm(partialArchive, { force: true });
    throw error;
  }
}

await prepareRuntime();
const env = nativeEnv();
if (mode === 'prepare') {
  console.log(`[desktop] Runtime: ${runtime}`);
} else if (mode === 'test') {
  await run('cargo', ['test', '--locked', '--manifest-path', manifest], { env });
} else {
  if (platform === 'win32') {
    await run(process.execPath, ['scripts/build-screensaver-stub.mjs'], { env });
    await copyRequired(join(helperRuntime, 'open-quartz-screensaver-stub.exe'), join(runtime, 'open-quartz-screensaver-stub.exe'));
  }
  const release = mode === 'build';
  await run('cargo', ['build', '--locked', '--manifest-path', manifest, ...(release ? ['--release'] : [])], { env });
  const directory = join(target, release ? 'release' : 'debug');
  const binary = join(directory, exe);
  await installRuntime(directory);
  if (release) await packageDesktop(binary);
  else await run(binary, appArgs, { env });
}
