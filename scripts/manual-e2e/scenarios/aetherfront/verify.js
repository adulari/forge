#!/usr/bin/env node
'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const { pathToFileURL } = require('url');
const { spawn } = require('child_process');
const CDP = require('/opt/Antigravity/resources/app/node_modules/chrome-remote-interface');

const pagePath = path.resolve(process.argv[2]);
const screenshotPath = path.resolve(process.argv[3]);
const port = 19333 + Math.floor(Math.random() * 1000);
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'forge-game-chrome-'));
const chrome = spawn('/usr/bin/google-chrome-stable', [
  '--headless=new',
  '--no-sandbox',
  '--disable-dev-shm-usage',
  '--hide-scrollbars',
  '--mute-audio',
  '--autoplay-policy=no-user-gesture-required',
  '--window-size=1440,900',
  `--remote-debugging-port=${port}`,
  `--user-data-dir=${profile}`,
  'about:blank',
], { stdio: ['ignore', 'ignore', 'pipe'] });

let chromeStderr = '';
chrome.stderr.on('data', chunk => {
  chromeStderr += chunk.toString();
  if (chromeStderr.length > 20000) chromeStderr = chromeStderr.slice(-20000);
});

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

async function connect() {
  let lastError;
  for (let attempt = 0; attempt < 50; attempt += 1) {
    try {
      return await CDP({ port });
    } catch (error) {
      lastError = error;
      await sleep(100);
    }
  }
  throw lastError;
}

async function main() {
  if (!fs.existsSync(pagePath)) throw new Error(`missing page: ${pagePath}`);
  const source = fs.readFileSync(pagePath, 'utf8');
  const marker = 'window.__AETHERFRONT_SELF_CHECK__=';
  if (!source.includes(marker)) throw new Error('missing Aetherfront self-check hook');
  const instrumentedPath = path.join(profile, 'instrumented.html');
  fs.writeFileSync(instrumentedPath, source.replace(marker,
    'window.__FORGE_TEST_STATE__=()=>S;window.__FORGE_TEST_MOVE__=(unit,x,y)=>commandMove([unit],x,y);' + marker));
  const client = await connect();
  const { Page, Runtime, Log } = client;
  const errors = [];
  Runtime.exceptionThrown(event => {
    const detail = event.exceptionDetails;
    errors.push(`exception: ${detail.text}: ${detail.exception?.description || ''}`);
  });
  Log.entryAdded(({ entry }) => {
    if (entry.level === 'error') errors.push(`console: ${entry.text}`);
  });
  await Promise.all([Page.enable(), Runtime.enable(), Log.enable()]);

  const evaluate = async expression => {
    const response = await Runtime.evaluate({
      expression,
      awaitPromise: true,
      returnByValue: true,
      userGesture: true,
    });
    if (response.exceptionDetails) {
      throw new Error(response.exceptionDetails.exception?.description || response.exceptionDetails.text);
    }
    return response.result.value;
  };

  const loaded = new Promise(resolve => Page.loadEventFired(resolve));
  await Page.navigate({ url: pathToFileURL(instrumentedPath).href });
  await loaded;
  await sleep(700);

  const before = await evaluate(`({
    readyState: document.readyState,
    title: document.title,
    selfCheck: window.__AETHERFRONT_SELF_CHECK__(),
    titleVisible: document.getElementById('titleScreen').classList.contains('active'),
    startLabel: document.getElementById('startBtn').textContent.trim()
  })`);

  const tutorialRoundTrip = await evaluate(`(() => {
    document.getElementById('tutorialBtn').click();
    const opened = document.getElementById('tutorial').classList.contains('active');
    document.getElementById('closeTutorial').click();
    return opened && !document.getElementById('tutorial').classList.contains('active');
  })()`);

  await evaluate(`(() => {
    document.getElementById('difficulty').value = 'easy';
    document.getElementById('startBtn').click();
    return true;
  })()`);
  await sleep(3500);

  const running = await evaluate(`(() => {
    const state = window.__FORGE_TEST_STATE__();
    const player = state.units.find(unit => unit.team === 0);
    state.selected = player ? [player.id] : [];
    if (player) window.__FORGE_TEST_MOVE__(player, player.x + 80, player.y + 40);
    const timeBeforePause = state.time;
    document.getElementById('menuBtn').click();
    const paused = state.mode === 'paused'
      && document.getElementById('pause').classList.contains('active');
    document.getElementById('resumeBtn').click();
    state.cam.x = 0;
    state.cam.y = 700;
    document.getElementById('game').dispatchEvent(new PointerEvent('pointermove', {
      clientX: innerWidth / 2,
      clientY: innerHeight / 2,
      bubbles: true
    }));
    return {
      running: state.mode === 'play',
      ended: state.mode === 'ended',
      sim: state.time,
      simBeforePause: timeBeforePause,
      units: state.units.length,
      playerUnits: state.units.filter(unit => unit.team === 0).length,
      enemyUnits: state.units.filter(unit => unit.team === 1).length,
      buildings: state.buildings.length,
      fields: state.crystals.length,
      controlPoints: state.nodes.length,
      pausedRoundTrip: paused && state.mode === 'play',
      selected: state.selected.length,
      titleHidden: !document.getElementById('titleScreen').classList.contains('active'),
      hudVisible: document.getElementById('gameScreen').classList.contains('active'),
      canvas: [document.getElementById('game').width, document.getElementById('game').height],
      resources: state.credits.slice(),
    };
  })()`);
  await sleep(1500);
  const after = await evaluate(`(() => {
    const state = window.__FORGE_TEST_STATE__();
    return { sim: state.time, running: state.mode === 'play', ended: state.mode === 'ended' };
  })()`);

  const shot = await Page.captureScreenshot({ format: 'png', fromSurface: true });
  fs.writeFileSync(screenshotPath, Buffer.from(shot.data, 'base64'));

  await client.close();
  const result = { before, tutorialRoundTrip, running, after, errors, screenshotPath };
  const valid = before.readyState === 'complete'
    && before.selfCheck.ok
    && before.titleVisible
    && tutorialRoundTrip
    && running.running
    && !running.ended
    && running.sim > 0
    && after.sim > running.sim
    && running.units >= 4
    && running.playerUnits > 0
    && running.enemyUnits > 0
    && running.buildings >= 4
    && running.fields >= 3
    && running.controlPoints === 3
    && running.pausedRoundTrip
    && running.selected === 1
    && running.titleHidden
    && running.hudVisible
    && errors.length === 0;
  console.log(JSON.stringify({ valid, ...result }, null, 2));
  if (!valid) process.exitCode = 1;
}

main()
  .catch(error => {
    console.error(error.stack || String(error));
    if (chromeStderr) console.error(chromeStderr);
    process.exitCode = 1;
  })
  .finally(async () => {
    chrome.kill('SIGTERM');
    await sleep(200);
    if (!chrome.killed) chrome.kill('SIGKILL');
    fs.rmSync(profile, { recursive: true, force: true });
  });
