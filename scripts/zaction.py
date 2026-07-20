#!/usr/bin/env python3
"""Discover the action enum for ping/traceroute/dnsquery diag methods, then
report which value actually starts a diag. ROUTER_PW env. Non-destructive."""
import os, sys, json, time, hashlib, urllib.request, http.cookiejar

GW=os.environ.get("GW","192.168.0.1"); PW=os.environ["ROUTER_PW"]; ANON="0"*32
T=lambda:int(time.time()*1000)
_J=http.cookiejar.CookieJar(); OP=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_J))
def post(pl):
    req=urllib.request.Request(f"http://{GW}/ubus/?t={T()}",data=json.dumps(pl).encode(),
        headers={"Content-Type":"application/json","Accept":"*/*","User-Agent":"Mozilla/5.0",
        "Origin":f"http://{GW}","Referer":f"http://{GW}/","Z-Mode":"1"})
    try:
        with OP.open(req,timeout=20) as r: return json.loads(r.read().decode())
    except Exception as e: return [{"_err":str(e)}]
def call(s,o,m,p=None): return post([{"jsonrpc":"2.0","id":1,"method":"call","params":[s,o,m,p or {}]}])[0]
def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()
def code(r):
    if "_err" in r: return "NETERR"
    if "error" in r: return f"ACL{r['error'].get('code')}"
    res=r.get("result"); 
    if not res: return "EMPTY"
    return res[0]
def body(r):
    if "error" in r or "_err" in r: return ""
    res=r.get("result")
    return json.dumps(res[1])[:140] if res and len(res)>1 else ""

salt=call(ANON,"zwrt_web","web_login_info",{})["result"][1]["zte_web_sault"]
s=call(ANON,"zwrt_web","web_login",{"password":sha(sha(PW)+salt)})["result"][1]["ubus_rpc_session"]
print("session", s[:8])

ACTIONS=["1","0","2","3","Start","Stop","start","stop","Diagnose","Clear",
         "begin","end","run","Run","reset","Reset","query","Query"]

def probe(name, setmethod, setkeys, getmethod, target="127.0.0.1"):
    print(f"\n===== {name} =====")
    for a in ACTIONS:
        params={setkeys["target"]:target, setkeys["action"]:a}
        r=call(s,"zwrt_router.api",setmethod,params)
        c=code(r)
        flag=""
        if c==0: flag="  <== code 0!"
        print(f"  action={a!r:12} -> [{c}] {body(r)}{flag}")
    # also try dnsquery-style alternate param names if dnsquery
    if "alt" in setkeys:
        for tgt in [target]:
            for variant in [{setkeys["target"]:tgt},
                            {setkeys.get("alt","domain"):tgt,setkeys["action"]:"1"},
                            {setkeys.get("alt","domain"):tgt}]:
                r=call(s,"zwrt_router.api",setmethod,variant)
                print(f"  variant {list(variant)} -> [{code(r)}] {body(r)}")

probe("PING", "router_set_ping_diagnose",
      {"target":"ping_target","action":"ping_action"}, "router_get_ping_diag_result")
# after probing actions, if any started, show result
print("  -- final ping result:", body(call(s,"zwrt_router.api","router_get_ping_diag_result",{})))

probe("DNSQUERY", "router_set_dnsquery_diagnose",
      {"target":"dnsquery_target","action":"dnsquery_action","domain":"domain"},
      "router_get_dnsquery_diag_result", target="localhost")
print("  -- final dnsquery result:", body(call(s,"zwrt_router.api","router_get_dnsquery_diag_result",{})))

probe("TRACEROUTE", "router_set_traceroute_diagnose",
      {"target":"traceroute_target","action":"traceroute_action"}, None, target="127.0.0.1")
