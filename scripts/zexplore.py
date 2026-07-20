#!/usr/bin/env python3
"""Explore uci readability + ping/traceroute diag state. Read-only. ROUTER_PW env."""
import os, sys, json, time, hashlib, urllib.request, http.cookiejar

GW = os.environ.get("GW", "192.168.0.1")
PW = os.environ["ROUTER_PW"]
ANON = "0"*32
T = lambda: int(time.time()*1000)
_JAR = http.cookiejar.CookieJar()
_OP = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_JAR))

def post(pl):
    req = urllib.request.Request(f"http://{GW}/ubus/?t={T()}",
        data=json.dumps(pl).encode(), headers={
        "Content-Type":"application/json","Accept":"application/json, text/plain, */*",
        "User-Agent":"Mozilla/5.0 zte-recon","Origin":f"http://{GW}",
        "Referer":f"http://{GW}/","Z-Mode":"1"})
    try:
        with _OP.open(req, timeout=12) as r: return json.loads(r.read().decode(errors="replace"))
    except Exception as e: return [{"_err":f"{type(e).__name__}: {e}"}]

def call(s,o,m,p=None): return post([{"jsonrpc":"2.0","id":1,"method":"call","params":[s,o,m,p or {}]}])[0]
def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()

def login():
    salt = call(ANON,"zwrt_web","web_login_info",{})["result"][1]["zte_web_sault"]
    h = sha(sha(PW)+salt)
    r = call(ANON,"zwrt_web","web_login",{"password":h})
    return r["result"][1]["ubus_rpc_session"]

def label(r):
    if "_err" in r: return f"NETERR {r['_err']}"
    if "error" in r: return f"ACL-DENIED code={r['error'].get('code')}"
    res=r.get("result")
    if isinstance(res,list):
        code = res[0] if res else "?"
        body = json.dumps(res[1])[:200] if len(res)>1 else ""
        # ubus codes: 0=ok 4=not_found 6=permission_denied 7=timeout
        names={0:"OK",1:"INVALID_CMD",2:"INVALID_ARG",3:"METHOD_NF",4:"NOT_FOUND",
               5:"NO_DATA",6:"PERM_DENIED",7:"TIMEOUT",8:"NOT_SUPPORTED"}
        return f"[{code} {names.get(code,'?')}] {body}"
    return f"raw {json.dumps(r)[:160]}"

s = login()
print("session:", s[:8]+"...\n")

print("=== uci readability across configs ===")
for cfg in ["firewall","network","wireless","dropbear","rpcd","system","uhttpd",
            "ucitrack","attendedsysupgrade","rpcd_ansi","zte_topsw_daemon"]:
    r = call(s,"uci","get",{"config":cfg})
    print(f"  {cfg:24s} {label(r)}")

print("\n=== uci: can we list configs? ===")
print("  uci configs:", label(call(s,"uci","configs",{})))

print("\n=== ping/traceroute diag params (current state) ===")
for m in ["router_get_ping_traceroute_para","router_get_ping_diag_result",
          "router_get_dnsquery_diag_result","router_get_status_no_auth"]:
    r = call(s,"zwrt_router.api",m,{})
    print(f"  {m:36s} {label(r)}")

print("\n=== superuser/developer toggles param probing (read-only gets) ===")
for o,m in [("zwrt_web","webfunction_check"),("zwrt_web","webtoken_check"),
            ("zwrt_web","web_security_check"),("zwrt_web","web_login_info"),
            ("zwrt_web","web_login_timeout_period_get")]:
    print(f"  {o}.{m:28s} {label(call(s,o,m,{}))}")
