#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const repoDir = expandPath(process.env.FINI_REPO_DIR || '~/projects/fini');
const repo = process.env.FINI_REPO || inferRepo(repoDir);
const mapPath = expandPath(
  process.env.FINI_ISSUE_TOPIC_SYNC_FILE
    || process.env.FINI_ISSUE_TG_TOPIC_MAP
    || path.join(repoDir, 'issue-topic-sync.json'),
);
const configPath = process.env.FINI_TELEGRAM_CONFIG_PATH
  ? expandPath(process.env.FINI_TELEGRAM_CONFIG_PATH)
  : null;

function expandPath(value) {
  return value.startsWith('~/') ? path.join(os.homedir(), value.slice(2)) : value;
}

function inferRepo(cwd) {
  const remote = execFileSync('git', ['config', '--get', 'remote.origin.url'], {
    cwd,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
  const match = remote.match(/github\.com[:/](.+?\/.+?)(?:\.git)?$/);
  if (!match) throw new Error('FINI_REPO is required when remote.origin.url is not a GitHub owner/repo URL');
  return match[1];
}

function runGh(args) {
  return execFileSync('gh', args, {
    cwd: repoDir,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
}

function runGhJson(args) {
  const raw = runGh(args);
  return raw ? JSON.parse(raw) : null;
}

function runGhApiJson(args) {
  const raw = runGh(['api', '--paginate', '--slurp', '-X', 'GET', ...args]);
  const pages = raw ? JSON.parse(raw) : [];
  return pages.flat();
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, 'utf8'));
}

function writeJson(filePath, value) {
  const tempPath = `${filePath}.${process.pid}.tmp`;
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(tempPath, `${JSON.stringify(value, null, 2)}\n`, 'utf8');
  fs.renameSync(tempPath, filePath);
}

function updateIssueEntry(issueKey, updater) {
  const latestMap = fs.existsSync(mapPath) ? readJson(mapPath) : { issues: {} };
  const latestIssues = latestMap.issues || {};
  const nextMap = {
    ...latestMap,
    issues: {
      ...latestIssues,
      [issueKey]: updater(latestIssues[issueKey] || {}, latestMap),
    },
    updatedAt: new Date().toISOString(),
  };
  writeJson(mapPath, nextMap);
  return nextMap;
}

function prNumberFromUrl(url) {
  const match = String(url || '').match(/\/pull\/(\d+)(?:$|[/?#])/);
  return match ? Number.parseInt(match[1], 10) : null;
}

function samePullRequest(leftUrl, rightPr) {
  const leftNumber = prNumberFromUrl(leftUrl);
  return Boolean(leftNumber && rightPr?.number && Number(leftNumber) === Number(rightPr.number));
}

function topicTitle(issue, title) {
  const shortTitle = String(title || '').replace(/^closed\s+/i, '').trim();
  return `closed #${issue} ${shortTitle}`.slice(0, 128);
}

function issueState(issue) {
  return runGhJson(['issue', 'view', String(issue), '--repo', repo, '--json', 'state']).state;
}

function closeIssue(issue) {
  runGh(['issue', 'close', String(issue), '--repo', repo, '--reason', 'completed']);
}

async function sendFinalTopicNote(address, issue, completionPr, closeStatus) {
  await telegram('sendMessage', {
    chat_id: address.chatId,
    message_thread_id: address.topicId,
    text: `Closed after merge: ${completionPr.url}\nIssue #${issue}: ${closeStatus}`,
    disable_web_page_preview: true,
  });
}

function telegramToken() {
  if (process.env.TELEGRAM_BOT_TOKEN) return process.env.TELEGRAM_BOT_TOKEN;
  if (!configPath) {
    throw new Error('Set TELEGRAM_BOT_TOKEN or FINI_TELEGRAM_CONFIG_PATH before reconciling topics');
  }
  const config = readJson(configPath);
  const token = config?.channels?.telegram?.botToken;
  if (!token) throw new Error(`Telegram bot token not found in ${configPath}`);
  return token;
}

async function telegram(method, payload) {
  const response = await fetch(`https://api.telegram.org/bot${telegramToken()}/${method}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(payload),
  });
  const data = await response.json();
  if (!data.ok) {
    throw new Error(`${method} failed: ${data.description || response.statusText}`);
  }
  return data.result;
}

function mergedPr(number) {
  const pr = runGhJson([
    'pr',
    'view',
    String(number),
    '--repo',
    repo,
    '--json',
    'number,title,state,mergedAt,url,closingIssuesReferences',
  ]);
  return pr.state === 'MERGED' || pr.mergedAt ? pr : null;
}

function entryMappedAt(entry, map) {
  const candidates = [
    entry.mappedAt,
    entry.createdAt,
    entry.startedAt,
    entry.topicCreatedAt,
    map.createdAt,
  ];
  for (const value of candidates) {
    const ms = Date.parse(value);
    if (Number.isFinite(ms)) return ms;
  }
  return null;
}

function mergedPrsAfter(timestampMs) {
  return runGhApiJson([
    `repos/${repo}/pulls`,
    '-f', 'state=closed',
    '-f', 'sort=updated',
    '-f', 'direction=desc',
    '-F', 'per_page=100',
  ])
    .filter((pr) => pr.merged_at && Date.parse(pr.merged_at) > timestampMs)
    .map((pr) => ({
      number: pr.number,
      title: pr.title,
      state: 'MERGED',
      mergedAt: pr.merged_at,
      url: pr.html_url,
    }));
}

function prMergedAfter(pr, timestampMs) {
  const mergedAtMs = Date.parse(pr?.mergedAt);
  return Number.isFinite(mergedAtMs) && mergedAtMs > timestampMs;
}

function prClosesIssue(prNumber, issue) {
  const pr = mergedPr(prNumber);
  if (!pr) return null;
  const refs = Array.isArray(pr.closingIssuesReferences) ? pr.closingIssuesReferences : [];
  return refs.some((ref) => Number(ref.number) === Number(issue)) ? pr : null;
}

function findCompletionPr(entry, map) {
  const issue = Number(entry.issue);
  const mappedAt = entryMappedAt(entry, map);
  if (!mappedAt) return null;

  const mappedPrNumber = prNumberFromUrl(entry.pullRequest);
  if (mappedPrNumber) {
    const pr = mergedPr(mappedPrNumber);
    return prMergedAfter(pr, mappedAt) ? pr : null;
  }

  for (const candidate of mergedPrsAfter(mappedAt)) {
    const pr = prClosesIssue(candidate.number, issue);
    if (pr) return pr;
  }
  return null;
}

function parseTopicTarget(value) {
  const match = String(value || '').match(/^(-?\d+):topic:(\d+)$/);
  if (!match) return null;
  return {
    chatId: match[1],
    topicId: Number.parseInt(match[2], 10),
  };
}

function topicAddress(map, entry) {
  const target = parseTopicTarget(entry.issueTarget || entry.target);
  return {
    chatId: target?.chatId || map.chatId,
    topicId: Number(entry.topicId || target?.topicId),
  };
}

async function main() {
  const map = fs.existsSync(mapPath) ? readJson(mapPath) : { issues: {} };
  const changes = [];
  const errors = [];

  for (const [issueKey, entry] of Object.entries(map.issues || {})) {
    try {
      const issue = Number(entry.issue || issueKey);
      const address = topicAddress(map, entry);
      if (!issue || !address.chatId || !address.topicId) continue;

      const completionPr = findCompletionPr({ ...entry, issue }, map);
      if (!completionPr) continue;

      const newTitle = topicTitle(issue, entry.title);
      const alreadyClosed = entry.status === 'closed'
        && entry.finalTopicNoteStatus === 'sent'
        && entry.topicTitle === newTitle
        && samePullRequest(entry.closedByPullRequest, completionPr);
      if (alreadyClosed) continue;

      const closeStatus = issueState(issue) === 'CLOSED' ? 'already closed' : 'closed';

      if (closeStatus === 'closed') {
        closeIssue(issue);
      }

      await telegram('editForumTopic', {
        chat_id: address.chatId,
        message_thread_id: address.topicId,
        name: newTitle,
      });

      await telegram('sendChatAction', {
        chat_id: address.chatId,
        message_thread_id: address.topicId,
        action: 'typing',
      });

      updateIssueEntry(issueKey, (latestEntry) => ({
        ...latestEntry,
        issue,
        topicId: address.topicId,
        issueTarget: latestEntry.issueTarget || latestEntry.target || entry.issueTarget || `${address.chatId}:topic:${address.topicId}`,
        status: 'closed',
        closedAt: completionPr.mergedAt || new Date().toISOString(),
        closedByPullRequest: completionPr.url,
        finalTopicNoteStatus: 'pending',
        topicTitle: newTitle,
      }));

      await sendFinalTopicNote(address, issue, completionPr, closeStatus);

      updateIssueEntry(issueKey, (latestEntry) => ({
        ...latestEntry,
        finalTopicNoteStatus: 'sent',
        finalTopicNoteSentAt: new Date().toISOString(),
      }));
      changes.push(`#${issue} via PR #${completionPr.number}`);
    } catch (error) {
      errors.push(`${issueKey}: ${error instanceof Error ? error.message : String(error)}`);
    }
  }

  console.log(JSON.stringify({ changed: changes.length > 0, changes, errors }, null, 2));
  if (errors.length > 0) process.exitCode = 1;
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
