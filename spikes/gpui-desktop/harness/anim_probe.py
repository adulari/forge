import sys,os,time,json
sys.path.insert(0,os.path.expanduser("~/.cache/forge-spike-gpui"))
import measure_gpui as g, mx
view=sys.argv[1]
try:
    g.stop(); g.launch(view=view, extra={"SPIKE_ANIM":"1"}); pid=g.proc.pid; time.sleep(6)
    g.dump(reset=True); c0=mx.cpu_s(pid); time.sleep(5); c1=mx.cpu_s(pid); d=g.dump()
    st=mx.frame_stats(d["frames"]); st["cpu_s_per_s"]=round((c1-c0)/5,2); print(view, st)
finally:
    g.stop()
