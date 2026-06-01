#!/usr/bin/env node
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const DAILY_JOB_ID = 'fini-daily-issue-report';
const FETCH_JOB_ID = 'fini-fetch-all-branches';
const DAILY_JOB_NAME = 'Fini daily issue and PR report';
const FETCH_JOB_NAME = 'Fini fetch all branches';
const DAILY_JOB_DESCRIPTION = 'Daily Fini issue and pull request report';
const FETCH_JOB_DESCRIPTION = 'Fetch all Fini remote branches every five minutes';
const CRON_EXPR = '0 8 * * *';
const FETCH_EVERY_MS = 5 * 60 * 1000;
const DAILY_MESSAGE = 'Use the fini-daily skill. Run from ~/projects/fini. Use FINI_DAILY_TG_TARGET, FINI_PROGRESS_TG_TARGET, FINI_REPO, and FINI_DAILY_RECIPIENT from the local agent environment when they are set. Query current open GitHub issues and pull requests using configured GitHub access without printing secrets, including the GitHub URL for each item. Run or load triage before choosing the recommendation. Call out stale, blocked, or near-ready pull requests and prefer finishing a stale or close PR over starting a new issue when triage supports it. Produce the daily report format with a configured-recipient greeting only when FINI_DAILY_RECIPIENT is set, and with full GitHub links for every listed issue and pull request. Deliver the final report to FINI_DAILY_TG_TARGET.';
const FETCH_MESSAGE = 'From ~/projects/fini, run git fetch --all --prune to update every remote branch reference. Do not switch branches, merge, rebase, reset, clean, edit files, or push. Report only if the fetch fails, including the command and error summary.';

function usage() {
  return `Usage: node upsert-fini-daily-cron.mjs [--dry-run|--write] [--store <path>]\n\nDefaults to --dry-run and ~/.openclaw/cron/jobs.json.`;
}

function parseArgs(argv) {
  const options = {
    dryRun: true,
    store: path.join(os.homedir(), '.openclaw', 'cron', 'jobs.json'),
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--dry-run') {
      options.dryRun = true;
    } else if (arg === '--write') {
      options.dryRun = false;
    } else if (arg === '--store') {
      const value = argv[i + 1];
      if (!value) throw new Error('--store requires a path');
      options.store = value.startsWith('~') ? path.join(os.homedir(), value.slice(1)) : value;
      i += 1;
    } else if (arg === '--help' || arg === '-h') {
      console.log(usage());
      process.exit(0);
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }

  return options;
}

function parseDailyTarget(value) {
  if (!value) {
    throw new Error('FINI_DAILY_TG_TARGET is required and should look like <group-id>:topic:<thread-id>');
  }

  const match = value.match(/^(-?\d+):topic:(\d+)$/);
  if (!match) {
    throw new Error('FINI_DAILY_TG_TARGET should look like <group-id>:topic:<thread-id>');
  }

  return {
    chatId: match[1],
    threadId: Number.parseInt(match[2], 10),
  };
}

function localTimezone() {
  return process.env.FINI_DAILY_TZ || Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC';
}

function partsInTimezone(date, timezone) {
  const formatter = new Intl.DateTimeFormat('en-US', {
    timeZone: timezone,
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });
  const entries = formatter.formatToParts(date)
    .filter((part) => part.type !== 'literal')
    .map((part) => [part.type, Number.parseInt(part.value, 10)]);
  const parts = Object.fromEntries(entries);
  if (parts.hour === 24) parts.hour = 0;
  return parts;
}

function zonedTimeToUtcMs(parts, timezone) {
  let guess = Date.UTC(parts.year, parts.month - 1, parts.day, parts.hour, parts.minute, parts.second || 0);
  for (let i = 0; i < 4; i += 1) {
    const actual = partsInTimezone(new Date(guess), timezone);
    const desiredLocalMs = Date.UTC(parts.year, parts.month - 1, parts.day, parts.hour, parts.minute, parts.second || 0);
    const actualLocalMs = Date.UTC(actual.year, actual.month - 1, actual.day, actual.hour, actual.minute, actual.second || 0);
    const delta = desiredLocalMs - actualLocalMs;
    if (delta === 0) break;
    guess += delta;
  }
  return guess;
}

function nextDailyRunAtMs(timezone, nowMs) {
  const now = partsInTimezone(new Date(nowMs), timezone);
  const useToday = now.hour < 8 || (now.hour === 8 && now.minute === 0 && now.second === 0);
  const baseMs = Date.UTC(now.year, now.month - 1, now.day + (useToday ? 0 : 1), 8, 0, 0);
  const base = new Date(baseMs);
  return zonedTimeToUtcMs({
    year: base.getUTCFullYear(),
    month: base.getUTCMonth() + 1,
    day: base.getUTCDate(),
    hour: 8,
    minute: 0,
    second: 0,
  }, timezone);
}

function readStore(storePath) {
  if (!fs.existsSync(storePath)) return { jobs: [] };
  const text = fs.readFileSync(storePath, 'utf8');
  if (!text.trim()) return { jobs: [] };
  const parsed = JSON.parse(text);
  if (Array.isArray(parsed)) return { jobs: parsed };
  if (parsed && Array.isArray(parsed.jobs)) return parsed;
  throw new Error(`Unsupported cron store shape at ${storePath}`);
}

function buildDailyJob(target, timezone, nowMs) {
  return {
    id: DAILY_JOB_ID,
    name: DAILY_JOB_NAME,
    description: DAILY_JOB_DESCRIPTION,
    enabled: true,
    createdAtMs: nowMs,
    schedule: {
      kind: 'cron',
      expr: CRON_EXPR,
      tz: timezone,
      staggerMs: 0,
    },
    sessionTarget: 'isolated',
    wakeMode: 'now',
    payload: {
      kind: 'agentTurn',
      message: DAILY_MESSAGE,
      timeoutSeconds: 900,
    },
    delivery: {
      mode: 'announce',
      channel: 'telegram',
      to: target.chatId,
      threadId: target.threadId,
    },
    state: {
      nextRunAtMs: nextDailyRunAtMs(timezone, nowMs),
    },
    updatedAtMs: nowMs,
  };
}

function buildFetchJob(nowMs) {
  return {
    id: FETCH_JOB_ID,
    name: FETCH_JOB_NAME,
    description: FETCH_JOB_DESCRIPTION,
    enabled: true,
    createdAtMs: nowMs,
    schedule: {
      kind: 'every',
      everyMs: FETCH_EVERY_MS,
    },
    sessionTarget: 'isolated',
    wakeMode: 'now',
    payload: {
      kind: 'agentTurn',
      message: FETCH_MESSAGE,
      timeoutSeconds: 120,
      lightContext: true,
      tools: ['exec'],
      toolsAllow: ['exec'],
    },
    delivery: {
      mode: 'none',
    },
    state: {
      nextRunAtMs: nowMs + FETCH_EVERY_MS,
    },
    updatedAtMs: nowMs,
  };
}

function comparable(job) {
  const copy = JSON.parse(JSON.stringify(job));
  delete copy.createdAtMs;
  delete copy.updatedAtMs;
  delete copy.state;
  return copy;
}

function upsert(store, desired) {
  const jobs = Array.isArray(store.jobs) ? store.jobs : [];
  const index = jobs.findIndex((job) => job && job.id === desired.id);
  const existing = index >= 0 ? jobs[index] : null;
  const finalJob = existing
    ? { ...desired, createdAtMs: existing.createdAtMs || desired.createdAtMs }
    : desired;

  const stateNeedsRepair = existing && (!existing.state || typeof existing.state.nextRunAtMs !== 'number');
  const changed = !existing || stateNeedsRepair || JSON.stringify(comparable(existing)) !== JSON.stringify(comparable(finalJob));
  const nextJobs = [...jobs];
  if (index >= 0) nextJobs[index] = finalJob;
  else nextJobs.push(finalJob);

  return {
    changed,
    existing: Boolean(existing),
    store: { ...store, jobs: nextJobs },
    job: finalJob,
  };
}

function writeStore(storePath, store) {
  fs.mkdirSync(path.dirname(storePath), { recursive: true });
  const tempPath = `${storePath}.${process.pid}.tmp`;
  fs.writeFileSync(tempPath, `${JSON.stringify(store, null, 2)}\n`, 'utf8');
  fs.renameSync(tempPath, storePath);
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const target = parseDailyTarget(process.env.FINI_DAILY_TG_TARGET);
  const timezone = localTimezone();
  const store = readStore(options.store);
  const nowMs = Date.now();
  const dailyResult = upsert(store, buildDailyJob(target, timezone, nowMs));
  const fetchResult = upsert(dailyResult.store, buildFetchJob(nowMs));

  if (!options.dryRun && (dailyResult.changed || fetchResult.changed)) {
    writeStore(options.store, fetchResult.store);
  }

  console.log(JSON.stringify({
    dryRun: options.dryRun,
    changed: dailyResult.changed || fetchResult.changed,
    written: !options.dryRun && (dailyResult.changed || fetchResult.changed),
    store: options.store,
    jobs: [
      {
        jobId: DAILY_JOB_ID,
        changed: dailyResult.changed,
        existing: dailyResult.existing,
        schedule: `${CRON_EXPR} @ ${timezone}`,
        delivery: `${target.chatId}:topic:${target.threadId}`,
      },
      {
        jobId: FETCH_JOB_ID,
        changed: fetchResult.changed,
        existing: fetchResult.existing,
        schedule: 'every 5m',
        delivery: 'none',
      },
    ],
  }, null, 2));
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
