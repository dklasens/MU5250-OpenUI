#!/usr/bin/env python3
"""Decisively determine the write gate on zwrt_router.api setters.
ROUTER_PW env. Tests: full-param set, zte_nwinfo setter, superuser toggle."""
import os, sys, json, time, hashlib, urllib.request, http.cookiejar

GW=os.environ.get("GW","192.168.0.1"); PW=os.environ["ROUTER_PW"]; ANON="0"*32
T=lambda:int(time.time()*1000)
_J=http.cookiejar.CookieJar(); OP=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_J))
def post(pl):
    req=urllib.request.Request(f"http://{GW}/ubus/?t={T()}",data=json.dumps(pl).encode(),
        headers={"Content-Type":"application/json","Accept":"*/*","User-Agent":"Mozilla/5.0",
        "Origin":f"http://{GW}","Referer":f"http://{GW}/","Z-Mode":"1"})
    with OP.open(req,timeout=20) as r: return json.loads(r.read().decode())
def call(s,o,m,p=None): return post([{"jsonrpc":"2.0","id":1,"method":"call","params":[s,o,m,p or {}]}])[0]
def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()
def st(r):
    if "error" in r: return f"ACL{r['error'].get('code')}"
    res=r.get("result")
    if not res: return "EMPTY"
    return f"[{res[0]}] {json.dumps(res[1])[:130] if len(res)>1 else ''}"

def fresh( elevate=False):
    salt=call(ANON,"zwrt_web","web_login_info",{})["result"][1]["zte_web_sault"]
    s=call(ANON,"zwrt_web","web_login",{"password":sha(sha(PW)+salt)})["result"][1]["ubus_rpc_session"]
    return s

s=fresh()
print("session", s[:8])
print("baseline webfunction_check:", st(call(s,"zwrt_web","webfunction_check",{})))
print("baseline developer_login_info:", st(call(s,"zwrt_web","web_developer_login_info",{})))

# current ddns
ddns=call(s,"zwrt_router.api","router_get_ddns",{})["result"][1]
print("\ncurrent ddns:", json.dumps(ddns)[:200])

print("\n=== A) full-param set_ddns mirroring current (enable stays 0) ===")
full={"enable":int(ddns.get("enable",0)),"accout":ddns.get("accout",""),
      "password":ddns.get("password",""),"mode":ddns.get("mode","auto"),
      "service":ddns.get("service",""),"domain":ddns.get("domain",""),"hash":ddns.get("hash","")}
print("set_ddns full ->", st(call(s,"zwrt_router.api","router_set_ddns",full)))

print("\n=== B) compare: zte_nwinfo_api setter (used in bootloop notes over ssh) ===")
# read current mode first
print("get netinfo:", st(call(s,"zte_nwinfo_api","nwinfo_get_netinfo",{})))
# harmless setter probe with empty to see requirement
print("nwinfo_set_netselect {} ->", st(call(s,"zte_nwinfo_api","nwinfo_set_netselect",{})))

print("\n=== C) superuser toggle attempts + recheck webfunction_allow ===")
for p in [{"switch":"1"},{"superuser":"1"},{"enable":"1"},{"value":"1"},
          {"superuser_switch":"1"},{"flag":"1"},{"status":"1"}]:
    r=call(s,"zwrt_web","web_superuser_switch_set",p)
    wf=call(s,"zwrt_web","webfunction_check",{})
    print(f"  superuser_set {p} -> {st(r)}   webfunction_check -> {st(wf)}")

print("\n=== D) developer elevation then re-test writes ===")
salt2=call(s,"zwrt_web","web_login_info",{})["result"][1]["zte_web_sault"]
dev=call(s,"zwrt_web","web_developer_option_login",{"password":sha(sha(PW)+salt2)})
print("dev elevation:", st(dev))
print("developer_login_info after:", st(call(s,"zwrt_web","web_developer_login_info",{})))
print("webfunction_check after dev:", st(call(s,"zwrt_web","webfunction_check",{})))
print("set_ddns full after dev ->", st(call(s,"zwrt_router.api","router_set_ddns",full)))
print("set_ping (8.8.8.8,start) after dev ->", st(call(s,"zwrt_router.api","router_set_ping_diagnose",{"ping_target":"8.8.8.8","ping_action":"1"})))
