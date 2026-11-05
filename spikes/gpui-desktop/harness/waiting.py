import json,os,asyncio,websockets,sys
H=os.path.expanduser("~/.cache/forge-spike-gpui")
T=open(H+"/daemon/config/forge/serve-token").read().strip(); ids=json.load(open(H+"/ids.json"))
async def need(sid):
    ws=await websockets.connect(f"ws://127.0.0.1:7611/{T}/ws?session={sid}&rev=0", max_size=None)
    await ws.send(json.dumps({"kind":"prompt","text":"mock:write again"}))
    for _ in range(60):
        m=json.loads(await asyncio.wait_for(ws.recv(),15))
        if m.get("permission_prompt"): print("waiting", m["permission_prompt"][:60]); await asyncio.sleep(1); return ws
asyncio.run(need(ids[sys.argv[1]]))
