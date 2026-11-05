#!/usr/bin/env python3
"""Baseline (Tauri + RN-web) measurements, off-screen on Xvfb :87, real XTest input."""
import json, os, subprocess, sys, time, signal
sys.path.insert(0, os.path.expanduser("~/.cache/forge-spike-gpui"))
sys.path.insert(0, "/home/floris/Documents/Repositories/Personal/AI/forge-wt/spike-gpui/mobile/scripts/perf")
import mx, xin, bl
from inspector import Inspector
H = os.path.expanduser("~/.cache/forge-spike-gpui")
INST = f"{H}/inst/base"
BIN = os.path.expanduser("~/.cache/forge-target-spike-gpui-base/release/forge-desktop")
PORT = 9361
IDS = json.load(open(f"{H}/ids.json"))

def find_app():
    marker = f"XDG_DATA_HOME={INST}/data"
    for e in os.listdir("/proc"):
        if e.isdigit():
            try:
                env = open(f"/proc/{e}/environ").read().split("\0"); exe = os.path.basename(os.readlink(f"/proc/{e}/exe"))
            except OSError: continue
            if marker in env and exe == "forge-desktop": return int(e)
    return None

def stop_app():
    # kill the dbus-run-session wrapper tree we started (matched by our private XDG_RUNTIME_DIR), then the app by exact pid
    pid = find_app()
    for e in os.listdir("/proc"):
        if e.isdigit():
            try:
                env = open(f"/proc/{e}/environ").read().split("\0")
                if f"XDG_RUNTIME_DIR={INST}/run" in env: os.kill(int(e), signal.SIGTERM)
            except (OSError, ProcessLookupError): pass
    time.sleep(1.5)

def launch():
    os.makedirs(f"{INST}/run", exist_ok=True)
    os.chmod(f"{INST}/run", 0o700)
    env = {k: v for k, v in os.environ.items() if k not in ("WAYLAND_DISPLAY", "HYPRLAND_INSTANCE_SIGNATURE", "DBUS_SESSION_BUS_ADDRESS")}
    env.update(DISPLAY=":87", GDK_BACKEND="x11", WEBKIT_DISABLE_DMABUF_RENDERER="1", FORGE_NO_REPORTS="1", XDG_CONFIG_HOME=f"{INST}/config",
               XDG_DATA_HOME=f"{INST}/data", XDG_CACHE_HOME=f"{INST}/cache", XDG_RUNTIME_DIR=f"{INST}/run",
               WEBKIT_INSPECTOR_HTTP_SERVER=f"127.0.0.1:{PORT}", FORGE_DESKTOP_AUTO_UPDATE="0", FORGE_DESKTOP_OTA="0")
    t0 = time.perf_counter()
    subprocess.Popen(["dbus-run-session", "--", BIN], env=env, stdout=open(f"{INST}/app.log", "w"), stderr=subprocess.STDOUT, start_new_session=True)
    return t0

def wait_app():
    for _ in range(200):
        p = find_app()
        if p: return p
        time.sleep(0.05)
    raise SystemExit("app did not start")

def resize(w=1440, h=900):
    subprocess.run(["python3", os.path.expanduser("~/.cache/forge-apptest/xresize.py"), str(w), str(h)], capture_output=True)

REGION = (20, 150, 220, 170)
def cold_start():
    stop_app()
    xin.move(5, 5)
    t0 = launch()
    # window comes up at default 1180x800; we only probe a region that exists in both sizes
    base = None; deadline = t0 + 30; first_pixels = None
    while time.perf_counter() < deadline:
        b = mx.grab(*REGION)
        if first_pixels is None and any(b[i] for i in range(0, len(b), 97)): first_pixels = time.perf_counter() - t0
        if mx.bright(b) >= 40: return (time.perf_counter() - t0) * 1000, (first_pixels or 0) * 1000
        time.sleep(0.004)
    return None, None

def js(i, expr): return i.eval(expr)

def row_rect(i, text):
    r = i.eval("""(function(t){var e=[...document.querySelectorAll('*')].filter(e=>(e.textContent||'').startsWith(t)&&e.children.length<=1).pop();if(!e)return null;var r=e.getBoundingClientRect();return JSON.stringify([r.x+r.width/2,r.y+r.height/2])})(%s)""" % json.dumps(text))
    return json.loads(r) if r else None

def scroll_queue_to_end(i):
    i.eval("""(function(){var s=[...document.querySelectorAll('div')].find(e=>e.scrollHeight>e.clientHeight+20&&getComputedStyle(e).overflowY!='visible'&&e.clientWidth<400);if(s)s.scrollTop=s.scrollHeight;return 0})()""")

RAF = """window.__f=[];window.__go=true;(function tick(t){if(window.__go){window.__f.push(t);requestAnimationFrame(tick)}})(performance.now());0"""

def drive_scroll(pid, x, y, notches=120, gap=0.025, down_then_up=True):
    """Wheel burst through XTest; returns CPU seconds consumed by the app tree during the burst."""
    c0 = mx.cpu_s(pid); t0 = time.perf_counter()
    for k in range(notches):
        xin.wheel(x, y, 1, down=True); time.sleep(gap)
    time.sleep(0.4)
    return mx.cpu_s(pid) - c0, time.perf_counter() - t0

def main(which):
    out = {}
    if which in ("all", "cold"):
        runs = []
        for k in range(5):
            m, f = cold_start(); runs.append(round(m, 0) if m else None); time.sleep(2)
        out["cold_start_ms_runs"] = runs
        print("cold", runs)
    pid = find_app() or (stop_app() or None)
    if which == "cold": json.dump(out, open(f"{H}/results/web_cold.json", "w")); return
    # fresh instance for the steady-state scenarios
    stop_app(); launch(); pid = wait_app(); time.sleep(6)
    i = Inspector(PORT)
    bl.fire(i, "localStorage.setItem('forge.ideMode.v1',JSON.stringify({enabled:true,railWidth:260,agentWidth:340,bottomHeight:240,coachSeen:true}));location.href='/';0")
    time.sleep(0.5); i.ws.close()
    time.sleep(7); resize(); time.sleep(2)
    i = Inspector(PORT)
    xin.focus_largest(); time.sleep(0.5)
    out["idle_rss_mib"] = round(mx.rss_mib(pid), 0)
    out["dom_nodes_idle"] = i.eval("document.querySelectorAll('*').length")
    print("idle", out["idle_rss_mib"])
    # ---- open the large diff
    rect = None
    for _ in range(60):
        r = row_rect(i, "RSengine.rs")
        if r and 340 < r[1] < 860 and r[0] > 20: rect = r; break
        xin.wheel(130, 600, 3, down=True); time.sleep(0.25)
    print("engine row", rect)
    pre = mx.grab(290, 180, 780, 660)
    t0 = time.perf_counter()
    xin.click(rect[0], rect[1])
    paint = None
    while time.perf_counter() - t0 < 20:
        b = mx.grab(290, 180, 780, 660)
        if mx.bright(b) >= 400: paint = (time.perf_counter() - t0) * 1000; break
        time.sleep(0.002)
    out["open_diff_click_to_paint_ms"] = round(paint, 0) if paint else None
    time.sleep(3)
    out["rss_after_diff_mib"] = round(mx.rss_mib(pid), 0)
    out["dom_nodes_diff"] = i.eval("document.querySelectorAll('*').length")
    bl.shot(i, f"{H}/results/web_ide_diff_{os.environ.get("TAG","")}.png")
    print("diff", out["open_diff_click_to_paint_ms"], out["rss_after_diff_mib"])
    # ---- scroll the diff
    i.eval(RAF); time.sleep(0.5)
    cpu, wall = drive_scroll(pid, 680, 520, notches=120)
    ts = json.loads(i.eval("window.__go=false;JSON.stringify(window.__f)"))
    # keep only frames in the scroll window (rAF runs always): last `wall` seconds
    end = ts[-1]; ts = [t for t in ts if t >= end - wall * 1000]
    out["scroll_diff"] = mx.frame_stats(ts); out["scroll_diff"]["cpu_s"] = round(cpu, 2); out["scroll_diff"]["wall_s"] = round(wall, 2)
    print("scroll diff", out["scroll_diff"])
    time.sleep(1.0); out["notch_latency_diff_ms"] = mx.notch_latency((290, 150, 780, 700), 680, 520); print("notch diff", out["notch_latency_diff_ms"])
    # ---- chat view: switch + load
    xin.click(1165, 18); time.sleep(1.5)
    t0 = time.perf_counter()
    i.eval("history.pushState({}, '', '/session/%s');window.dispatchEvent(new PopStateEvent('popstate'));0" % IDS["long"])
    for _ in range(300):
        if i.eval("!!document.querySelector('[data-testid=composer-input]')"): break
        time.sleep(0.05)
    out["chat_open_ms"] = round((time.perf_counter() - t0) * 1000)
    time.sleep(4)
    out["rss_after_chat_mib"] = round(mx.rss_mib(pid), 0)
    out["dom_nodes_chat"] = i.eval("document.querySelectorAll('*').length")
    out["transcript_rows_dom"] = i.eval("document.querySelectorAll('[data-testid*=row],[data-testid*=message]').length")
    bl.shot(i, f"{H}/results/web_chat_{os.environ.get("TAG","")}.png")
    print("chat", out["rss_after_chat_mib"], out["dom_nodes_chat"])
    # scroll the transcript (wheel up through history)
    i.eval(RAF); time.sleep(0.5)
    c0 = mx.cpu_s(pid); t0 = time.perf_counter()
    for k in range(120):
        xin.wheel(800, 450, 1, down=False); time.sleep(0.025)
    time.sleep(0.4); wall = time.perf_counter() - t0; cpu = mx.cpu_s(pid) - c0
    ts = json.loads(i.eval("window.__go=false;JSON.stringify(window.__f)")); end = ts[-1]; ts = [t for t in ts if t >= end - wall * 1000]
    out["scroll_chat"] = mx.frame_stats(ts); out["scroll_chat"]["cpu_s"] = round(cpu, 2); out["scroll_chat"]["wall_s"] = round(wall, 2)
    print("scroll chat", out["scroll_chat"])
    time.sleep(1.0); out["notch_latency_chat_ms"] = mx.notch_latency((290, 100, 780, 650), 800, 450, down=False); print("notch chat", out["notch_latency_chat_ms"])
    out["rss_after_chat_scroll_mib"] = round(mx.rss_mib(pid), 0)
    # ---- typing latency in the composer
    import measure_gpui as g
    xin.click(700, 853); time.sleep(0.6)
    lat = g.typing_latency((270, 832, 500, 45))
    out["typing_ms"] = {"n": len(lat), "p50": round(mx.pct(lat, 50), 1), "p95": round(mx.pct(lat, 95), 1), "max": round(max(lat), 1)} if lat else None
    print("typing", out["typing_ms"])
    json.dump(out, open(f"{H}/results/web_{os.environ.get('TAG', which)}.json", "w"), indent=1)

if __name__ == "__main__":
    os.makedirs(f"{H}/results", exist_ok=True)
    main(sys.argv[1] if len(sys.argv) > 1 else "all")
