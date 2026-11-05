import sqlite3, time, uuid, json, os, random
random.seed(42)
H=os.path.expanduser("~/.cache/forge-spike-gpui")
ids=json.load(open(H+"/ids.json"))
db=sqlite3.connect(H+"/daemon/data/forge/forge.db")
R=H+"/fixture/vectra"; now=int(time.time())
seq=0
def add(sid,role,content,calls=None,cid=None,t=None):
    global seq; seq+=1
    db.execute("insert into message (id,session_id,seq,role,content,model,tool_calls_json,tool_call_id,visibility,created_at) values (?,?,?,?,?,?,?,?,?,?)",
      (str(uuid.uuid4()),sid,seq,role,content,"mock" if role=="assistant" else None,json.dumps(calls) if calls else None,cid,"llm",t or now-5000+seq))
# attribution for the agent trees
for key,p in (("tests","crates/core/src/mod1.rs"),("shared","crates/core/src/engine.rs")):
    sid=ids[key]; seq=db.execute('select coalesce(max(seq),0) from message where session_id=?',(sid,)).fetchone()[0]
    add(sid,"user","Please work on this and keep the diff small.")
    add(sid,"assistant","",[{"id":"c1","name":"read_file","args":{"path":p}},{"id":"c2","name":"write_file","args":{"path":f"{R}/{p}","content":"x"}},{"id":"c3","name":"shell","args":{"command":"cargo test -p core"}}])
    for c in ("c1","c2","c3"): add(sid,"tool","ok",None,c)
    add(sid,"assistant","Done. I updated the file and ran the tests.")
# the 1,000-message transcript
CODE={"rust":["fn match_orders(book: &mut Book, incoming: Order) -> Vec<Fill> {\n    let mut fills = Vec::new();\n    while let Some(best) = book.best_opposite(&incoming) {\n        if !incoming.crosses(best.price) { break; }\n        let qty = incoming.remaining().min(best.remaining());\n        fills.push(Fill::new(best.id, incoming.id, best.price, qty));\n        book.reduce(best.id, qty);\n    }\n    fills\n}",
"impl Book {\n    pub fn best_opposite(&self, o: &Order) -> Option<&Order> {\n        match o.side {\n            Side::Buy => self.asks.first(),\n            Side::Sell => self.bids.first(),\n        }\n    }\n}"],
"typescript":["export async function loadBook(client: Client, symbol: string): Promise<Book> {\n  const res = await client.get(`/book/${symbol}`);\n  if (!res.ok) throw new Error(`book ${symbol}: ${res.status}`);\n  return normalise(await res.json());\n}",
"const queue = new Map<string, Order[]>();\nfor (const o of orders) {\n  const list = queue.get(o.symbol) ?? [];\n  list.push(o);\n  queue.set(o.symbol, list);\n}"],
"json":['{\n  "symbol": "ETH-USD",\n  "bids": [[3120.5, 4.2], [3120.0, 1.1]],\n  "asks": [[3121.0, 2.7]],\n  "seq": 99142\n}'],
"bash":["cargo test -p core --release -- --nocapture\ncargo clippy --workspace -- -D warnings"]}
TOP=["matching engine","retry policy","book normalisation","latency budget","risk limits","tick size handling","snapshot recovery","fee schedule"]
def para(i):
    t=TOP[i%len(TOP)]
    return random.choice([f"The {t} path keeps the hot loop allocation-free; the only heap use is the fill vector, which we reserve up front.",
      f"I checked how **{t}** behaves under load and the p99 stays below `2ms` as long as the book depth is under 10k levels.",
      f"For {t}, the safest option is to keep the invariant in one place and assert it in `debug_assertions` builds only.",
      f"Short answer: yes. Longer answer: {t} interacts with the sequencing guarantees, so we should not reorder those writes."])
def assistant(i):
    parts=[]
    if i%9==0: parts.append(f"## Notes on {TOP[i%len(TOP)]}")
    parts.append(para(i))
    if i%4==0:
        parts.append("\n".join(f"- {para(i+k)}" for k in range(3)))
    if i%3==0:
        lang=random.choice(list(CODE)); parts.append(f"```{lang}\n{random.choice(CODE[lang])}\n```")
    if i%7==0: parts.append(para(i+1)+" See the **design doc** and the linked `RFC-12` for the full trade-off table.")
    if i%11==0: parts.append("1. Reserve the vector\n2. Match against the best opposite level\n3. Emit fills in price-time order")
    return "\n\n".join(parts)
def user(i): return random.choice([f"What about {TOP[i%len(TOP)]}?",f"Can you check the {TOP[i%len(TOP)]} once more and tell me if the p99 holds?","ok, and then?",f"Please explain how {TOP[i%len(TOP)]} works with the sequencer, with an example."])
sid=ids["long"]; seq=0
import sys
db.execute('pragma busy_timeout=5000')
for i in range(500):
    add(sid,"user",user(i)); add(sid,"assistant",assistant(i))
db.commit(); print("seeded", seq)
