#!/usr/bin/env python3
"""GPUI spike measurements, off-screen on Xvfb :87, real XTest input, same probes as measure_web.py."""
import json, os, subprocess, sys, time, signal
sys.path.insert(0, os.path.expanduser("~/.cache/forge-spike-gpui"))
import mx, xin
H = os.path.expanduser("~/.cache/forge-spike-gpui")
BIN = os.environ.get("BIN", os.path.expanduser("~/.cache/forge-target-spike-gpui/release/forge-gpui-spike"))
TAG = os.environ.get("TAG", "gpui")
SD = f"{H}/spikedir"
IDS = json.load(open(f"{H}/ids.json"))
proc = None

def launch(view="ide", extra=None, key="long"):
    global proc
    subprocess.run(f"rm -rf {SD}; mkdir -p {SD}", shell=True)
    env = {k: v for k, v in os.environ.items() if k not in ("WAYLAND_DISPLAY", "HYPRLAND_INSTANCE_SIGNATURE")}
    env.update(DISPLAY=":87", XDG_CONFIG_HOME=f"{H}/daemon/config", SPIKE_DIR=SD, SPIKE_VIEW=view, SPIKE_CHAT_SESSION=IDS[key])
    env.update(extra or {})
    t0 = time.perf_counter()
    proc = subprocess.Popen([BIN], env=env, stdout=open(f"{SD}/app.log", "w"), stderr=subprocess.STDOUT, start_new_session=True)
    return t0

def stop():
    global proc
    if proc and proc.poll() is None:
        proc.terminate()
        try: proc.wait(5)
        except subprocess.TimeoutExpired: proc.kill()
    proc = None
    time.sleep(0.5)

def dump(reset=False):
    if reset: open(f"{SD}/reset", "w").close(); time.sleep(0.2); return
    try: os.remove(f"{SD}/dump.json")
    except OSError: pass
    open(f"{SD}/dump", "w").close()
    for _ in range(100):
        if os.path.exists(f"{SD}/dump.json"):
            time.sleep(0.05)
            return json.load(open(f"{SD}/dump.json"))
        time.sleep(0.05)
    raise SystemExit("no dump")

REGION = (20, 150, 220, 170)
def cold_start():
    stop(); xin.move(5, 5)
    t0 = launch(); first = None
    while time.perf_counter() - t0 < 30:
        b = mx.grab(*REGION)
        if first is None and any(b[i] for i in range(0, len(b), 97)): first = time.perf_counter() - t0
        if mx.bright(b) >= 40: return (time.perf_counter() - t0) * 1000, (first or 0) * 1000
        time.sleep(0.004)
    return None, None

def typing_latency(region, n=40, gap=0.15):
    lat = []
    keys = "mwhmwhmwhm"
    b0 = mx.bright(mx.grab(*region), 100)
    for k in range(n):
        ch = keys[k % len(keys)]
        t0 = time.perf_counter(); xin.key(ch)
        ok = None
        while time.perf_counter() - t0 < 1.0:
            c = mx.bright(mx.grab(*region), 100)
            if c >= b0 + 12: ok = (time.perf_counter() - t0) * 1000; b0 = c; break
            time.sleep(0.0005)
        if ok is None: b0 = mx.bright(mx.grab(*region), 100)
        else: lat.append(ok)
        time.sleep(gap)
    return lat

def main():
    out = {}
    runs = []
    if "nocold" not in sys.argv:
        for k in range(5):
            m, f = cold_start(); runs.append(round(m) if m else None); time.sleep(2)
        out["cold_start_ms_runs"] = runs; print("cold", runs)
    stop(); launch(); pid = proc.pid; time.sleep(8)
    xin.focus_largest(); time.sleep(0.5)
    out["idle_rss_mib"] = round(mx.rss_mib(pid), 0); print("idle", out["idle_rss_mib"])
    xin.click(800, 400); time.sleep(0.3)
    t0 = time.perf_counter(); xin.key("o"); paint = None
    while time.perf_counter() - t0 < 20:
        if mx.bright(mx.grab(290, 180, 780, 660)) >= 400: paint = (time.perf_counter() - t0) * 1000; break
        time.sleep(0.002)
    out["open_diff_key_to_paint_ms"] = round(paint) if paint else None
    time.sleep(3)
    d = dump(); out["rss_after_diff_mib"] = round(mx.rss_mib(pid), 0)
    out["diff_events"] = [e for e in d["events"] if e[0].startswith("diff")]
    subprocess.run(f"DISPLAY=:87 import -window root -crop 1440x900+0+0 {H}/results/{TAG}_ide_diff.png", shell=True)
    print("diff", out["open_diff_key_to_paint_ms"], out["rss_after_diff_mib"], out["diff_events"])
    dump(reset=True)
    c0 = mx.cpu_s(pid); t0 = time.perf_counter()
    for k in range(120): xin.wheel(680, 520, 1, down=True); time.sleep(0.025)
    time.sleep(0.4); wall = time.perf_counter() - t0; cpu = mx.cpu_s(pid) - c0
    d = dump(); ts = d["frames"]
    out["scroll_diff"] = mx.frame_stats(ts); out["scroll_diff"].update(cpu_s=round(cpu, 2), wall_s=round(wall, 2)); print("scroll diff", out["scroll_diff"])
    time.sleep(1.0); out["notch_latency_diff_ms"] = mx.notch_latency((290, 150, 780, 700), 680, 520); print("notch diff", out["notch_latency_diff_ms"])
    xin.click(1369, 17); time.sleep(4)
    out["rss_after_chat_mib"] = round(mx.rss_mib(pid), 0)
    subprocess.run(f"DISPLAY=:87 import -window root -crop 1440x900+0+0 {H}/results/{TAG}_chat.png", shell=True)
    dump(reset=True)
    c0 = mx.cpu_s(pid); t0 = time.perf_counter()
    for k in range(120): xin.wheel(800, 450, 1, down=False); time.sleep(0.025)
    time.sleep(0.4); wall = time.perf_counter() - t0; cpu = mx.cpu_s(pid) - c0
    d = dump(); ts = d["frames"]
    out["scroll_chat"] = mx.frame_stats(ts); out["scroll_chat"].update(cpu_s=round(cpu, 2), wall_s=round(wall, 2)); print("scroll chat", out["scroll_chat"])
    time.sleep(1.0); out["notch_latency_chat_ms"] = mx.notch_latency((290, 100, 780, 650), 800, 450, down=False); print("notch chat", out["notch_latency_chat_ms"])
    out["rss_after_chat_scroll_mib"] = round(mx.rss_mib(pid), 0)
    xin.click(700, 872); time.sleep(0.5)
    lat = typing_latency((10, 850, 500, 45))
    out["typing_ms"] = {"n": len(lat), "p50": round(mx.pct(lat, 50), 1), "p95": round(mx.pct(lat, 95), 1), "max": round(max(lat), 1)} if lat else None
    print("typing", out["typing_ms"])
    stop()
    json.dump(out, open(f"{H}/results/{TAG}.json", "w"), indent=1)

if __name__ == "__main__":
    os.makedirs(f"{H}/results", exist_ok=True); main()
