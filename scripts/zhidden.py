#!/usr/bin/env python3
"""Try web_hidden_page_login with the factory sticker password, then if it
succeeds, attempt zwrt_bsp.usb set {mode:debug} (the ADB restore call).
ROUTER_PW env = current web pw; STICKER env = factory default to try as hidden pw."""
import os, sys, json, time, hashlib, urllib.request, http.cookiejar

GW=os.environ.get("GW","192.168.0.1")
PW=os.environ["ROUTER_PW"]            # current web password (works)
STICKER=os.environ.get("STICKER","")  # factory default to try as hidden-page pw
ANON="0"*32
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
def st(r):
    if "error" in r: return f"ERR{r['error'].get('code')}"
    res=r.get("result"); return f"[{res[0] if res else '?'}] {json.dumps(res[1]) if res and len(res)>1 else ''}"

salt=call(ANON,"zwrt_web","web_login_info",{})["result"][1]["zte_web_sault"]
s=call(ANON,"zwrt_web","web_login",{"password":sha(sha(PW)+salt)})["result"][1]["ubus_rpc_session"]
print(f"web login OK (current pw). session {s[:8]}")
print(f"trying hidden_page_login with sticker default {STICKER!r} ...\n")

cands = [
    ("existing-session, double-hash", s, sha(sha(STICKER)+salt)),
    ("anon, double-hash",             ANON, sha(sha(STICKER)+salt)),
]
got_session = None
for label, sess, hpw in cands:
    r=call(sess,"zwrt_web","web_hidden_page_login",{"password":hpw})
    res=r.get("result")
    ok = res and len(res)>1 and str(res[1].get("result"))=="0"
    print(f"  [{label}] -> {st(r)}  {'<<< SUCCESS' if ok else ''}")
    if ok:
        got_session = res[1].get("ubus_rpc_session") or sess
        # if it returned a new session use it, else keep elevating the existing one
        got_session = got_session if isinstance(got_session,str) and got_session!=ANON else s
        break
    time.sleep(0.5)

if not got_session:
    print("\nhidden_page_login did not succeed with double-hash. (Not trying more variants to avoid lockout.)")
    sys.exit(0)

print(f"\nElevated session available: {got_session[:8]}")
print("=== Testing the ADB-restore call on the elevated session ===")
r=call(got_session,"zwrt_bsp.usb","set",{"mode":"debug"})
print("  zwrt_bsp.usb set {mode:debug} ->", st(r))
# also try superuser enable + webfunction re-check, in case elevation needs it too
call(got_session,"zwrt_web","web_superuser_switch_set",{"switch":"1"})
print("  webfunction_check ->", st(call(got_session,"zwrt_web","webfunction_check",{})))
r2=call(got_session,"zwrt_bsp.usb","set",{"mode":"debug"})
print("  zwrt_bsp.usb set {mode:debug} (after superuser) ->", st(r2))
print("  zwrt_bsp.usb list ->", st(call(got_session,"zwrt_bsp.usb","list",{})))
