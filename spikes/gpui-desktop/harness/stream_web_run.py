import sys,os,time
sys.path.insert(0,os.path.expanduser("~/.cache/forge-spike-gpui"))
import measure_web as m
m.stop_app(); m.launch(); m.wait_app(); time.sleep(7); m.resize(); time.sleep(2)
