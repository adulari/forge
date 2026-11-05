"""Black-box measurement helpers shared by both apps (private Xvfb :87 only)."""
import ctypes, os, sys, time, json, statistics, subprocess
sys.path.insert(0, os.path.expanduser("~/.cache/forge-spike-gpui"))
import xin
X = xin.X; d = xin.d
class XImage(ctypes.Structure):
    _fields_ = [("width", ctypes.c_int), ("height", ctypes.c_int), ("xoffset", ctypes.c_int), ("format", ctypes.c_int),
                ("data", ctypes.c_void_p), ("byte_order", ctypes.c_int), ("bitmap_unit", ctypes.c_int), ("bitmap_bit_order", ctypes.c_int),
                ("bitmap_pad", ctypes.c_int), ("depth", ctypes.c_int), ("bytes_per_line", ctypes.c_int), ("bits_per_pixel", ctypes.c_int)]
X.XGetImage.restype = ctypes.POINTER(XImage)
X.XGetImage.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_int]
X.XDefaultRootWindow.restype = ctypes.c_ulong
X.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
ROOT = X.XDefaultRootWindow(d)
def grab(x, y, w, h):
    img = X.XGetImage(d, ROOT, x, y, w, h, 0xFFFFFFFF, 2)
    n = img.contents.bytes_per_line * h
    b = ctypes.string_at(img.contents.data, n)
    X.XFree(ctypes.c_void_p(img.contents.data)); X.XFree(img)
    return b
def bright(b, thresh=120):
    # BGRX pixels; count pixels whose min channel > thresh
    mv = memoryview(b); n = 0
    for i in range(0, len(b) - 3, 4):
        if mv[i] > thresh and mv[i + 1] > thresh and mv[i + 2] > thresh: n += 1
    return n
def pstat(pid):
    try:
        f = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
        return int(f[11]) + int(f[12]), int(f[1])  # utime+stime ticks, ppid
    except OSError: return 0, 0
def children(pid):
    out = []
    for e in os.listdir("/proc"):
        if e.isdigit():
            t, pp = pstat(int(e))
            if pp == pid: out.append(int(e))
    return out
def tree(pid):
    r = [pid]
    for c in children(pid): r += tree(c)
    return r
def rss_mib(pid):
    s = 0
    for p in tree(pid):
        try:
            for l in open(f"/proc/{p}/status"):
                if l.startswith("VmRSS:"): s += int(l.split()[1])
        except OSError: pass
    return s / 1024
def cpu_s(pid):
    return sum(pstat(p)[0] for p in tree(pid)) / os.sysconf("SC_CLK_TCK")
def pct(v, p):
    v = sorted(v); return v[min(len(v) - 1, int(round(p / 100 * (len(v) - 1))))]
def frame_stats(ts, budget=1000 / 60):
    gaps = [b - a for a, b in zip(ts, ts[1:])]
    if not gaps: return None
    return {"frames": len(ts), "p50": round(pct(gaps, 50), 2), "p95": round(pct(gaps, 95), 2), "p99": round(pct(gaps, 99), 2), "max": round(max(gaps), 2),
            "dropped_gt_1.5x": sum(1 for g in gaps if g > 1.5 * budget), "gaps_gt_50ms": sum(1 for g in gaps if g > 50)}

import zlib
def notch_latency(region, x, y, n=25, down=True):
    lat = []
    for _ in range(n):
        h0 = zlib.crc32(grab(*region)); t0 = time.perf_counter(); xin.wheel(x, y, 1, down=down); ok = None
        while time.perf_counter() - t0 < 1.0:
            if zlib.crc32(grab(*region)) != h0: ok = (time.perf_counter() - t0) * 1000; break
            time.sleep(0.0005)
        if ok is not None: lat.append(ok)
        time.sleep(0.4)
    return {"n": len(lat), "p50": round(pct(lat, 50), 1), "p95": round(pct(lat, 95), 1), "max": round(max(lat), 1)} if lat else None
