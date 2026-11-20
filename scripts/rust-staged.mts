// Pre-commit checks for staged Rust files. lint-staged calls this script
// with the staged `.rs` files; it runs only when such files exist.
//
//   node scripts/rust-staged.mts fmt [files...]     rustfmt on the staged content of each file
//   node scripts/rust-staged.mts clippy [files...]  cargo clippy on each crate with a staged file
//
// rustfmt formats full files and clippy checks the full crate, but this script
// fails only on problems on the lines that `git diff --cached -U0` shows as
// added or changed. Old problems in other lines do not stop the commit.
// Compile errors and files that rustfmt cannot parse always stop it.
// Without files, both modes use the staged `.rs` files.

import { spawnSync } from 'node:child_process';
import type { SpawnSyncOptions } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const MAX_BUFFER = 512 * 1024 * 1024;
const [mode, ...fileArgs] = process.argv.slice(2);

function run(command: string, args: string[], options: SpawnSyncOptions = {}) {
  const result = spawnSync(command, args, { maxBuffer: MAX_BUFFER, ...options });
  if ((result.error as NodeJS.ErrnoException | undefined)?.code === 'ENOENT') {
    console.error(`pre-commit: "${command}" is not on PATH, so the Rust checks cannot run.`);
    console.error('pre-commit: install the Rust toolchain (rustup, with the clippy and rustfmt components),');
    console.error('pre-commit: or skip the checks one time with "git commit --no-verify".');
    process.exit(1);
  }
  if (result.error) throw result.error;
  return result;
}

function git(args: string[]): string {
  const result = run('git', args, { encoding: 'utf8' });
  if (result.status !== 0) {
    console.error(result.stderr);
    process.exit(2);
  }
  return result.stdout as string;
}

const root = path.resolve(git(['rev-parse', '--show-toplevel']).trim());
const toRepoPath = (file: string) => path.relative(root, path.resolve(root, file)).split(path.sep).join('/');

function stagedRustFiles(): string[] {
  const files = fileArgs.length > 0
    ? fileArgs
    : git(['diff', '--cached', '--name-only', '--diff-filter=ACMR', '--', '*.rs']).split('\n').filter(Boolean);
  return files.filter((file) => file.endsWith('.rs')).map(toRepoPath);
}

// The nearest directory above the file that has a Cargo.toml.
function crateDir(repoPath: string): string | undefined {
  let dir = path.dirname(path.join(root, repoPath));
  while (dir.startsWith(root)) {
    if (existsSync(path.join(dir, 'Cargo.toml'))) return dir;
    const parent = path.dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return undefined;
}

function edition(dir: string | undefined): string {
  if (!dir) return '2021';
  const match = /^edition\s*=\s*"(\d+)"/m.exec(readFileSync(path.join(dir, 'Cargo.toml'), 'utf8'));
  return match ? match[1] : '2021';
}

type Range = [number, number];

const overlaps = (ranges: Range[] | undefined, [start, end]: Range) =>
  ranges?.some(([from, to]) => start <= to && end >= from) ?? false;

// rustfmt reads the staged content from stdin. Then it checks only this file,
// not the modules that the file declares with `mod`. The formatted output is
// compared with the staged content, and only the rustfmt changes that touch
// staged lines stop the commit.
function fmt(files: string[]): number {
  const changed = changedLines(files);
  const temp = mkdtempSync(path.join(tmpdir(), 'rust-staged-'));
  let onChangedLines = 0;
  let elsewhere = 0;
  let broken = 0;
  try {
    for (const file of files) {
      const staged = run('git', ['show', `:${file}`], { encoding: 'utf8' });
      if (staged.status !== 0) continue;
      const args = ['--edition', edition(crateDir(file))];
      const formatted = run('rustfmt', args, { input: staged.stdout, encoding: 'utf8' });
      if (formatted.status !== 0) {
        broken += 1;
        console.error(`rustfmt: cannot format ${file}:\n${String(formatted.stderr).replaceAll('<stdin>', file)}`);
        continue;
      }
      if (formatted.stdout === staged.stdout) continue;

      const before = path.join(temp, 'staged.rs');
      const after = path.join(temp, 'formatted.rs');
      writeFileSync(before, String(staged.stdout));
      writeFileSync(after, String(formatted.stdout));
      const diff = run('git', ['diff', '--no-index', '--no-color', '-U0', '--', before, after], { encoding: 'utf8' });

      // Each hunk is one rustfmt change; its old side is a line range of the staged file.
      const hunks: { range: Range; text: string[] }[] = [];
      for (const line of String(diff.stdout).split('\n')) {
        const header = /^@@ -(\d+)(?:,(\d+))? \+\d+(?:,\d+)? @@/.exec(line);
        if (header) {
          const start = Number(header[1]);
          const count = header[2] === undefined ? 1 : Number(header[2]);
          // A pure insertion goes between line `start` and the next line.
          const range: Range = count === 0 ? [Math.max(start, 1), start + 1] : [start, start + count - 1];
          hunks.push({ range, text: [] });
        } else if (hunks.length > 0 && /^[-+]/.test(line)) {
          hunks[hunks.length - 1].text.push(line);
        }
      }
      for (const hunk of hunks) {
        if (overlaps(changed.get(file), hunk.range)) {
          onChangedLines += 1;
          console.error(`rustfmt: ${file}:${hunk.range[0]} is not formatted:\n${hunk.text.join('\n')}\n`);
        } else {
          elsewhere += 1;
        }
      }
    }
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }

  if (broken > 0 || onChangedLines > 0) {
    if (onChangedLines > 0) {
      console.error(`rustfmt: ${onChangedLines} place(s) on staged lines need formatting (rustfmt shows the fix above).`);
    }
    return 1;
  }
  console.error(`rustfmt: staged lines are formatted (${elsewhere} place(s) on other lines were ignored).`);
  return 0;
}

// Added or changed line ranges of each staged file, from the zero-context diff.
function changedLines(files: string[]): Map<string, Range[]> {
  const diff = git([
    '-c', 'core.quotePath=false',
    'diff', '--cached', '-U0', '--no-color', '--no-ext-diff', '--src-prefix=a/', '--dst-prefix=b/',
    '--diff-filter=ACMR', '--', ...files,
  ]);
  const changed = new Map<string, Range[]>();
  let ranges: Range[] | undefined;
  for (const line of diff.split('\n')) {
    if (line.startsWith('+++ ')) {
      ranges = line.startsWith('+++ b/') ? [] : undefined;
      if (ranges) changed.set(line.slice('+++ b/'.length), ranges);
      continue;
    }
    const hunk = /^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@/.exec(line);
    if (hunk && ranges) {
      const start = Number(hunk[1]);
      const count = hunk[2] === undefined ? 1 : Number(hunk[2]);
      if (count > 0) ranges.push([start, start + count - 1]);
    }
  }
  return changed;
}

type Span = { file_name: string; line_start: number; line_end: number; is_primary: boolean };
type Diagnostic = { level: string; rendered: string | null; spans: Span[] };

function clippy(files: string[]): number {
  const changed = changedLines(files);
  const crates = new Set(files.map(crateDir).filter((dir): dir is string => dir !== undefined));
  let failed = false;

  for (const dir of crates) {
    const manifest = path.join(dir, 'Cargo.toml');
    console.error(`clippy: checking ${toRepoPath(manifest)} (only warnings on staged lines stop the commit)`);
    const result = run(
      'cargo',
      ['clippy', '--manifest-path', manifest, '--all-targets', '--all-features', '--message-format=json'],
      { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] },
    );

    const reported = new Set<string>();
    let errors = 0;
    let onChangedLines = 0;
    let elsewhere = 0;
    for (const line of String(result.stdout).split('\n')) {
      if (!line.startsWith('{')) continue;
      const message = JSON.parse(line) as { reason: string; manifest_path?: string; message?: Diagnostic };
      if (message.reason !== 'compiler-message' || !message.message) continue;
      const diagnostic = message.message;
      const rendered = diagnostic.rendered ?? '';
      if (reported.has(rendered)) continue;

      if (diagnostic.level === 'error' || diagnostic.level === 'error: internal compiler error') {
        reported.add(rendered);
        errors += 1;
        console.error(rendered);
        continue;
      }
      if (diagnostic.level !== 'warning') continue;

      const bases = [dir, path.dirname(message.manifest_path ?? manifest)];
      const hit = diagnostic.spans.some((span) => span.is_primary && bases.some((base) => {
        const ranges = changed.get(toRepoPath(path.resolve(base, span.file_name)));
        return ranges?.some(([start, end]) => span.line_start <= end && span.line_end >= start);
      }));
      reported.add(rendered);
      if (hit) {
        onChangedLines += 1;
        console.error(rendered);
      } else {
        elsewhere += 1;
      }
    }

    if (errors > 0 || (result.status !== 0 && onChangedLines === 0)) {
      console.error(`clippy: ${toRepoPath(manifest)} does not compile (cargo exit code ${result.status}).`);
      failed = true;
    } else if (onChangedLines > 0) {
      console.error(`clippy: ${onChangedLines} warning(s) on staged lines. Fix them, or use #[allow(...)] with a reason.`);
      failed = true;
    } else {
      console.error(`clippy: no warnings on staged lines (${elsewhere} warning(s) on other lines were ignored).`);
    }
  }
  return failed ? 1 : 0;
}

const files = stagedRustFiles();
if (mode === 'fmt') {
  process.exit(fmt(files));
} else if (mode === 'clippy') {
  process.exit(clippy(files));
} else {
  console.error('usage: rust-staged.mts fmt|clippy [files...]');
  process.exit(2);
}
