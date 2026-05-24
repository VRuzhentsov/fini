#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const repo = process.env.FINI_REPO || 'VRuzhentsov/fini';
const repoDir = expandPath(process.env.FINI_REPO_DIR || '~/projects/fini');
const mapPath = expandPath(process.env.FINI_ISSUE_TG_TOPIC_MAP || path.join(repoDir, '.fini-dev', 'fini-issue-topics.json'));
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
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
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

function mergedPr(number) {
  const raw = runGh([
    'pr',
    'view',
    String(number),
    '--repo',
    repo,
    '--json',
    'number,title,state,mergedAt,url,closingIssuesReferences',
  ]);
  const pr = JSON.parse(raw);
  return pr.state === 'MERGED' || pr.mergedAt ? pr : null;
}

function findCompletionPr(entry) {
  const mappedPrNumber = prNumberFromUrl(entry.pullRequest);
  if (!mappedPrNumber) return null;

  const pr = mergedPr(mappedPrNumber);
  if (!pr) return null;

  const refs = Array.isArray(pr.closingIssuesReferences) ? pr.closingIssuesReferences : [];
  const closesIssue = refs.some((ref) => Number(ref.number) === Number(entry.issue));
  return closesIssue || entry.pullRequest === pr.url ? pr : null;
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

      const completionPr = findCompletionPr({ ...entry, issue });
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
        chat_id: address.chatId,
        message_thread_id: address.topicId,
        name: newTitle,
      });

      await telegram('sendMessage', {
        chat_id: address.chatId,
        message_thread_id: address.topicId,
        text: `Closed after merge: ${completionPr.url}\nIssue #${issue}: ${closeStatus}`,
        disable_web_page_preview: true,
      });

      map.issues[issueKey] = {
        ...entry,
        issue,
        topicId: address.topicId,
        issueTarget: entry.issueTarget || `${address.chatId}:topic:${address.topicId}`,
        status: 'closed',
        closedAt: completionPr.mergedAt || new Date().toISOString(),
        closedByPullRequest: completionPr.url,
        topicTitle: newTitle,
      };
      map.updatedAt = new Date().toISOString();
      writeJson(mapPath, map);
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
