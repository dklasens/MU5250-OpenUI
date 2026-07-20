#!/usr/bin/env python3
"""zcall.py — generic authenticated ubus caller + session-cookie GET for the MU5250.

Reads ROUTER_PW (and optionally GW, default 192.168.0.1) from the environment.

Usage:
  ROUTER_PW=... python3 scripts/zcall.py call <object> <method> ['<params-json>']
  ROUTER_PW=... python3 scripts/zcall.py get <path> [-o outfile]     # authed GET
  ROUTER_PW=... python3 scripts/zcall.py login                       # print session only
"""
import sys, os, json, time, hashlib, urllib.request, http.cookiejar

GW = os.environ.get("GW", "192.168.0.1")
BASE = f"http://{GW}"
PW = os.environ.get("ROUTER_PW")
if not PW:
    sys.exit("ROUTER_PW not set")

_cj = http.cookiejar.CookieJar()
_opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_cj))
SESSION = None

def call(session, obj, method, params):
    url = f"{BASE}/ubus/?t={int(time.time())}"
    body = json.dumps([{"jsonrpc": "2.0", "id": 1, "method": "call",
                        "params": [session, obj, method, params]}]).encode()
    req = urllib.request.Request(url, data=body, headers={
        "Content-Type": "application/json", "Origin": BASE, "Referer": BASE + "/"})
    return json.loads(_opener.open(req, timeout=20).read())

def _sha(x): return hashlib.sha256(x.encode()).hexdigest().upper()

def login(username=None):
    global SESSION
    salt = call("0" * 32, "zwrt_web", "web_login_info", {})[0]["result"][1]["zte_web_sault"]
    h = _sha(_sha(PW) + salt)
    params = {"password": h}
    if username:
        params["username"] = username
    resp = call("0" * 32, "zwrt_web", "web_login", params)
    SESSION = resp[0]["result"][1]["ubus_rpc_session"]
    return SESSION

def authed_get(path):
    req = urllib.request.Request(BASE + path, headers={"Referer": BASE + "/"})
    return _opener.open(req, timeout=30).read()

if __name__ == "__main__":
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    login()
    if args[0] == "login":
        print(SESSION)
    elif args[0] == "call":
        obj, method = args[1], args[2]
        params = json.loads(args[3]) if len(args) > 3 else {}
        print(json.dumps(call(SESSION, obj, method, params), indent=None))
    elif args[0] == "get":
        data = authed_get(args[1])
        if "-o" in args:
            out = args[args.index("-o") + 1]
            open(out, "wb").write(data)
            print(f"{len(data)} bytes -> {out}")
        else:
            sys.stdout.buffer.write(data)
