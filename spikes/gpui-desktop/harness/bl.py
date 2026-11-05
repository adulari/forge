import sys,time,json
sys.path.insert(0,"/home/floris/Documents/Repositories/Personal/AI/forge-wt/spike-gpui/mobile/scripts/perf")
from inspector import Inspector
PORT=9361
def ins(): return Inspector(PORT)
def click_text(i,text,tag_max_children=6,exact=True):
    js="""(function(t){var els=[...document.querySelectorAll('div,a,span,button')].filter(e=>e.children.length<%d&&(e.innerText||'').trim()===t);
    var e=els.pop(); if(!e) return 'none'; ['mousedown','mouseup','click'].forEach(k=>e.dispatchEvent(new MouseEvent(k,{bubbles:true}))); return 'ok'})(%s)"""%(tag_max_children,json.dumps(text))
    return i.eval(js)
def shot(i,out):
    r=i.send("Page.snapshotRect",{"x":0,"y":0,"width":int(i.eval("innerWidth")),"height":int(i.eval("innerHeight")),"coordinateSystem":"Viewport"})
    import base64; open(out,"wb").write(base64.b64decode(r["result"]["dataURL"].split(",",1)[1]))
def fire(i,expr):
    i.mid+=1
    inner={"id":i.mid,"method":"Runtime.evaluate","params":{"expression":expr}}
    i.ws.send(json.dumps({"id":i.mid+100000,"method":"Target.sendMessageToTarget","params":{"targetId":i.target,"message":json.dumps(inner)}}))
