#!/usr/bin/env python3
"""Probe DDNS + LAN hostname setters: confirm they work, then charset-probe
string fields for command-injection potential. ROUTER_PW env."""
import os, json, time, hashlib, urllib.request, http.cookiejar, base64

GW=os.environ.get("GW","192.168.0.1"); PW=os.environ["ROUTER_PW"]; ANON="0"*32
_J=http.cookiejar.CookieJar(); OP=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_J))
def post(pl):
    req=urllib.request.Request(f"http://{GW}/ubus/?t={int(time.time()*1000)}",data=json.dumps(pl).encode(),
        headers={"Content-Type":"application/json","Accept":"*/*","User-Agent":"Mozilla/5.0",
        "Origin":f"http://{GW}/","Referer":f"http://{GW}/","Z-Mode":"1"})
    try:
        with OP.open(req,timeout=20) as r: return json.loads(r.read().decode(errors="replace"))
    except urllib.error.HTTPError as e:
        try: return json.loads(e.read().decode())
        except: return [{"error":{"code":e.code}}]
def call(s,o,m,p=None):
    for _ in range(3):
        r=post([{"jsonrpc":"2.0","id":1,"method":"call","params":[s,o,m,p or {}]}])[0]
        if not (isinstance(r,dict) and r.get("error",{}).get("code")==400): return r
        time.sleep(0.2)
    return r
def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()
def code(r):
    if "error" in r: return f"E{r['error'].get('code')}"
    res=r.get("result"); return res[0] if res else "?"
def body(r):
    res=r.get("result"); return json.dumps(res[1])[:120] if res and len(res)>1 else ""

salt=call(ANON,"zwrt_web","web_login_info",{})["result"][1]["zte_web_sault"]
s=call(ANON,"zwrt_web","web_login",{"password":sha(sha(PW)+salt)})["result"][1]["ubus_rpc_session"]
print("session",s[:8])

print("\n=== LAN hostname ===")
print("get_modified_lan_hostname:", body(call(s,"zwrt_router.api","router_get_modified_lan_hostname",{})))
# charset probe on hostname
print("hostname charset probe (0=accepted):")
for c in list(";|&`$() <>\n\t'\"\\*?,#@!^~%+=:/"):
    h="host"+c+"x"
    r=call(s,"zwrt_router.api","router_modify_lan_hostname",{"hostname":h})
    cc=code(r)
    if cc==0: print(f"  {c!r:6} ACCEPTED  -> {body(r)}")
# restore a safe hostname
call(s,"zwrt_router.api","router_modify_lan_hostname",{"hostname":"OpenU60"})

print("\n=== DDNS set (valid enable=1 config) ===")
b64=lambda x: base64.b64encode(x.encode()).decode()
cfg={"enable":1,"accout":b64("testuser"),"password":b64("testpass"),"mode":"auto",
     "service":"dyndns.org","domain":"injtest.dyndns.org","hash":""}
print("set_ddns enable=1 full:", code(call(s,"zwrt_router.api","router_set_ddns",cfg)), body(call(s,"zwrt_router.api","router_set_ddns",cfg)))
# charset probe on domain
print("domain charset probe (0=accepted):")
for c in list(";|&`$() <>\n\t'\"*?,#@!^~%+="):
    d="x.dyndns.org"+c+"id"
    r=call(s,"zwrt_router.api","router_set_ddns",{**cfg,"domain":d})
    cc=code(r)
    if cc==0: print(f"  {c!r:6} ACCEPTED")
# disable ddns after
call(s,"zwrt_router.api","router_set_ddns",{"enable":0,"accout":"","password":"","mode":"auto","service":"","domain":"","hash":""})

print("\n=== PPPoE username (wan mode is PPP already) — charset probe via set_pppoe ===")
print("get pppoe info:", body(call(s,"zwrt_router.api","router_get_wan_mode_para",{})))
