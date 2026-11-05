import sys,os,time,json
sys.path.insert(0,os.path.expanduser("~/.cache/forge-spike-gpui"))
import measure_gpui as g, mx, xin
TAG=os.environ.get("TAG","gpui")
g.stop(); g.launch(view="chat", extra={"SPIKE_STREAM":"50"}); pid=g.proc.pid
time.sleep(1.5); xin.focus_largest(); time.sleep(1.8)
t0=time.perf_counter(); c_start=c_end=None; w_start=w_end=None
while True:
    time.sleep(0.2); d=g.dump(); names={e[0] for e in d["events"]}
    if "stream_start" in names and c_start is None: c_start=mx.cpu_s(pid); w_start=time.perf_counter()
    if "stream_end" in names: c_end=mx.cpu_s(pid); w_end=time.perf_counter(); break
    if time.perf_counter()-t0>40: break
c0=c_start; wall=w_end-w_start
ev={e[0]:e[1] for e in d["events"] if not e[0].startswith("tick")}
ts=[f for f in d["frames"] if ev.get("stream_start",0)<=f<=ev.get("stream_end",1e18)]
ticks=[float(e[0].split("=")[1]) for e in d["events"] if e[0].startswith("tick_ms=")]
st=mx.frame_stats(ts); st.update(cpu_s=round(c_end-c_start,2), wall_s=round(wall,2), tick_ms_p50=round(mx.pct(ticks,50),3), tick_ms_p95=round(mx.pct(ticks,95),3), tick_ms_max=round(max(ticks),3), ticks=len(ticks), stream_s=round((ev.get("stream_end",0)-ev.get("stream_start",0))/1000,2), rss_mib=round(mx.rss_mib(pid)))
import subprocess; subprocess.run(f"DISPLAY=:87 import -window root -crop 1440x900+0+0 {g.H}/results/{TAG}_stream.png",shell=True)
print(json.dumps(st)); json.dump(st,open(f"{g.H}/results/{TAG}_stream.json","w"),indent=1)
g.stop()
