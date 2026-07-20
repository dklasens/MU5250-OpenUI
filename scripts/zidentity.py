#!/usr/bin/env python3
"""Gather device identity + privilege flags for the hidden-page / superuser path.
ROUTER_PW env. READ-ONLY (no login attempts against gated endpoints)."""
import os, json, time, hashlib, urllib.request, http.cookiejar

GW=os.environ.get("GW","192.168.0.1"); PW=os.environ["ROUTER_PW"]; ANON="0"*32
T=lambda:int(time.time()*1000)
_J=http.cookiejar.CookieJar(); OP=urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_J))
def post(pl):
    req=urllib.request.Request(f"http://{GW}/ubus/?t={T()}",data=json.dumps(pl).encode(),
        headers={"Content-Type":"application/json","Accept":"*/*","User-Agent":"Mozilla/5.0",
        "Origin":f"http://{GW}/","Referer":f"http://{GW}/","Z-Mode":"1"})
    try:
        with OP.open(req,timeout=20) as r: return json.loads(r.read().decode(errors="replace"))
    except urllib.error.HTTPError as e:
        try: return json.loads(e.read().decode(errors="replace"))
        except Exception: return [{"error":{"code":e.code,"message":e.reason}}]
def call(s,o,m,p=None):
    for _ in range(3):
        r=post([{"jsonrpc":"2.0","id":1,"method":"call","params":[s,o,m,p or {}]}])[0]
        if not (isinstance(r,dict) and r.get("error",{}).get("code")==400): return r
        time.sleep(0.3)
    return r
def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()
def show(label,r):
    if "error" in r: print(f"  {label:42s}: ACL{r['error'].get('code')}"); return
    res=r.get("result")
    if not res: print(f"  {label:42s}: EMPTY"); return
    print(f"  {label:42s}: [{res[0]}] {json.dumps(res[1]) if len(res)>1 else ''}")

salt=call(ANON,"zwrt_web","web_login_info",{})["result"][1]["zte_web_sault"]
s=call(ANON,"zwrt_web","web_login",{"password":sha(sha(PW)+salt)})["result"][1]["ubus_rpc_session"]
print("session",s[:8])

print("\n=== device identity ===")
for o,m in [("zwrt_zte_mdm.api","get_imei"),("zwrt_zte_mdm.api","get_imei2"),
            ("zwrt_zte_mdm.api","get_mac_address"),("zwrt_zte_mdm.api","get_modem_msn"),
            ("zwrt_zte_mdm.api","get_sim_info"),("zwrt_web","device_info"),
            ("zwrt_web","web_info"),("zwrt_router.api","router_get_status_no_auth")]:
    show(f"{o}.{m}", call(s,o,m,{}))

print("\n=== privilege / hidden-page flags ===")
for o,m in [("zwrt_web","web_hidden_page_password_settings_flag_get"),
            ("zwrt_web","admin_password_changed_flag_get"),
            ("zwrt_web","web_password_is_defaultvalue_flag_get"),
            ("zwrt_web","web_password_settings_flag_get"),
            ("zwrt_web","web_password_settings_flag_get"),
            ("zwrt_web","web_debug_mode_get"),
            ("zwrt_web","webfunction_check"),
            ("zwrt_web","web_developer_login_info"),
            ("zwrt_web","web_privacy_read_flag_get"),
            ("zwrt_web","web_login_info")]:
    show(f"{o}.{m}", call(s,o,m,{}))

print("\n=== list ALL methods on zwrt_web to find any ADB/shell/usb-toggle we missed ===")
r=call(s,"zwrt_web","list",{})
show("zwrt_web.list", r)
