#!/usr/bin/env node
import { readdir, readFile, writeFile } from 'node:fs/promises';
import { basename, join } from 'node:path';
import { pathToFileURL } from 'node:url';

const ARCHES = new Map([
  ['x64', 'x86_64'],
  ['arm64', 'aarch64'],
]);

const LINUX_SUFFIXES = new Map([
  ['.AppImage', 'appimage'],
  ['.deb', 'deb'],
  ['.rpm', 'rpm'],
]);

export async function generateUpdaterManifest({
  assetsDir,
  repo,
  tag,
  version,
  notes = '',
  output,
  surface = 'desktop',
  pubDate = new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
}) {
  if (!['desktop', 'cli'].includes(surface)) {
    throw new Error(`unsupported updater manifest surface: ${surface}`);
  }

  const targetForSurface = surface === 'cli' ? cliPlatformTarget : platformTarget;
  const platforms = {};
  const entries = await readdir(assetsDir, { withFileTypes: true });

  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    if (!entry.isFile() || entry.name.endsWith('.sig') || entry.name.endsWith('.json')) {
      continue;
    }

    const target = targetForSurface(entry.name);
    if (!target) {
      continue;
    }

    const sigPath = join(assetsDir, `${entry.name}.sig`);
    let signature;
    try {
      signature = (await readFile(sigPath, 'utf8')).trim();
    } catch (error) {
      if (error?.code === 'ENOENT') {
        throw new Error(`missing Tauri updater signature for ${entry.name}: expected ${basename(sigPath)}`);
      }
      throw error;
    }

    if (!signature) {
      throw new Error(`empty Tauri updater signature for ${entry.name}: ${basename(sigPath)}`);
    }

    const manifestEntry = {
      signature,
      url: githubReleaseDownloadUrl(repo, tag, entry.name),
    };
    platforms[target] = manifestEntry;

    const fallback = fallbackTarget(target);
    if (fallback && !platforms[fallback]) {
      platforms[fallback] = manifestEntry;
    }
  }

  if (Object.keys(platforms).length === 0) {
    throw new Error(`no supported signed desktop updater artifacts found in ${assetsDir}`);
  }

  const manifest = {
    version,
    notes,
    pub_date: pubDate,
    platforms: Object.fromEntries(Object.entries(platforms).sort(([a], [b]) => a.localeCompare(b))),
  };

  await writeFile(output, `${JSON.stringify(manifest, null, 2)}\n`, 'utf8');
  return manifest;
}

export function cliPlatformTarget(filename) {
  for (const [archLabel, arch] of ARCHES) {
    if (filename.endsWith(`-linux-${archLabel}-cli.tar.gz`)) {
      return `cli-linux-${arch}`;
    }

    if (filename.endsWith(`-windows-${archLabel}-cli.zip`)) {
      return `cli-windows-${arch}`;
    }
  }

  return null;
}

export function platformTarget(filename) {
  for (const [archLabel, arch] of ARCHES) {
    if (filename.includes(`-linux-${archLabel}`)) {
      for (const [suffix, installer] of LINUX_SUFFIXES) {
        if (filename.endsWith(suffix)) {
          return `linux-${arch}-${installer}`;
        }
      }
    }

    if (filename.endsWith(`-windows-${archLabel}-setup.exe`)) {
      return `windows-${arch}-nsis`;
    }
  }

  return null;
}

export function fallbackTarget(target) {
  if (target.endsWith('-appimage') || target.endsWith('-nsis')) {
    return target.slice(0, target.lastIndexOf('-'));
  }
  return null;
}

function githubReleaseDownloadUrl(repo, tag, filename) {
  return `https://github.com/${repo}/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(filename)}`;
}

function parseCliArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 2) {
    const key = argv[index];
    const value = argv[index + 1];
    if (!key?.startsWith('--') || value === undefined) {
      throw new Error(`invalid arguments near ${key ?? '<end>'}`);
    }
    parsed[key.slice(2)] = value;
  }

  for (const required of ['assets-dir', 'repo', 'tag', 'version', 'output']) {
    if (!parsed[required]) {
      throw new Error(`missing required --${required}`);
    }
  }

  return {
    assetsDir: parsed['assets-dir'],
    repo: parsed.repo,
    tag: parsed.tag,
    version: parsed.version,
    notes: parsed.notes ?? '',
    output: parsed.output,
    surface: parsed.surface ?? 'desktop',
    pubDate: parsed['pub-date'],
  };
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    await generateUpdaterManifest(parseCliArgs(process.argv.slice(2)));
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  }
}
