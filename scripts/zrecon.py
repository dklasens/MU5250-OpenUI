#!/usr/bin/env python3
"""ZTE U60 Pro access-recon over the stock web ubus RPC.

Auth model (from service_rpc.js + zte-script-ng.js):
  1. salt  = ubus zwrt_web web_login_info {} -> zte_web_sault
  2. ph    = SHA256(plain_password).upper()
  3. h     = SHA256(ph + salt).upper()
  4. web_login {password: h}            -> session S   (normal user)
  5. web_developer_option_login {password: h}  (called within S, same pw) -> elevates S

This tool is READ-ONLY recon: it logs in, baselines which generic rpcd objects
are ACL-denied vs allowed, elevates to developer, and re-checks. It also reads
state of the privileged zwrt_web toggles. Nothing destructive is called.

Password comes from ROUTER_PW env (never written to disk).
"""
import os, sys, json, time, hashlib, urllib.request
import http.cookiejar

GW   = os.environ.get("GW", "192.168.0.1")
PW   = os.environ.get("ROUTER_PW") or (_ := (_ for _ in ()).throw(SystemExit("set ROUTER_PW")))
ANON = "0" * 32
T    = lambda: int(time.time() * 1000)

# Cookie jar so the webtoken cookie issued by uhttpd is carried on later calls.
_JAR = http.cookiejar.CookieJar()
_OPENER = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_JAR))

def post(payload):
    url = f"http://{GW}/ubus/?t={T()}"
    req = urllib.request.Request(url, data=json.dumps(payload).encode(), headers={
        "Content-Type": "application/json",
        "Accept": "application/json, text/plain, */*",
        "User-Agent": "Mozilla/5.0 zte-recon",
        "Origin": f"http://{GW}", "Referer": f"http://{GW}/",
        "Z-Mode": "1",
    })
    try:
        with _OPENER.open(req, timeout=10) as r:
            return json.loads(r.read().decode(errors="replace"))
    except Exception as e:
        return [{"_err": f"{type(e).__name__}: {e}"}]

def call(sess, obj, method, params=None):
    return post([{"jsonrpc": "2.0", "id": 1, "method": "call",
                  "params": [sess, obj, method, params or {}]}])[0]

def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()

def get_salt(sess=ANON):
    r = call(sess, "zwrt_web", "web_login_info", {})
    try:
        return r["result"][1].get("zte_web_sault")
    except Exception:
        return None

def login(pw):
    salt = get_salt()
    if not salt:
        return None, "no salt"
    h = sha(sha(pw) + salt)
    r = call(ANON, "zwrt_web", "web_login", {"password": h})
    if "result" in r and len(r["result"]) > 1:
        d = r["result"][1]
        if str(d.get("result")) == "0" and d.get("ubus_rpc_session"):
            return d["ubus_rpc_session"], None
        return None, f"login denied: {d}"
    return None, f"login err: {r}"

def dev_login(sess, pw):
    salt = get_salt(sess) or get_salt()
    h = sha(sha(pw) + salt)
    r = call(sess, "zwrt_web", "web_developer_option_login", {"password": h})
    if "result" in r and len(r["result"]) > 1:
        return str(r["result"][1].get("result")) == "0", r["result"][1]
    return False, r

def status(r):
    if "_err" in r:   return f"NETERR {r['_err']}"
    if "error" in r:
        e = r["error"]
        return f"ERR code={e.get('code')} {e.get('message','')}"
    res = r.get("result")
    if isinstance(res, list):
        if len(res) > 1:
            return f"ok[{res[0]}] {json.dumps(res[1])[:160]}"
        return f"ok[{res[0] if res else '?'}]"
    return f"raw {json.dumps(r)[:160]}"

# Generic rpcd objects that are typically ACL-denied to a plain web session
# but become powerful (file read/write, uci set, exec) if unlocked.
GENERIC = [
    ("uci",          "get",        {"config":"system","section":"@system[0]"}),
    ("system",       "board",      {}),
    ("session",      "access",     {"scope":"ubus","function":"*","object":"*"}),
    ("file",         "stat",       {"path":"/etc/passwd"}),
    ("file",         "list",       {"path":"/data"}),
    ("service",      "list",       {}),
    ("network",      "get_proto_handlers", {}),
    ("luci-rpc",     "getBoardJSON", {}),
]

# Privileged zwrt_web reads (non-destructive) + a couple of zte_bsp reads.
ZWRT_READS = [
    ("zwrt_web", "web_info", {}),
    ("zwrt_web", "web_developer_login_info", {}),
    ("zwrt_web", "web_debug_mode_get", {}),
    ("zwrt_web", "web_hidden_page_password_settings_flag_get", {}),
    ("zwrt_web", "admin_password_changed_flag_get", {}),
    ("zwrt_web", "web_superuser_switch_set", {}),   # may be write-only; will tell us
    ("zwrt_web", "web_developer_login_info", {}),
    ("zwrt_bsp.usb", "list", {}),
]

def banner(s): print("\n" + "=" * 70 + f"\n {s}\n" + "=" * 70)

def main():
    banner(f"ZTE U60 Pro recon  gw={GW}")
    sess, err = login(PW)
    if not sess:
        print("NORMAL LOGIN FAILED:", err); return 1
    print("normal login OK  session=" + sess[:8] + "...")

    banner("BASELINE: generic rpcd objects (plain web session)")
    base = {}
    for obj, m, p in GENERIC:
        r = call(sess, obj, m, p); st = status(r); base[(obj, m)] = st
        print(f"  {obj:14s}.{m:22s} {st}")

    banner("zwrt_web privileged state reads")
    for obj, m, p in ZWRT_READS:
        r = call(sess, obj, m, p)
        print(f"  {obj:14s}.{m:34s} {status(r)}")

    banner("DEVELOPER ELEVATION (web_developer_option_login, same password)")
    ok, info = dev_login(sess, PW)
    print("developer elevation:", "OK" if ok else "FAILED", "->", json.dumps(info)[:200])

    banner("RE-CHECK generic rpcd objects AFTER developer elevation")
    changed = []
    for obj, m, p in GENERIC:
        r = call(sess, obj, m, p); st = status(r)
        before = base[(obj, m)]
        flag = "  *** CHANGED ***" if st != before else ""
        print(f"  {obj:14s}.{m:22s} {st}{flag}")
        if st != before:
            changed.append((obj, m, before, st))

    banner("SUMMARY")
    print("developer elevated:", ok)
    print("objects unlocked by elevation:", [f"{o}.{m}" for o,m,_,_ in changed] or "NONE")
    # Save nothing to disk. Caller may redirect if desired.
    return 0

if __name__ == "__main__":
    sys.exit(main())
