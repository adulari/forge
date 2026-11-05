import sys,os,time
sys.path.insert(0,os.path.expanduser("~/.cache/forge-spike-gpui"))
import measure_web as m, bl
m.stop_app(); m.launch(); m.wait_app(); time.sleep(6)
i=m.Inspector(m.PORT)
bl.fire(i, "localStorage.setItem('forge.ideMode.v1',JSON.stringify({enabled:true,railWidth:260,agentWidth:340,bottomHeight:240,coachSeen:true}));localStorage.removeItem('forge.queryCache');location.href='/';0")
time.sleep(0.5); i.ws.close(); time.sleep(6); m.stop_app()
