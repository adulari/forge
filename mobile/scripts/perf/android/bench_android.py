#!/usr/bin/env python3
"""Android streaming benchmark for the Forge app (release APK, emulator or device).

Needs adb on PATH (ANDROID_SERIAL set when several devices are attached), the app installed and
paired with fixture_server.py, which must already be running on this host. Per run: cold start,
session open, scroll the 1,000-message transcript, then stream a 600-token reply at ~50 tok/s and
record process CPU (per thread), gfxinfo frame percentiles and PSS. Compare two APKs by
interleaving runs (install A, run, install B, run, ...): emulator CPU shifts with host load, so a
single run per APK, back to back, is not a comparison.

  bench_android.py --runs 3 --cold 5 --out run.json
"""
import argparse, json, re, statistics, subprocess, time, urllib.request, os
ap = argparse.ArgumentParser()
ap.add_argument("--runs", type=int, default=3)
ap.add_argument("--tokens", type=int, default=600, help="reply length streamed per run")
ap.add_argument("--cold", type=int, default=5)
ap.add_argument("--out", required=True)
ap.add_argument("--session", default="0dc07717-ba48-5bae-8df1-e50d6f303426")
ap.add_argument("--token", default="a1b2c3d4e5f60718293a4b5c6d7e8f90")
ap.add_argument("--no-cold", action="store_true")
a = ap.parse_args()
PKG = "dev.adulari.forge"
def sh(c): return subprocess.run(["adb", "shell", c], capture_output=True, text=True).stdout
def pid(): 
    o = sh(f"pidof {PKG}").strip(); return o.split()[0] if o else None
def launch():
    out = sh(f"am start -W -a android.intent.action.VIEW -d forge://session/{a.session} {PKG}")
    m = re.search(r"TotalTime: (\d+)", out); return int(m.group(1)) if m else None
def meminfo():
    out = sh(f"dumpsys meminfo {PKG}")
    t = re.search(r"TOTAL PSS:\s+(\d+)", out)
    def row(n):
        m = re.search(rf"^\s*{n}:?\s+(\d+)", out, re.M); return int(m.group(1)) if m else None
    return {"pss_kb": int(t.group(1)) if t else None, "java_kb": row("Java Heap"), "native_kb": row("Native Heap"), "graphics_kb": row("Graphics"), "code_kb": row("Code")}
def gfx():
    out = sh(f"dumpsys gfxinfo {PKG}")
    g = lambda p: (lambda m: float(m.group(1)) if m else None)(re.search(p, out))
    return {k: g(p) for k, p in dict(frames=r"Total frames rendered: (\d+)", janky_pct=r"Janky frames: \d+ \(([\d.]+)%\)", p50=r"50th percentile: (\d+)ms", p90=r"90th percentile: (\d+)ms", p95=r"95th percentile: (\d+)ms", p99=r"99th percentile: (\d+)ms", slow_ui=r"Number Slow UI thread: (\d+)", slow_draw=r"Number Slow issue draw commands: (\d+)").items()}
def threads(p):
    cmd = f"for t in /proc/{p}/task/*; do echo \"${{t##*/}} $(cat $t/comm | tr ' ' _) $(cut -d')' -f2- $t/stat | awk '{{print $12+$13}}')\"; done"
    d = {}
    for line in sh(cmd).splitlines():
        parts = line.split()
        if len(parts) == 3: d[f"{parts[1]}:{parts[0]}"] = int(parts[2])
    return d
def total(p):
    f = sh(f"cat /proc/{p}/stat").split(")")[-1].split(); return int(f[11]) + int(f[12])
def stream(n):
    urllib.request.urlopen(f"http://127.0.0.1:7431/{a.token}/bench/stream?session={a.session}&tokens={n}").read()
def swipes():
    sh("for i in $(seq 40); do input swipe 540 700 540 1900 140; sleep 0.25; done; for i in $(seq 10); do input swipe 540 1900 540 700 140; sleep 0.25; done")
def stop(): sh(f"am force-stop {PKG}")
res = {"cold": [], "runs": []}
sh("input keyevent KEYCODE_WAKEUP; svc power stayon true")
sh(f"cmd package compile -m speed -f {PKG}")
if not a.no_cold:
    for _ in range(a.cold):
        stop(); sh("input keyevent KEYCODE_HOME"); time.sleep(2.5); res["cold"].append(launch()); time.sleep(1)
    res["cold_median"] = statistics.median(res["cold"])
for r in range(a.runs):
    stop(); time.sleep(2); launch(); time.sleep(10)
    run = {"pss_session_open": meminfo()}
    sh(f"dumpsys gfxinfo {PKG} reset"); swipes(); time.sleep(1)
    run["scroll_gfx"] = gfx(); run["pss_after_scroll"] = meminfo()
    # return to newest end
    sh("for i in $(seq 12); do input swipe 540 1900 540 700 100; sleep 0.2; done"); time.sleep(1)
    p = pid(); t0 = total(p); time.sleep(8); t1 = total(p); run["idle_cpu_pct"] = (t1 - t0) / 8.0
    sh(f"dumpsys gfxinfo {PKG} reset")
    th0 = threads(p); c0 = total(p); pss0 = meminfo()["pss_kb"]
    stream(a.tokens); time.sleep(a.tokens / 50 + 2.5)
    c1 = total(p); th1 = threads(p)
    run["stream_cpu_pct"] = (c1 - c0) / (a.tokens / 50 + 2.5)
    run["stream_gfx"] = gfx()
    run["threads_pct"] = {k: round((th1[k] - th0.get(k, 0)) / (a.tokens / 50 + 2.5), 1) for k in th1 if th1[k] - th0.get(k, 0) > 0}
    run["pss_stream_delta_kb"] = meminfo()["pss_kb"] - pss0
    res["runs"].append(run); time.sleep(3)
def med(f): 
    xs = [f(r) for r in res["runs"]]; return statistics.median(xs)
res["summary"] = {"stream_cpu_pct": med(lambda r: r["stream_cpu_pct"]), "stream_p99": med(lambda r: r["stream_gfx"]["p99"]), "stream_p90": med(lambda r: r["stream_gfx"]["p90"]), "stream_janky": med(lambda r: r["stream_gfx"]["janky_pct"]), "scroll_p99": med(lambda r: r["scroll_gfx"]["p99"]), "pss_open_mb": med(lambda r: r["pss_session_open"]["pss_kb"]) / 1024, "pss_scroll_mb": med(lambda r: r["pss_after_scroll"]["pss_kb"]) / 1024, "pss_stream_delta_mb": med(lambda r: r["pss_stream_delta_kb"]) / 1024, "idle_cpu": med(lambda r: r["idle_cpu_pct"])}
if "cold_median" in res: res["summary"]["cold_ms"] = res["cold_median"]
json.dump(res, open(a.out, "w"), indent=1)
print(json.dumps(res["summary"], indent=1)); print(json.dumps([r["threads_pct"] for r in res["runs"]]))
