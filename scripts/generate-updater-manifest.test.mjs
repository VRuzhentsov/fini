import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import {
  cliPlatformTarget,
  generateUpdaterManifest,
  platformTarget,
} from './generate-updater-manifest.mjs';

async function withAssets(files, fn) {
  const dir = await mkdtemp(join(tmpdir(), 'fini-updater-manifest-'));
  try {
    for (const [name, signature] of Object.entries(files)) {
      await writeFile(join(dir, name), 'artifact', 'utf8');
      await writeFile(join(dir, `${name}.sig`), signature, 'utf8');
    }
    return await fn(dir);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

test('generates static updater manifest for desktop bundles', async () => {
  await withAssets(
    {
      'fini-v1.2.3-linux-x64.AppImage': 'sig-appimage-x64',
      'fini-v1.2.3-linux-x64.deb': 'sig-deb-x64',
      'fini-v1.2.3-linux-x64.rpm': 'sig-rpm-x64',
      'fini-v1.2.3-linux-arm64.AppImage': 'sig-appimage-arm64',
      'fini-v1.2.3-windows-x64-setup.exe': 'sig-windows-x64',
      'fini-v1.2.3-windows-arm64-setup.exe': 'sig-windows-arm64',
      'fini-v1.2.3-linux-x64-cli.tar.gz': 'ignored-cli-signature',
    },
    async (assetsDir) => {
      const output = join(assetsDir, 'latest.json');
      await generateUpdaterManifest({
        assetsDir,
        repo: 'VRuzhentsov/fini',
        tag: 'v1.2.3',
        version: '1.2.3',
        notes: 'test notes',
        output,
        pubDate: '2026-07-07T00:00:00Z',
      });

      const manifest = JSON.parse(await readFile(output, 'utf8'));
      assert.equal(manifest.version, '1.2.3');
      assert.equal(manifest.notes, 'test notes');
      assert.equal(manifest.pub_date, '2026-07-07T00:00:00Z');

      assert.deepEqual(manifest.platforms['linux-x86_64-appimage'], {
        signature: 'sig-appimage-x64',
        url: 'https://github.com/VRuzhentsov/fini/releases/download/v1.2.3/fini-v1.2.3-linux-x64.AppImage',
      });
      assert.equal(manifest.platforms['linux-x86_64-deb'].signature, 'sig-deb-x64');
      assert.equal(manifest.platforms['linux-x86_64-rpm'].signature, 'sig-rpm-x64');
      assert.equal(manifest.platforms['linux-aarch64-appimage'].signature, 'sig-appimage-arm64');
      assert.equal(manifest.platforms['windows-x86_64-nsis'].signature, 'sig-windows-x64');
      assert.equal(manifest.platforms['windows-aarch64-nsis'].signature, 'sig-windows-arm64');

      assert.deepEqual(manifest.platforms['linux-x86_64'], manifest.platforms['linux-x86_64-appimage']);
      assert.deepEqual(manifest.platforms['windows-x86_64'], manifest.platforms['windows-x86_64-nsis']);
      assert.equal(manifest.platforms['cli-linux-x86_64'], undefined);
    },
  );
});

test('fails when a supported desktop artifact is missing its Tauri updater signature', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'fini-updater-manifest-'));
  try {
    await writeFile(join(dir, 'fini-v1.2.3-linux-x64.AppImage'), 'artifact', 'utf8');
    await assert.rejects(
      () => generateUpdaterManifest({
        assetsDir: dir,
        repo: 'VRuzhentsov/fini',
        tag: 'v1.2.3',
        version: '1.2.3',
        notes: '',
        output: join(dir, 'latest.json'),
        pubDate: '2026-07-07T00:00:00Z',
      }),
      /missing Tauri updater signature/,
    );
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});

test('recognizes only GUI installer artifacts as updater platforms', () => {
  assert.equal(platformTarget('fini-v1.2.3-linux-x64.AppImage'), 'linux-x86_64-appimage');
  assert.equal(platformTarget('fini-v1.2.3-linux-x64-cli.tar.gz'), null);
  assert.equal(platformTarget('fini-v1.2.3-android.apk'), null);
});

test('recognizes standalone CLI archive targets separately from desktop installers', () => {
  assert.equal(cliPlatformTarget('fini-v1.2.3-linux-x64-cli.tar.gz'), 'cli-linux-x86_64');
  assert.equal(cliPlatformTarget('fini-v1.2.3-linux-arm64-cli.tar.gz'), 'cli-linux-aarch64');
  assert.equal(cliPlatformTarget('fini-v1.2.3-windows-x64-cli.zip'), 'cli-windows-x86_64');
  assert.equal(cliPlatformTarget('fini-v1.2.3-windows-arm64-cli.zip'), 'cli-windows-aarch64');
  assert.equal(cliPlatformTarget('fini-v1.2.3-linux-x64.AppImage'), null);
});

test('generates a CLI-only updater manifest from signed CLI archives', async () => {
  await withAssets(
    {
      'fini-v1.2.3-linux-x64.AppImage': 'desktop-signature',
      'fini-v1.2.3-linux-x64-cli.tar.gz': 'cli-linux-signature',
      'fini-v1.2.3-windows-arm64-cli.zip': 'cli-windows-signature',
    },
    async (assetsDir) => {
      const output = join(assetsDir, 'latest-cli.json');
      const manifest = await generateUpdaterManifest({
        assetsDir,
        repo: 'VRuzhentsov/fini',
        tag: 'v1.2.3',
        version: '1.2.3',
        output,
        pubDate: '2026-07-07T00:00:00Z',
        surface: 'cli',
      });

      assert.deepEqual(manifest.platforms, {
        'cli-linux-x86_64': {
          signature: 'cli-linux-signature',
          url: 'https://github.com/VRuzhentsov/fini/releases/download/v1.2.3/fini-v1.2.3-linux-x64-cli.tar.gz',
        },
        'cli-windows-aarch64': {
          signature: 'cli-windows-signature',
          url: 'https://github.com/VRuzhentsov/fini/releases/download/v1.2.3/fini-v1.2.3-windows-arm64-cli.zip',
        },
      });
    },
  );
});
