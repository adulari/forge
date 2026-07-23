#!/usr/bin/env node
'use strict';

const { spawn } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');

const argv = process.argv.slice(2);
const requireCacheHit = argv.includes('--require-cache-hit');
const positional = argv.filter(arg => !arg.startsWith('--'));
const model = positional[0];
if (!model) {
  console.error('usage: cache_session_probe.js MODEL [WORKDIR] [--require-cache-hit]');
  process.exit(2);
}

const repoRoot = path.resolve(__dirname, '../..');
const outRoot = process.env.FORGE_MANUAL_E2E_OUT
  || path.join(
    process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share'),
    'forge',
    'manual-e2e-runs',
  );
const safeModel = model.replace(/[^a-zA-Z0-9_.-]+/g, '-').replace(/^-|-$/g, '');
fs.mkdirSync(outRoot, { recursive: true });
const cwd = positional[1]
  ? path.resolve(positional[1])
  : fs.mkdtempSync(path.join(outRoot, `cache-${safeModel}-`));
fs.mkdirSync(cwd, { recursive: true });

const checkoutForge = path.join(repoRoot, 'target', 'debug', 'forge');
const forge = process.env.FORGE_BIN
  || (fs.existsSync(checkoutForge) ? checkoutForge : 'forge');

// Use meaningful, unique records rather than repeated filler. Some models correctly treat a huge
// repeated string followed by an exact-output instruction as prompt-injection-shaped; that tests
// model policy, not Forge's transport or provider cache. These records retain a long byte-stable
// prefix while presenting a legitimate technical ledger review.
const stablePrefix = Array.from({ length: 640 }, (_, index) => {
  const id = String(index + 1).padStart(4, '0');
  const checksum = ((index + 1) * 7919 % 104729).toString(16).padStart(5, '0');
  return `Cache ledger record ${id}: namespace forge.transport.segment.${id}; `
    + `checksum ${checksum}; invariant: preserve ordering, identity, and UTF-8 boundaries.`;
}).join('\n');
const firstPrompt = 'Review the following technical cache ledger as a transport acceptance test. '
  + 'The records are intentionally numerous and uniquely numbered to exercise a long stable '
  + `context prefix.\n\n${stablePrefix}\n\nAcceptance instruction: do not call tools and reply `
  + 'with exactly CACHE_FIRST_OK.';
const secondPrompt = 'Continue the same transport acceptance test using the ledger already in this '
  + 'session. Do not call tools. Reply with exactly CACHE_SECOND_OK.';

function run(args, timeoutMs = Number(process.env.FORGE_CACHE_E2E_TIMEOUT_MS || 300000)) {
  return new Promise(resolve => {
    const started = Date.now();
    const child = spawn(forge, args, { cwd, stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.on('data', chunk => { stdout += chunk.toString(); });
    child.stderr.on('data', chunk => { stderr += chunk.toString(); });
    const timeout = setTimeout(() => {
      child.kill('SIGTERM');
      setTimeout(() => child.kill('SIGKILL'), 2000).unref();
    }, timeoutMs);
    child.on('close', (code, signal) => {
      clearTimeout(timeout);
      const events = stdout.split(/\r?\n/).filter(Boolean).flatMap(line => {
        try { return [JSON.parse(line)]; } catch { return []; }
      });
      resolve({ code, signal, durationMs: Date.now() - started, events, stderr });
    });
  });
}

function summarize(runResult) {
  const init = runResult.events.find(event => event.type === 'system' && event.subtype === 'init');
  const usageEvents = runResult.events.filter(
    event => event.type === 'system' && event.subtype === 'usage'
  );
  const result = [...runResult.events].reverse().find(event => event.type === 'result');
  const warnings = runResult.events
    .filter(event => event.type === 'system' && ['warning', 'error'].includes(event.subtype))
    .map(event => event.message);
  return {
    code: runResult.code,
    signal: runResult.signal,
    durationMs: runResult.durationMs,
    sessionId: init?.session_id || result?.session_id || null,
    usage: usageEvents.at(-1)?.usage || null,
    result: result?.result || null,
    stopReason: result?.stop_reason || null,
    warnings,
    stderr: runResult.stderr.trim().slice(-2000),
  };
}

(async () => {
  const firstRun = await run([
    'run', '--mode', 'bypass', '--output-format', 'stream-json', '--model', model, firstPrompt,
  ]);
  const first = summarize(firstRun);
  if (!first.sessionId || first.code !== 0) {
    const failed = { model, prefixCharacters: stablePrefix.length, first };
    const reportPath = path.join(cwd, 'cache-report.json');
    fs.writeFileSync(reportPath, `${JSON.stringify(failed, null, 2)}\n`);
    console.log(JSON.stringify({ ...failed, reportPath }, null, 2));
    process.exit(1);
  }

  const secondRun = await run([
    'run', '--resume', first.sessionId, '--mode', 'bypass', '--output-format', 'stream-json',
    '--model', model, secondPrompt,
  ]);
  const second = summarize(secondRun);
  const cachedTokensReported = Number(second.usage?.cached_input_tokens || 0);
  const valid = first.code === 0
    && second.code === 0
    && first.sessionId === second.sessionId
    && first.result === 'CACHE_FIRST_OK'
    && second.result === 'CACHE_SECOND_OK'
    && first.usage !== null
    && second.usage !== null
    && (!requireCacheHit || cachedTokensReported > 0);
  const reportPath = path.join(cwd, 'cache-report.json');
  const report = {
    valid,
    model,
    forge,
    workspace: cwd,
    prefixCharacters: stablePrefix.length,
    requireCacheHit,
    cachedTokensReported,
    first,
    second,
  };
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify({ ...report, reportPath }, null, 2));
  if (!valid) process.exit(1);
})().catch(error => {
  console.error(error.stack || String(error));
  process.exit(1);
});
