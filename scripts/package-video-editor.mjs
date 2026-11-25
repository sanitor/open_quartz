import { execFile, spawn } from 'node:child_process';
import { chmod, copyFile, cp, mkdir, mkdtemp, readFile, readdir, rename, rm, stat, writeFile } from 'node:fs/promises';
import { basename, join } from 'node:path';
import { promisify } from 'node:util';
import { mediaSdk, version as mediaVersion } from './prepare-media-sdk.mjs';
import { copyWindowsCrt } from './copy-windows-crt.mjs';

const exec = promisify(execFile);
const libraries = ['libavfilter.11.dylib', 'libavcodec.62.dylib', 'libavformat.62.dylib', 'libavutil.60.dylib',
  'libswscale.9.dylib', 'libswresample.6.dylib'];
const notices = ['COPYING.LGPLv2.1', 'FFMPEG-MEDIA-NOTICE.txt', 'BUILD-CONFIG.txt', `ffmpeg-${mediaVersion}.tar.xz`,
  'ffmpeg-videotoolbox-hardware.patch', 'ffmpeg-videotoolbox-reorder.patch'];

async function run(command, args, env = process.env) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, { env, stdio: 'inherit' });
    child.once('error', reject);
    child.once('exit', (code, signal) => {
      if (!signal && code === 0) resolve();
      else reject(new Error(`${command} failed (${signal || code})`));
    });
  });
}

export async function packageVideoEditor(root, binary) {
  const packageInfo = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'));
  const version = packageInfo.version.match(/^\d+\.\d+\.\d+/)?.[0];
  if (!version) throw new Error('Invalid Studio version');
  const output = join(root, 'dist-desktop');
  await mkdir(output, { recursive: true });
  const stage = await mkdtemp(join(output, '.studio-'));
  const platform = process.platform === 'win32' ? 'windows' : 'macos';
  const archive = join(output, `OpenQuartz-Studio-${packageInfo.version}-${platform}-${process.arch}.zip`);
  try {
    if (process.platform === 'win32') {
      await packageWindows(root, binary, stage, archive);
      return;
    }
    const app = join(stage, 'OpenQuartz Studio.app');
    const contents = join(app, 'Contents');
    const runtime = join(contents, 'Resources', 'runtime');
    const executable = join(contents, 'MacOS', basename(binary));
    await mkdir(runtime, { recursive: true });
    await mkdir(join(contents, 'MacOS'), { recursive: true });
    await copyFile(binary, executable);
    await chmod(executable, 0o755);

    const source = join(root, 'apps', 'open_quartz_slint');
    for (const name of libraries) await copyFile(join(mediaSdk, 'lib', name), join(runtime, name));
    for (const name of notices) await copyFile(join(mediaSdk, name), join(runtime, name));
    await copyFile(join(mediaSdk, 'bin', 'ffmpeg'), join(runtime, 'ffmpeg'));
    await chmod(join(runtime, 'ffmpeg'), 0o755);
    await copyFile(join(source, 'icons', 'icon.icns'), join(contents, 'Resources', 'icon.icns'));
    await copyFile(join(root, 'LICENSE'), join(contents, 'Resources', 'LICENSE.txt'));
    await writeFile(join(contents, 'Info.plist'), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>OpenQuartz Studio</string>
<key>CFBundleDisplayName</key><string>OpenQuartz Studio</string>
<key>CFBundleIdentifier</key><string>com.openquartz.studio</string>
<key>CFBundleExecutable</key><string>${basename(binary)}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>icon.icns</string>
<key>CFBundleShortVersionString</key><string>${version}</string>
<key>CFBundleVersion</key><string>${version}</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>\n`);
    await run('plutil', ['-lint', join(contents, 'Info.plist')]);
    for (const name of [...libraries, 'ffmpeg']) {
      await run('codesign', ['--force', '--sign', '-', join(runtime, name)]);
    }
    await run('codesign', ['--force', '--deep', '--sign', '-', app]);
    const zip = join(stage, 'studio.zip');
    await run('ditto', ['-c', '-k', '--sequesterRsrc', '--keepParent', app, zip]);
    const extracted = join(stage, 'verification');
    await run('ditto', ['-x', '-k', zip, extracted]);
    const verifyApp = join(extracted, basename(app));
    await run('codesign', ['--verify', '--deep', '--strict', verifyApp]);
    const env = { ...process.env };
    for (const name of Object.keys(env)) {
      if (name.startsWith('DYLD_') || name === 'OPEN_QUARTZ_FFMPEG_PATH') delete env[name];
    }
    const found = join(verifyApp, 'Contents', 'MacOS', basename(binary));
    const { stdout } = await exec(found, ['--help'], { cwd: extracted, env });
    if (!stdout.includes('OpenQuartz Studio')) throw new Error('Packaged Studio help did not run');
    const helper = join(verifyApp, 'Contents', 'Resources', 'runtime', 'ffmpeg');
    const bundled = [found, helper, ...libraries.map(name => join(verifyApp, 'Contents', 'Resources', 'runtime', name))];
    for (const file of bundled) {
      const { stdout: linked } = await exec('otool', ['-L', file], { cwd: extracted, env });
      for (const line of linked.trimEnd().split('\n').slice(1)) {
        const dependency = line.trim().split(' (compatibility version')[0];
        if (dependency.startsWith('/System/Library/') || dependency.startsWith('/usr/lib/')
          || (dependency.startsWith('@rpath/') && libraries.includes(basename(dependency)))) continue;
        throw new Error(`Unexpected dependency of ${basename(file)}: ${dependency}`);
      }
    }
    const cli = await exec(helper, ['-version'], { cwd: extracted, env });
    if (!cli.stdout.includes(`ffmpeg version ${mediaVersion}-oq-`)
        || !cli.stdout.includes('--disable-gpl') || !cli.stdout.includes('--disable-nonfree')) {
      throw new Error('Unexpected FFmpeg build in the Studio bundle');
    }
    await verifyAndSaveArchive(root, helper, extracted, env, zip, archive);
  } finally {
    await rm(stage, { recursive: true, force: true });
  }
}

async function packageWindows(root, binary, stage, archive) {
  const name = 'OpenQuartz Studio';
  const app = join(stage, name);
  const source = join(root, 'apps', 'open_quartz_slint', 'runtime');
  const sdk = join(source, 'ffmpeg-shared');
  const runtime = join(app, 'runtime');
  await mkdir(runtime, { recursive: true });
  await copyFile(binary, join(app, basename(binary)));
  await copyFile(join(root, 'LICENSE'), join(app, 'LICENSE.txt'));
  await copyWindowsCrt(app);
  const sdkBin = join(sdk, 'bin');
  for (const file of await readdir(sdkBin)) {
    if (file === 'ffmpeg.exe' || file.toLowerCase().endsWith('.dll')) {
      await copyFile(join(sdkBin, file), join(runtime, file));
    }
  }
  await cp(join(sdk, 'doc'), join(runtime, 'doc'), { recursive: true });
  await copyFile(join(sdk, 'LICENSE.txt'), join(runtime, 'LICENSE.txt'));
  await copyFile(join(source, 'FFMPEG-NOTICE.txt'), join(runtime, 'FFMPEG-NOTICE.txt'));

  const zip = join(stage, 'studio.zip');
  await run('tar.exe', ['-a', '-cf', zip, '-C', stage, name]);
  const extracted = join(stage, 'verification');
  await mkdir(extracted);
  await run('tar.exe', ['-xf', zip, '-C', extracted]);
  const env = { ...process.env };
  delete env.OPEN_QUARTZ_FFMPEG_PATH;
  const packaged = join(extracted, name);
  const { stdout } = await exec(join(packaged, basename(binary)), ['--help'], { cwd: extracted, env });
  if (!stdout.includes('OpenQuartz Studio')) throw new Error('Packaged Studio help did not run');
  const ffmpeg = join(packaged, 'runtime', 'ffmpeg.exe');
  await verifyAndSaveArchive(root, ffmpeg, extracted, env, zip, archive);
}

async function verifyAndSaveArchive(root, ffmpeg, extracted, env, zip, archive) {
  const fixture = join(root, 'crates', 'oq-media', 'tests', 'source-time.mp4');
  const remuxed = join(extracted, 'runtime-check.mp4');
  await exec(ffmpeg, ['-v', 'error', '-i', fixture, '-map', '0:v:0', '-c:v', 'copy', remuxed], { cwd: extracted, env });
  if (!(await stat(remuxed)).size) throw new Error('Packaged media runtime did not produce the fixture remux');
  if (!(await stat(zip)).size) throw new Error('Studio archive is empty');
  await rename(zip, archive);
  console.log(`[video-editor] Verified archive: ${archive}`);
}
