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
  const instrumentedPath = path.join(profile, 'instrumented.html');
  const legacyMarker = 'window.__AETHERFRONT_SELF_CHECK__=';
  const sourceWithLegacyExports = source.includes(legacyMarker)
    ? source.replace(
      legacyMarker,
      'window.__FORGE_NATIVE_STATE__=()=>S;'
        + 'window.__FORGE_NATIVE_MOVE__=(unit,x,y)=>commandMove([unit],x,y);'
        + legacyMarker,
    )
    : source;
  const scopeBridge = `
window.__FORGE_NATIVE_STATE__ ??= () => {
  if (typeof S !== 'undefined') return S;
  if (typeof game !== 'undefined') return game;
  if (typeof G !== 'undefined') return G;
  if (typeof state !== 'undefined' && typeof state !== 'function') return state;
  return null;
};
window.__FORGE_NATIVE_SELECT_MOVE__ ??= (unit, x, y) => {
  const native = window.__FORGE_NATIVE_STATE__?.();
  if (Array.isArray(native?.selection) && typeof setOrder === 'function') {
    for (const selected of native.selection) selected.selected = false;
    native.selection.splice(0, native.selection.length, unit);
    unit.selected = true;
    setOrder([unit], 'move', x, y);
    return true;
  }
  if (Array.isArray(native?.selected) && typeof commandMove === 'function') {
    native.selected.splice(0, native.selected.length, unit.id);
    commandMove([unit], x, y);
    return true;
  }
  return false;
};
`;
  const closingScript = sourceWithLegacyExports.lastIndexOf('</script>');
  const closingIife = closingScript < 0
    ? -1
    : sourceWithLegacyExports.lastIndexOf('})();', closingScript);
  const sourceWithScopeExports = closingIife < 0
    ? sourceWithLegacyExports
    : `${sourceWithLegacyExports.slice(0, closingIife)}${scopeBridge}`
      + sourceWithLegacyExports.slice(closingIife);
  const adapter = `<script>
(() => {
  const value = (name, fallback) => {
    try { return (0, eval)(\`typeof \${name} === 'undefined' ? undefined : \${name}\`) ?? fallback; }
    catch (_) { return fallback; }
  };
  const state = () => {
    const native = window.__FORGE_NATIVE_STATE__?.()
      ?? value('S', null)
      ?? value('game', null)
      ?? value('G', null);
    const currentUnits = native?.units ?? value('units', []);
    const currentBuildings = native?.buildings ?? value('buildings', []);
    const currentFields = native?.crystals ?? value('resources', []);
    const currentNodes = native?.nodes ?? value('nodes', []);
    const currentSelection = native?.selected ?? native?.selection ?? value('selection', []);
    const isPaused = Boolean(
      native?.mode === 'paused' || native?.paused || value('paused', false)
    );
    const isEnded = Boolean(
      native?.mode === 'ended' || native?.ended || value('ended', false)
    );
    const isRunning = Boolean(
      native?.mode === 'play' || native?.running || value('running', false)
    );
    const own = value('player', null);
    const enemy = value('enemy', null);
    return {
      mode: isEnded ? 'ended' : isPaused ? 'paused' : isRunning ? 'play' : (native?.mode ?? 'title'),
      time: native?.time ?? value('simTime', 0),
      units: currentUnits,
      buildings: currentBuildings,
      fields: currentFields,
      nodes: currentNodes,
      selectedCount: currentSelection.length,
      credits: native?.credits
        ?? native?.resources
        ?? [own?.credits ?? 0, enemy?.credits ?? 0],
    };
  };
  const selectMove = (unit, x, y) => {
    const native = window.__FORGE_NATIVE_STATE__?.() ?? value('S', null);
    if (typeof window.__FORGE_NATIVE_SELECT_MOVE__ === 'function'
      && window.__FORGE_NATIVE_SELECT_MOVE__(unit, x, y)) {
      return true;
    }
    if (typeof window.__FORGE_NATIVE_MOVE__ === 'function') {
      native.selected = [unit.id];
      window.__FORGE_NATIVE_MOVE__(unit, x, y);
      return true;
    }
    const commandMoveFn = value('commandMove', null);
    if (native && typeof commandMoveFn === 'function') {
      native.selected = [unit.id];
      commandMoveFn([unit], x, y);
      return true;
    }
    const currentSelection = value('selection', null);
    const issueMoveFn = value('issueMove', null);
    if (Array.isArray(currentSelection) && typeof issueMoveFn === 'function') {
      currentSelection.splice(0, currentSelection.length, unit);
      issueMoveFn(x, y);
      return true;
    }
    const nativeSelection = native?.selection;
    const setOrderFn = value('setOrder', null);
    if (Array.isArray(nativeSelection) && typeof setOrderFn === 'function') {
      for (const selected of nativeSelection) selected.selected = false;
      nativeSelection.splice(0, nativeSelection.length, unit);
      unit.selected = true;
      setOrderFn([unit], 'move', x, y);
      return true;
    }
    return false;
  };
  const selfCheck = () => {
    const check = window.__AETHERFRONT_SELF_CHECK__
      ?? window.AETHERFRONT_SELF_CHECK
      ?? window.Aetherfront?.selfCheck;
    if (typeof check === 'function') return check();
    if (check && typeof check === 'object') return check;
    return { ok: false, errors: ['missing embedded Aetherfront self-check'] };
  };
  window.__FORGE_AETHERFRONT_ADAPTER__ = { state, selectMove, selfCheck };
})();
</script>`;
  const instrumented = sourceWithScopeExports.includes('</body>')
    ? sourceWithScopeExports.replace('</body>', `${adapter}</body>`)
    : `${sourceWithScopeExports}${adapter}`;
  fs.writeFileSync(instrumentedPath, instrumented);
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

  const before = await evaluate(`(() => {
    const visible = element => Boolean(element
      && getComputedStyle(element).display !== 'none'
      && getComputedStyle(element).visibility !== 'hidden');
    const titleScreen = document.getElementById('titleScreen') ?? document.getElementById('title');
    const start = document.getElementById('startBtn');
    return {
      readyState: document.readyState,
      title: document.title,
      selfCheck: window.__FORGE_AETHERFRONT_ADAPTER__.selfCheck(),
      titleVisible: visible(titleScreen),
      startLabel: start?.textContent.trim() ?? ''
    };
  })()`);

  const tutorialRoundTrip = await evaluate(`(() => {
    const visible = element => Boolean(element
      && getComputedStyle(element).display !== 'none'
      && getComputedStyle(element).visibility !== 'hidden');
    const tutorial = document.getElementById('tutorial');
    const open = document.getElementById('tutorialBtn');
    const close = document.getElementById('closeTutorial')
      ?? document.getElementById('tutorialClose');
    if (!tutorial || !open || !close) return false;
    open.click();
    const opened = visible(tutorial);
    close.click();
    return opened && !visible(tutorial);
  })()`);

  await evaluate(`(() => {
    const difficulty = document.getElementById('difficulty');
    if (difficulty) difficulty.selectedIndex = 0;
    document.getElementById('startBtn').click();
    return true;
  })()`);
  await sleep(3500);

  const running = await evaluate(`(() => {
    const adapter = window.__FORGE_AETHERFRONT_ADAPTER__;
    const canvas = ['game', 'world', 'field', 'gameCanvas']
      .map(id => document.getElementById(id))
      .find(element => element instanceof HTMLCanvasElement)
      ?? document.querySelector('canvas');
    let state = adapter.state();
    const player = state.units.find(unit => (unit.team ?? unit.side) === 0);
    if (player) adapter.selectMove(player, player.x + 80, player.y + 40);
    state = adapter.state();
    const timeBeforePause = state.time;
    const pauseButton = document.getElementById('menuBtn') ?? document.getElementById('pauseBtn');
    pauseButton?.click();
    state = adapter.state();
    const pauseScreen = document.getElementById('pause')
      ?? document.getElementById('pauseMenu');
    const pauseVisible = Boolean(pauseScreen
      && getComputedStyle(pauseScreen).display !== 'none'
      && getComputedStyle(pauseScreen).visibility !== 'hidden');
    const paused = state.mode === 'paused' && pauseVisible;
    document.getElementById('resumeBtn').click();
    state = adapter.state();
    canvas?.dispatchEvent(new PointerEvent('pointermove', {
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
      playerUnits: state.units.filter(unit => (unit.team ?? unit.side) === 0).length,
      enemyUnits: state.units.filter(unit => (unit.team ?? unit.side) === 1).length,
      buildings: state.buildings.length,
      fields: state.fields.length,
      controlPoints: state.nodes.length,
      pausedRoundTrip: paused && state.mode === 'play',
      selected: state.selectedCount,
      titleHidden: (() => {
        const title = document.getElementById('titleScreen') ?? document.getElementById('title');
        return !title || getComputedStyle(title).display === 'none';
      })(),
      hudVisible: (() => {
        const hud = document.getElementById('gameScreen') ?? document.getElementById('hud');
        return Boolean(hud && getComputedStyle(hud).display !== 'none');
      })(),
      canvas: [canvas?.width ?? 0, canvas?.height ?? 0],
      resources: state.credits,
    };
  })()`);
  await sleep(1500);
  const after = await evaluate(`(() => {
    const state = window.__FORGE_AETHERFRONT_ADAPTER__.state();
    return { sim: state.time, running: state.mode === 'play', ended: state.mode === 'ended' };
  })()`);

  const shot = await Page.captureScreenshot({ format: 'png', fromSurface: true });
  fs.writeFileSync(screenshotPath, Buffer.from(shot.data, 'base64'));

  await client.close();
  const reportPath = screenshotPath.replace(/\.png$/i, '.verification.json');
  const result = { before, tutorialRoundTrip, running, after, errors, screenshotPath, reportPath };
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
    && running.canvas[0] > 0
    && running.canvas[1] > 0
    && errors.length === 0;
  const report = { valid, ...result };
  fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  console.log(JSON.stringify(report, null, 2));
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
