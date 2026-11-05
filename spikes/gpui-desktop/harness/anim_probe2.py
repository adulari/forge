import sys,os,time,json
sys.path.insert(0,os.path.expanduser("~/.cache/forge-spike-gpui"))
import measure_gpui as g, mx
env={"VK_ICD_FILENAMES":os.path.expanduser("~/.cache/forge-spike-gpui/lvp/lvp_icd.json"),"VK_DRIVER_FILES":os.path.expanduser("~/.cache/forge-spike-gpui/lvp/lvp_icd.json"),"SPIKE_ANIM":"1"}
try:
    g.stop(); g.launch(view=sys.argv[1], extra=env); pid=g.proc.pid; time.sleep(8)
    print(open(g.SD+"/app.log").read().strip().splitlines()[-1])
    g.dump(reset=True); c0=mx.cpu_s(pid); time.sleep(5); c1=mx.cpu_s(pid); d=g.dump()
    st=mx.frame_stats(d["frames"]); st["cpu_s_per_s"]=round((c1-c0)/5,2); print(sys.argv[1], st, "rss", round(mx.rss_mib(pid)))
finally:
    g.stop()
