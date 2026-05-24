#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const repo = process.env.FINI_REPO || 'VRuzhentsov/fini';
const repoDir = expandPath(process.env.FINI_REPO_DIR || '~/projects/fini');
const mapPath = expandPath(process.env.FINI_ISSUE_TG_TOPIC_MAP || '~/.openclaw/workspace/fini-issue-topics.json');
const configPath = expandPath(process.env.OPENCLAW_CONFIG_PATH || '~/.openclaw/openclaw.json');

function expandPath(value) {
  return value.startsWith('~/') ? path.join(os.homedir(), value.slice(2)) : value;
}

function runGh(args) {
  return execFileSync('gh', args, {
    cwd: repoDir,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
}

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, 'utf8'));
}

function writeJson(filePath, value) {
  const tempPath = `${filePath}.${process.pid}.tmp`;
  fs.writeFileSync(tempPath, `${JSON.stringify(value, null, 2)}\n`, 'utf8');
  fs.renameSync(tempPath, filePath);
}

function prNumberFromUrl(url) {
  const match = String(url || '').match(/\/pull\/(\d+)(?:$|[/?#])/);
  return match ? Number.parseInt(match[1], 10) : null;
}

function topicTitle(issue, title) {
  const shortTitle = String(title || '').replace(/^closed\s+/i, '').trim();
  return `closed #${issue} ${shortTitle}`.slice(0, 128);
}

function issueState(issue) {
  const raw = runGh(['issue', 'view', String(issue), '--repo', repo, '--json', 'state']);
  return JSON.parse(raw).state;
}

function closeIssue(issue) {
  runGh(['issue', 'close', String(issue), '--repo', repo, '--reason', 'completed']);
}

function telegramToken() {
  if (process.env.TELEGRAM_BOT_TOKEN) return process.env.TELEGRAM_BOT_TOKEN;
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

function mergedPrs() {
  const raw = runGh([
    'pr',
    'list',
    '--repo',
    repo,
    '--state',
    'all',
    '--limit',
    '100',
    '--json',
    'number,title,state,mergedAt,url,closingIssuesReferences',
  ]);
  return JSON.parse(raw).filter((pr) => pr.state === 'MERGED' || pr.mergedAt);
}

function findCompletionPr(entry, prs) {
  const mappedPrNumber = prNumberFromUrl(entry.pullRequest);
  if (mappedPrNumber) {
    const mapped = prs.find((pr) => pr.number === mappedPrNumber);
    if (mapped) return mapped;
  }

  return prs.find((pr) => {
    const refs = Array.isArray(pr.closingIssuesReferences) ? pr.closingIssuesReferences : [];
    return refs.some((ref) => Number(ref.number) === Number(entry.issue));
  });
}

async function main() {
  const map = readJson(mapPath);
  const chatId = map.chatId;
  if (!chatId) throw new Error(`Missing chatId in ${mapPath}`);

  const prs = mergedPrs();
  const changes = [];

  for (const [issueKey, entry] of Object.entries(map.issues || {})) {
    const issue = Number(entry.issue || issueKey);
    if (!issue || !entry.topicId) continue;

    const completionPr = findCompletionPr({ ...entry, issue }, prs);
    if (!completionPr) continue;

    const newTitle = topicTitle(issue, entry.title);
    const alreadyClosed = entry.status === 'closed' && entry.topicTitle === newTitle && entry.closedByPullRequest === completionPr.url;
    if (alreadyClosed) continue;

    let closeStatus = 'already closed';
    if (issueState(issue) !== 'CLOSED') {
      closeIssue(issue);
      closeStatus = 'closed';
    }

    await telegram('editForumTopic', {
      chat_id: chatId,
      message_thread_id: Number(entry.topicId),
      name: newTitle,
    });

    await telegram('sendMessage', {
      chat_id: chatId,
      message_thread_id: Number(entry.topicId),
      text: `Closed after merge: ${completionPr.url}\nIssue #${issue}: ${closeStatus}`,
      disable_web_page_preview: true,
    });

    map.issues[issueKey] = {
      ...entry,
      issue,
      status: 'closed',
      closedAt: completionPr.mergedAt || new Date().toISOString(),
      closedByPullRequest: completionPr.url,
      topicTitle: newTitle,
    };
    changes.push(`#${issue} via PR #${completionPr.number}`);
  }

  if (changes.length > 0) {
    map.updatedAt = new Date().toISOString();
    writeJson(mapPath, map);
  }

  console.log(JSON.stringify({ changed: changes.length > 0, changes }, null, 2));
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
