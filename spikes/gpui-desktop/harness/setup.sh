#!/bin/bash
# Reset the isolated mock daemon (port 7611) and seed it: 4 agent sessions + 1,000-message transcript
H=$HOME/.cache/forge-spike-gpui; cd $H
stop() { [ -f daemon/config/forge/serve-state.json ] && P=$(python3 -c "import json;print(json.load(open('daemon/config/forge/serve-state.json'))['pid'])") && kill $P 2>/dev/null; sleep 3; }
stop
rm -rf daemon/data daemon/config/forge/serve-state.json daemon/config/forge/serve.lock daemon/config/forge/serve-token; mkdir -p daemon/data/forge
./daemon.sh; sleep 4
python3 - <<'P'
import json, urllib.request, os
D=os.path.expanduser("~/.cache/forge-spike-gpui/daemon")
T=open(D+"/config/forge/serve-token").read().strip(); B=f"http://127.0.0.1:7611/{T}"
R=os.path.expanduser("~/.cache/forge-spike-gpui/fixture/vectra")
def post(path, body):
    r=urllib.request.Request(B+path, data=json.dumps(body).encode(), headers={"content-type":"application/json"}); return json.load(urllib.request.urlopen(r))
ids={}
for key,title,cwd in (("fix","Fix retry logic in the API client",f"{R}/.forge/worktrees/api-fix"),("tests","Add unit tests for the normalisers",f"{R}/.forge/worktrees/tests"),("docs","Tidy the docs",f"{R}/.forge/worktrees/docs"),("shared","Tune the risk settings",R),("long","Design review: order-book engine",R)):
    ids[key]=post("/api/sessions",{"cwd":cwd,"title":title,"permission_mode":"default"})["id"]
json.dump(ids,open(os.path.expanduser("~/.cache/forge-spike-gpui/ids.json"),"w")); print(ids)
import sqlite3
db=sqlite3.connect(D+"/data/forge/forge.db"); db.execute("pragma busy_timeout=5000")
for k in ("fix","tests","docs"):
    cwd=db.execute("select cwd from session where id=?",(ids[k],)).fetchone()[0]
    db.execute("update session set worktree_path=? where id=?",(cwd,ids[k]))
db.commit()
P
python3 seed.py
stop; ./daemon.sh; sleep 4
python3 - <<'P'
import json,os,asyncio,websockets
H=os.path.expanduser("~/.cache/forge-spike-gpui")
T=open(H+"/daemon/config/forge/serve-token").read().strip(); ids=json.load(open(H+"/ids.json"))
async def need(sid):
    async with websockets.connect(f"ws://127.0.0.1:7611/{T}/ws?session={sid}&rev=0", max_size=None) as ws:
        await ws.send(json.dumps({"kind":"prompt","text":"mock:write please"}))
        for _ in range(60):
            m=json.loads(await asyncio.wait_for(ws.recv(),15))
            if m.get("permission_prompt"): print("waiting", m["permission_prompt"][:60]); await asyncio.sleep(1); return
asyncio.run(need(ids["fix"]))
P
