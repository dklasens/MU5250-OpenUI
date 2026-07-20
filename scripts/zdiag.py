#!/usr/bin/env python3
"""Run a ping/traceroute diag and poll the result, reporting timing.

Usage:
  zdiag.py ping  <target> [action]
  zdiag.py trace <target> [action]

Reads ROUTER_PW env. Non-destructive unless you pass an injection target.
"""
import os, sys, json, time, hashlib, urllib.request, http.cookiejar

GW=os.environ["GW"] if "GW" in os.environ else "192.168.0.1"
PW=os.environ["ROUTER_PW"]
ANON="0"*32
T=lambda:int(time.time()*1000)
_JAR=http.cookiejar.CookieJar()
_OP=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_JAR))
def post(pl):
    req=urllib.request.Request(f"http://{GW}/ubus/?t={T()}",
        data=json.dumps(pl).encode(),headers={"Content-Type":"application/json",
        "Accept":"application/json, text/plain, */*","User-Agent":"Mozilla/5.0 zte-recon",
        "Origin":f"http://{GW}","Referer":f"http://{GW}/","Z-Mode":"1"})
    try:
        with _OP.open(req,timeout=30) as r: return json.loads(r.read().decode(errors="replace"))
    except Exception as e: return [{"_err":f"{type(e).__name__}: {e}"}]
def call(s,o,m,p=None): return post([{"jsonrpc":"2.0","id":1,"method":"call","params":[s,o,m,p or {}]}])[0]
def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()
def login():
    salt=call(ANON,"zwrt_web","web_login_info",{})["result"][1]["zte_web_sault"]
    h=sha(sha(PW)+salt)
    return call(ANON,"zwrt_web","web_login",{"password":h})["result"][1]["ubus_rpc_session"]
def st(r):
    if "_err" in r: return f"NETERR {r['_err']}"
    if "error" in r: return f"ACL code={r['error'].get('code')}"
    res=r.get("result"); 
    return f"[{res[0] if res else '?'}] {json.dumps(res[1])[:200] if res and len(res)>1 else ''}"

mode=sys.argv[1] if len(sys.argv)>1 else "ping"
target=sys.argv[2] if len(sys.argv)>2 else "127.0.0.1"
actions=sys.argv[3:] or (["1","start","0","diag"] if mode=="ping" else ["1","start"])
obj="zwrt_router.api"
setm= "router_set_ping_diagnose" if mode=="ping" else "router_set_traceroute_diagnose"
tgtkey="ping_target" if mode=="ping" else "traceroute_target"
actkey="ping_action" if mode=="ping" else "traceroute_action"
getm= "router_get_ping_diag_result"  # ping stats; trace has no separate result method

s=login()
print(f"session {s[:8]}...  mode={mode} target={target!r}")
print("current para:", st(call(s,obj,"router_get_ping_traceroute_para",{})))

for action in actions:
    t0=time.time()
    r=call(s,obj,setm,{tgtkey:target,actkey:action})
    dt=time.time()-t0
    print(f"\n--- {setm} {tgtkey}={target!r} {actkey}={action!r}  ({dt:.2f}s) ---")
    print("  set ->", st(r))
    # poll result a few times
    for k in range(5):
        time.sleep(1.0)
        rr=call(s,obj,getm,{})
        print(f"  result[{k}] ({time.time()-t0:4.1f}s) ->", st(rr))
