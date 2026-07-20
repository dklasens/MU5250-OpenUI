#!/usr/bin/env python3
"""Test the NEW leads from ubus_vvv.txt: zwrt_mc.device.manager ADB/factory
methods, and web_hidden_page_login WITH a username field. ROUTER_PW env.

Starts with safe reads (accessibility check, fac_query_key), then tries the
adb_switch / fac_open / hidden_page_login with candidate credentials.
"""
import os, sys, json, time, hashlib, urllib.request, http.cookiejar, base64

GW=os.environ.get("GW","192.168.0.1"); PW=os.environ["ROUTER_PW"]; ANON="0"*32
STICKER_WEB=os.environ.get("STICKER_WEB","")   # factory sticker web password
STICKER_WIFI=os.environ.get("STICKER_WIFI","") # factory sticker WiFi password
IMEI=os.environ.get("ZTE_IMEI","")             # device IMEI (candidate credential)
_J=http.cookiejar.CookieJar(); OP=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_J))
def post(pl):
    req=urllib.request.Request(f"http://{GW}/ubus/?t={int(time.time()*1000)}",data=json.dumps(pl).encode(),
        headers={"Content-Type":"application/json","Accept":"*/*","User-Agent":"Mozilla/5.0",
        "Origin":f"http://{GW}/","Referer":f"http://{GW}/","Z-Mode":"1"})
    last=None
    for _ in range(4):
        try:
            with OP.open(req,timeout=15) as r: return json.loads(r.read().decode(errors="replace"))
        except Exception as e: last=e; time.sleep(1.0)
    return [{"error":{"code":"timeout","message":str(last)}}]
def call(s,o,m,p=None): return post([{"jsonrpc":"2.0","id":1,"method":"call","params":[s,o,m,p or {}]}])[0]
def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()
def md5u(s): return hashlib.md5(s.encode()).hexdigest().upper()
def st(r):
    if "error" in r: return f"ACL{r['error'].get('code')}"
    res=r.get("result")
    if not res: return "EMPTY"
    return f"[{res[0]}] {json.dumps(res[1])[:200] if len(res)>1 else '(no body)'}"

def login():
    salt=post([{"jsonrpc":"2.0","id":1,"method":"call","params":[ANON,"zwrt_web","web_login_info",{}]}])[0]
    slt=salt.get("result",[None,{}])[1].get("zte_web_sault") if salt.get("result") else None
    if not slt: return None, "no salt / unreachable"
    s=call(ANON,"zwrt_web","web_login",{"password":sha(sha(PW)+slt)})
    res=s.get("result")
    if res and len(res)>1 and str(res[1].get("result"))=="0":
        return res[1]["ubus_rpc_session"], slt
    return None, f"login fail: {s}"

s,slt=login()
if not s:
    print("LOGIN FAILED:", slt); sys.exit(1)
print(f"web login OK. session {s[:8]}  salt {slt[:8]}..")
DM="WEB"

print("\n=== [1] zwrt_mc.device.manager accessibility (safe read) ===")
print("  get_device_info:", st(call(s,"zwrt_mc.device.manager","get_device_info",{"deviceInfoList":["sw_version","wa_version","cr_version"]})))

print("\n=== [2] factory-key query (no params, safe) ===")
print("  fac_query_key:", st(call(s,"zwrt_mc.device.manager","fac_query_key",{})))
print("  fac_query:", st(call(s,"zwrt_mc.device.manager","fac_query",{})))

print("\n=== [3] adb_switch — candidate passwords (LOOK FOR [0] or non-[3]) ===")
cands=[
    ("router-pw plaintext", PW),
    ("router-pw double-hash", sha(sha(PW)+slt)),
    ("router-pw single-hash", sha(PW)),
    ("sticker-web", STICKER_WEB),
    ("sticker-web double-hash", sha(sha(STICKER_WEB)+slt)),
    ("sticker-wifi", STICKER_WIFI),
    ("sticker-wifi double-hash", sha(sha(STICKER_WIFI)+slt)),
    ("empty", ""),
    ("ZTE", "ZTE"),
    ("imei", IMEI),
    ("imei double-hash", sha(sha(IMEI)+slt)),
]
for label,pwd in cands:
    r=call(s,"zwrt_mc.device.manager","adb_switch",{"adbSwitchPwd":pwd})
    res=r.get("result"); code=res[0] if res else "?"
    mark = "   <<<<<<<<<< ADB?" if (code==0) else ("  (changed from 3)" if code not in (3,) else "")
    print(f"  {label:28} [{code}] {json.dumps(res[1])[:120] if res and len(res)>1 else ''}{mark}")
    if code==0: break  # don't keep hammering if one works

print("\n=== [4] fac_open with candidates (factory mode — could unlock file.exec) ===")
for label,pwd in [("router-pw",PW),("router-pw hash",sha(sha(PW)+slt)),("sticker-web",STICKER_WEB),("empty","")]:
    r=call(s,"zwrt_mc.device.manager","fac_open",{"facPwd":pwd,"moduleName":DM})
    res=r.get("result"); code=res[0] if res else "?"
    print(f"  fac_open {label:18} [{code}] {json.dumps(res[1])[:120] if res and len(res)>1 else ''}")
    if code==0: 
        print("  >>> factory mode open? re-test file.exec / usb.set next"); break

print("\n=== [5] web_hidden_page_login WITH username field ===")
for user in ["admin","user1","user","Admin",""]:
    for label,pwd in [("router-pw hash",sha(sha(PW)+slt)),("sticker-web hash",sha(sha(STICKER_WEB)+slt))]:
        r=call(s,"zwrt_web","web_hidden_page_login",{"username":user,"password":pwd})
        res=r.get("result"); code=res[0] if res else "?"
        ok = res and len(res)>1 and str(res[1].get("result"))=="0"
        print(f"  user={user!r:8} {label:18} [{code}] {json.dumps(res[1])[:100] if res and len(res)>1 else ''}{' <<< SUCCESS' if ok else ''}")
        if ok:
            print("  >>> hidden page OK — testing zwrt_bsp.usb set + adb_switch")
            print("    usb.set:", st(call(s,"zwrt_bsp.usb","set",{"mode":"debug"})))
            print("    adb_switch:", st(call(s,"zwrt_mc.device.manager","adb_switch",{"adbSwitchPwd":PW})))
            sys.exit(0)

print("\n=== [6] sanity: zwrt_bsp.usb.set still masked? ===")
print("  usb.set debug:", st(call(s,"zwrt_bsp.usb","set",{"mode":"debug"})))
