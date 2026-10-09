#!/usr/bin/env python3
"""Mock zte-agent for local dashboard demos and mutation/read-back tests.

Serves realistic U60 Pro data on :9090 so the dashboard can be reviewed
without the device. Payload shapes follow the Rust producers in agent/src/
(sanitised: every identifier, address and message is synthetic), and
mutations behave like the real agent: they validate like it, return what it
returns, and change the state the *next read* reports, including the
cross-resource effects (APN activation flips APN mode, a Wi-Fi TX write shows
up in /api/wifi/status, a NR band lock shows up in the netinfo fields, ...).

Usage:  python3 web-app/tools/mock_agent.py [--port 9090] [--host 0.0.0.0]
                                            [--scenario SA|NSA|LTE|disconnected]

Environment:
  MOCK_SCENARIO   radio scenario served by /api/dashboard `signal`
                  (default SA; also NSA, LTE, disconnected)

Test hooks (not part of the agent API; the contract checker only reads /api/):
  GET  /__mock/requests   every recorded non-GET request: method, path, body, status
  POST /__mock/reset      restore initial state (optional body {"scenario": "..."})

Routes the mock answers *without* simulating any state (fire-and-forget; the
request is still recorded): POST /api/device/reboot, /api/device/shutdown,
/api/system/restart-agent, /api/system/kill-bloat (returns the bloat set but
does not remove it from /api/system/top), PUT /api/usb/powerbank only mirrors
the OTG flag, POST /api/at/send (canned replies), and PUT /api/router/dns
(merged, not validated beyond shape).

Known simplifications are listed in the comments next to each handler
(LAN transition, serving-cell changes after a lock, no auth).
"""
import argparse
import calendar
import datetime
import json
import os
import random
import re
import socketserver
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

BOOT = time.time()
SCENARIOS = ("SA", "NSA", "LTE", "disconnected")

# Knobs tests (or a demo) may turn. `jitter` adds live noise to signal/throughput.
CONFIG = {
    "jitter": True,
    # PUT /api/usb/mode answers 202 immediately; the new composition is
    # reported by /api/usb/status only once this many seconds have passed
    # (agent: 1 s scheduling delay plus USB re-enumeration).
    "usb_switch_settle_s": 1.5,
    "scenario": os.environ.get("MOCK_SCENARIO", "SA"),
}

LOCK = threading.RLock()
REQUESTS = []
MAX_RECORDED = 500


def uptime():
    return int(time.time() - BOOT) + 384200  # pretend ~4.4 days of uptime


def jitter(base, pct=0.15):
    if not CONFIG["jitter"]:
        return base
    # No clamp: RSRP/RSRQ are negative dBm; positive bases stay positive
    # because (1 ± pct) never crosses zero for pct < 1.
    return base * (1 + random.uniform(-pct, pct))


class ApiError(Exception):
    """Raised by handlers; rendered as the agent's `{"ok": false, "error": …}`."""

    def __init__(self, status, message):
        super().__init__(message)
        self.status = status
        self.message = message


class Reply:
    """A non-default response: a status other than 200, a bare envelope, or raw bytes."""

    def __init__(self, status=200, data=None, envelope=None, raw=None, headers=None):
        self.status = status
        self.data = data
        self.envelope = envelope
        self.raw = raw  # (content_type, bytes)
        self.headers = headers or {}


# ── serde-style request validation ────────────────────────────────────────────

def _is_int(v):
    return isinstance(v, int) and not isinstance(v, bool)


KINDS = {
    "str": (lambda v: isinstance(v, str), "a string"),
    "bool": (lambda v: isinstance(v, bool), "a boolean"),
    "u8": (lambda v: _is_int(v) and 0 <= v <= 255, "u8"),
    "u32": (lambda v: _is_int(v) and 0 <= v <= 0xFFFFFFFF, "u32"),
    "u64": (lambda v: _is_int(v) and v >= 0, "u64"),
}


def need_object(body, label):
    if body is None:
        raise ApiError(400, "invalid JSON" if not label else f"{label}: invalid JSON")
    if not isinstance(body, dict):
        raise ApiError(400, "invalid JSON" if not label else f"{label}: invalid type, expected a map")
    return body


def struct_body(body, label, fields, optional=()):
    """Mirror `#[derive(Deserialize)] #[serde(deny_unknown_fields)]`.

    `fields` maps name -> kind; names in `optional` may be missing or null.
    Error text follows the agent's `format!("{label}: {serde error}")`.
    """
    obj = need_object(body, label)
    prefix = f"{label}: " if label else ""
    for key in obj:
        if key not in fields:
            expected = ", ".join(f"`{k}`" for k in fields)
            raise ApiError(400, f"{prefix}unknown field `{key}`, expected one of {expected}")
    out = {}
    for key, kind in fields.items():
        if key not in obj or (obj[key] is None and key in optional):
            if key not in optional:
                raise ApiError(400, f"{prefix}missing field `{key}`")
            out[key] = None
            continue
        check, name = KINDS[kind]
        if not check(obj[key]):
            raise ApiError(400, f"{prefix}invalid value for field `{key}`, expected {name}")
        out[key] = obj[key]
    return out


def ok_result():
    # ubus `set` calls on this firmware answer {"result": "success"}
    # (firmware/emulation/payload/root/rpcd/zte_nwinfo_api models the same).
    return {"result": "success"}


# ── Radio fixtures (zte_nwinfo_api.nwinfo_get_netinfo) ───────────────────────

# Modem-reported supported LTE bands, and the matching "all bands" mask the
# firmware reports when nothing is restricted.
LTE_SUPPORTED = [1, 2, 3, 4, 5, 7, 8, 18, 19, 20, 26, 28, 29, 32, 34, 38, 39, 40, 41, 42, 43, 48]
LTE_DEFAULT_LOCK = "0x87e29a0e00df"
# agent/src/cell.rs: the bands the dashboard may ask for.
AGENT_LTE_BANDS = [1, 2, 3, 4, 5, 7, 8, 18, 19, 20, 26, 28, 29, 32, 34, 38, 39, 40, 41, 42, 43, 48, 66, 71]
AGENT_NR_BANDS = [1, 2, 3, 5, 7, 8, 18, 20, 26, 28, 29, 38, 40, 41, 48, 66, 71, 75, 77, 78, 79]
NR_DEFAULT_LOCK = ",".join(str(b) for b in AGENT_NR_BANDS)
NETWORK_MODES = [
    ("WL_AND_5G", "5G / 4G / 3G"),
    ("LTE_AND_5G", "5G NSA"),
    ("Only_5G", "5G SA"),
    ("WCDMA_AND_LTE", "4G / 3G"),
    ("Only_LTE", "4G only"),
    ("Only_WCDMA", "3G only"),
]


def netinfo_for(scenario):
    """Raw netinfo for a scenario. Sanitised shape based on HK B04 captures."""
    if scenario not in SCENARIOS:
        raise ValueError(f"unknown MOCK_SCENARIO {scenario!r}; expected one of {', '.join(SCENARIOS)}")
    base = {
        "net_select_mode": "auto_select",
        "domain_stat": "CS_PS",
        "simcard_roam": "Home",
        "rmcc": 505,
        "rmnc": 1,
        "lac_code": 52254,
        "lte_band": ",".join(str(b) for b in LTE_SUPPORTED),
        "lte_band_lock": LTE_DEFAULT_LOCK,
        "gw_band_lock": "0x2000006c00000",
        "nr5g_sa_band_lock": NR_DEFAULT_LOCK,
        "nr5g_nsa_band_lock": NR_DEFAULT_LOCK,
        "lock_lte_cell": "",
        "lock_nr_cell": "",
        "nitz_sync_flag": 1,
    }
    if scenario == "SA":
        base.update({
            "network_type": "SA",
            "net_select": "Only_5G",
            "network_provider_fullname": "Telstra",
            "network_provider": "Telstra",
            "signalbar": "5",
            "cell_id": 19706418,
            # In SA the LTE fields hold stale values and the "active band" is NR.
            "lte_pci": 312,
            "wan_active_band": "n78",
            "wan_active_channel": 643392,
            "lte_rsrp": -48,
            "lte_rsrq": -9,
            "lte_snr": "28.0",
            "lte_rssi": -40,
            "lteca": "",
            "ltecasig": "",
            "nr5g_pci": 745,
            "nr5g_action_channel": 643392,
            "nr5g_action_band": "n78",
            "nr5g_bandwidth": "100",
            "nr5g_rsrp": -53,
            "nr5g_rsrq": -11,
            "nr5g_snr": "31.0",
            "nr5g_rssi": -43,
            "nr5g_cell_id": 5_345_678_901,
            "nrca": "",
        })
    elif scenario == "NSA":
        base.update({
            "network_type": "ENDC",
            "net_select": "LTE_AND_5G",
            "network_provider_fullname": "Telstra",
            "network_provider": "Telstra",
            "signalbar": "4",
            "cell_id": 134479973,
            # LTE PCC: B8, EARFCN 3650, PCI 312, 20 MHz
            "lte_pci": 312,
            "wan_active_channel": 3650,
            "wan_active_band": "B8",
            "lte_rsrp": -71,
            "lte_rsrq": -11,
            "lte_snr": "19.5",
            "lte_rssi": -62,
            # LTE CA: PCC + one SCC (B1, EARFCN 300, PCI 314, 15 MHz)
            "lteca": "312,8,1,3650,20,1;314,1,1,300,15,1",
            "ltecasig": "-79,-12.1,14.0,-71,1,2",
            # NR PCC: n78, ARFCN 630912, PCI 801, 100 MHz
            "nr5g_rsrp": -77,
            "nr5g_action_band": "n78",
            "nr5g_action_channel": 630912,
            "nr5g_pci": 801,
            "nr5g_bandwidth": "100",
            "nr5g_snr": "23.5",
            "nr5g_rsrq": -9,
            "nr5g_rssi": -58,
            "nr5g_cell_id": 268566611,
            # NR CA: one active SCC (n40, ARFCN 472000, PCI 803, 40 MHz), plus a
            # configured but unmeasured SCC (3GPP reporting-floor values).
            # Indices: 0=ul,1=pci,2=active,3=band,4=arfcn,5=bw,6=pad,7=rsrp,8=rsrq,9=sinr,10=rssi
            "nrca": "1,803,2,40,472000,40,0,-88,-11.4,12.5,-69;0,56,1,78,643392,60,1,-140.0,-43.0,-23.0,-120.0;",
        })
    elif scenario == "LTE":
        base.update({
            "network_type": "LTE",
            "net_select": "WCDMA_AND_LTE",
            "network_provider_fullname": "Telstra",
            "network_provider": "Telstra",
            "signalbar": "4",
            "cell_id": 19706418,
            "lte_pci": 417,
            "wan_active_band": "LTE BAND 28",
            "wan_active_channel": 9260,
            "lte_rsrp": -96,
            "lte_rsrq": -12,
            "lte_snr": "11.0",
            "lte_rssi": -63,
            "lteca": "417,28,0,9260,10;",
            "ltecasig": "",
            "nr5g_rsrp": 0,
            "nr5g_rsrq": 0,
            "nr5g_snr": "",
            "nr5g_rssi": 0,
            "nrca": "",
        })
    else:  # disconnected: registered nowhere, modem reports zeroes
        base.update({
            "network_type": "No Service",
            "net_select": "WL_AND_5G",
            "network_provider_fullname": "",
            "network_provider": "",
            "signalbar": "0",
            "cell_id": 0,
            "lte_pci": 0,
            "wan_active_band": "0",
            "wan_active_channel": 0,
            "lte_rsrp": 0, "lte_rsrq": 0, "lte_snr": "", "lte_rssi": 0,
            "lteca": "", "ltecasig": "",
            "nr5g_rsrp": 0, "nr5g_rsrq": 0, "nr5g_snr": "", "nr5g_rssi": 0,
            "nrca": "",
        })
    return base


def signal_raw():
    """The dashboard's `signal`: current netinfo with a little live noise."""
    raw = dict(STATE["netinfo"])
    for key in ("lte_rsrp", "nr5g_rsrp"):
        value = raw.get(key)
        if CONFIG["jitter"] and isinstance(value, int) and value < 0:
            raw[key] = value + random.randint(-2, 2)
    return raw


# ── Data usage (handlers.rs::read_data_usage_live) ───────────────────────────

def cycle_dates(reset_day, today=None):
    """(clear_date_record 'YYYY/MM/DD', next_clear_date 'YYYYMMDD') for a reset day."""
    today = today or datetime.date.today()

    def clamp(year, month):
        return datetime.date(year, month, min(reset_day, calendar.monthrange(year, month)[1]))

    def shift(year, month, by):
        index = year * 12 + (month - 1) + by
        return index // 12, index % 12 + 1

    this_month = clamp(today.year, today.month)
    if this_month <= today:
        start, nxt = this_month, clamp(*shift(today.year, today.month, 1))
    else:
        start, nxt = clamp(*shift(today.year, today.month, -1)), this_month
    return start.strftime("%Y/%m/%d"), nxt.strftime("%Y%m%d")


def usage_defaults():
    record, nxt = cycle_dates(16)
    return {
        "reset_day": 16,
        "reset_enabled": 1,  # agent: number 1/0, or null when unreadable
        "clear_date_record": record,
        "next_clear_date": nxt,
    }


def data_usage_payload():
    usage = STATE["usage"]
    connected = STATE["scenario"] != "disconnected"

    def period(rx, tx, secs, rx_packets, tx_packets):
        return {"rx_bytes": rx, "tx_bytes": tx, "time_secs": secs,
                "rx_packets": rx_packets, "tx_packets": tx_packets}

    month = period(64_800_000_000, 3_900_000_000, 640_000, 52_400_000, 18_900_000)
    return {
        "day": period(2_350_000_000, 118_000_000, 32_400, 1_900_000, 690_000),
        "month": month,
        "cycle": dict(month),
        "since_power_on": period(18_400_000_000 if connected else 0, 1_260_000_000 if connected else 0,
                                 uptime(), 14_800_000, 5_100_000),
        "total": period(402_000_000_000, 21_700_000_000, 4_120_000, 330_000_000, 118_000_000),
        "reset_day": usage["reset_day"],
        "reset_enabled": usage["reset_enabled"],
        "clear_date_record": usage["clear_date_record"],
        "next_clear_date": usage["next_clear_date"],
    }


def freshness(ttl_ms):
    return {"sampled_at_ms": int(time.time() * 1000), "age_ms": 0, "ttl_ms": ttl_ms,
            "stale": False, "error": None}


def battery_status():
    return STATE["charge_control"]["battery_status"]


def dashboard_batch():
    connected = STATE["scenario"] != "disconnected"
    rx = jitter(610_000_000 / 8, 0.3) if connected else 0  # ~610 Mbps in bytes/s
    tx = jitter(38_000_000 / 8, 0.3) if connected else 0
    charging = battery_status() == "Charging"
    wan = {
        "up": True,
        "ipv4-address": [{"address": "10.150.82.14"}],
        "ipv6-address": [{"address": "2406:3400:8123:4a00::1c"}],
        "route": [{"nexthop": "10.150.82.1"}],
        "dns-server": ["10.150.82.1"],
        "proto": "qmi",
    } if connected else {"up": False, "proto": "qmi"}
    wan6 = {
        "up": True,
        "ipv6-address": [{"address": "2406:3400:8123:4a00::1c", "mask": 64}],
        "ipv6-prefix": [{"address": "2406:3400:8123:4a00", "mask": 64}],
        "dns-server": ["2406:3400:8123::1"],
    } if connected else {"up": False}
    return {
        "device": {
            "hostname": "U60-Pro",
            "uptime_secs": uptime(),
            "load_avg": [0.42, 0.35, 0.31],
            "kernel": "Linux version 5.15.170-perf (builder@zte) "
                      "(aarch64-openwrt-linux-musl-gcc 12.3.0) #1 SMP PREEMPT",
            "firmware": "XCBZ_HK_MU5250V1.0.0B04",
            "hardware": "MU5250_HW1.0",
        },
        "battery": {
            "capacity": STATE["charge_control"]["capacity"],
            "status": battery_status(),
            "voltage_uv": 4_210_000,
            "temperature": 330,  # tenths of a degree
            "current_ua": 1_450_000 if charging else 0,
            "external_power": True,
        },
        "cpu": {"overall": round(jitter(23, 0.4), 1), "cores": [31, 22, 19, 20]},
        "memory": {"total_kb": 1_638_000, "used_kb": 612_000, "free_kb": 1_026_000, "usage_pct": 37.4},
        # Mirrors agent/src/system.rs::SpeedSnapshot exactly. Rates are bytes/sec.
        "speed": {
            "rx_bytes": 18_400_000_000,
            "tx_bytes": 1_260_000_000,
            "rx_speed": int(rx),
            "tx_speed": int(tx),
            "max_rx_speed": 712_000_000 // 8,
            "max_tx_speed": 46_000_000 // 8,
            "elapsed_ms": 3_000,
        },
        "data_usage": data_usage_payload(),
        "signal": signal_raw(),
        "bearer": {"qci": 6, "ambr_dl_mbps": 2000, "ambr_ul_mbps": 200},
        "wan": wan,
        "wan6": wan6,
        "thermal": {"cpuss_temp": round(jitter(61, 0.06), 1)},
        # agent/src/handlers.rs: per-source freshness (TTLs 1 s / 30 s / 10 s / 30 s).
        "sources": {
            "signal": freshness(1000), "wan": freshness(30000), "wan6": freshness(30000),
            "thermal": freshness(10000), "data_usage": freshness(30000), "speed": freshness(1000),
        },
        "charge_control_error": STATE["charge_control"]["last_error"],
    }


def clients():
    return {
        "clients": [
            {"mac": "00:00:5E:00:53:01", "ip": "192.168.0.101", "hostname": "macbook-pro",
             "medium": "wifi", "medium_detail": "wifi_5ghz", "wifi_band": "5 GHz",
             "signal_dbm": -42, "tx_bitrate_mbps": 2401.9, "rx_bitrate_mbps": 2401.9,
             "expected_throughput_mbps": 1680.0, "connected_secs": 184_200},
            {"mac": "00:00:5E:00:53:02", "ip": "192.168.0.102", "hostname": "iPhone-16",
             "medium": "wifi", "medium_detail": "wifi_5ghz", "wifi_band": "5 GHz",
             "signal_dbm": -55, "tx_bitrate_mbps": 1152.0, "rx_bitrate_mbps": 864.0,
             "expected_throughput_mbps": 780.0, "connected_secs": 96_500},
            {"mac": "00:00:5E:00:53:03", "ip": "192.168.0.103", "hostname": "living-room-tv",
             "medium": "wifi", "medium_detail": "wifi_2ghz", "wifi_band": "2.4 GHz",
             "signal_dbm": -67, "tx_bitrate_mbps": 144.4, "rx_bitrate_mbps": 115.6,
             "expected_throughput_mbps": 90.0, "connected_secs": 402_100},
            {"mac": "00:00:5E:00:53:04", "ip": "192.168.0.104", "hostname": "work-laptop",
             "medium": "usb-c", "medium_detail": "usb_c", "interface": "ncm0",
             "connected_secs": 7_800},
            {"mac": "00:00:5E:00:53:05", "ip": "192.168.0.105", "hostname": "office-pc",
             "medium": "ethernet", "medium_detail": "ethernet", "interface": "eth0",
             "wired_link_mbps": 1000, "connected_secs": 512_000},
        ]
    }


# ── Wi-Fi (wifi.rs) ───────────────────────────────────────────────────────────

def wifi_defaults():
    return {
        "wifi_onoff": "1", "wifi_onoff_supported": True,
        "wifi6_switch": "1", "wifi6_supported": True,
        "wifi7_supported": True,
        "radio2_disabled": "0", "radio5_disabled": "0",
        "channel_2g": "0", "channel_5g": "44",
        "actual_channel_2g": 6, "actual_channel_5g": 44,
        "actual_bw_2g": "40 MHz", "actual_bw_5g": "80 MHz",
        "htmode_2g": "EHT40", "htmode_5g": "EHT80",
        "hwmode_2g": "11beg", "hwmode_5g": "11bea",
        "supported_standards_2g": "b,g,n,ax,be",
        "supported_standards_5g": "a,n,ac,ax,be",
        "bandwidth_options_2g": ["EHT20", "EHT40"],
        "bandwidth_options_5g": ["EHT20", "EHT40", "EHT80", "EHT160"],
        "txpower_2g": "80", "txpower_5g": "80",
        "country_code": "AU",
        "ssid_2g": "U60Pro-Home", "ssid_5g": "U60Pro-Home",
        # The agent returns the stored passphrase verbatim (synthetic here).
        "key_2g": "mock-passphrase-2g", "key_5g": "mock-passphrase-5g",
        "has_key_2g": True, "has_key_5g": True,
        "encryption_2g": "psk3-mixed", "encryption_5g": "psk3-mixed",
        "hidden_2g": "0", "hidden_5g": "0",
        "clients_2g": 1, "clients_5g": 2, "clients_total": 3,
        "guest_ssid": "",
        "guest_disabled_2g": "1", "guest_disabled_5g": "1",
    }


WIFI_FIELDS = (
    "ssid_2g", "ssid_5g", "key_2g", "key_5g", "encryption_2g", "encryption_5g",
    "hidden_2g", "hidden_5g", "channel_2g", "channel_5g", "txpower_2g", "txpower_5g",
    "htmode_2g", "htmode_5g", "radio2_disabled", "radio5_disabled",
)
WIFI_GLOBAL = ("wifi_onoff", "wifi6_switch")
WIFI_ENCRYPTIONS = ("none", "psk2", "psk2+ccmp", "psk2+aes", "psk3", "psk3-mixed", "sae", "sae-mixed")
WIFI_5G_CHANNELS = (36, 40, 44, 48, 52, 56, 60, 64, 100, 104, 108, 112, 116, 120, 124, 128,
                    132, 136, 140, 144, 149, 153, 157, 161, 165)
SSID_FORBIDDEN = set("'\";$`\\|<>&")


def _valid_wifi_key(text):
    return 8 <= len(text) <= 63 or (len(text) == 64 and re.fullmatch(r"[0-9a-fA-F]{64}", text) is not None)


def wifi_value(key, value, wifi):
    """wifi.rs::wifi_value — the text to store, or ApiError(400)."""
    if isinstance(value, str):
        text = value
    elif isinstance(value, bool):
        text = "1" if value else "0"
    elif _is_int(value) and value >= 0:
        text = str(value)
    else:
        raise ApiError(400, f"{key} has an invalid type")
    if any(ord(c) < 32 or ord(c) == 127 for c in text):
        raise ApiError(400, f"{key} contains a control character")
    if key.startswith("ssid_"):
        valid = 1 <= len(text.encode()) <= 32 and not (set(text) & SSID_FORBIDDEN)
    elif key.startswith("key_"):
        valid = _valid_wifi_key(text)
    elif key.startswith("encryption_"):
        valid = text in WIFI_ENCRYPTIONS
    elif key.startswith("txpower_"):
        valid = text.isascii() and text.isdigit() and 1 <= int(text) <= 100
    elif key.startswith("channel_"):
        if text in ("auto", "0"):
            valid = True
        elif text.isascii() and text.isdigit():
            n = int(text)
            valid = 1 <= n <= 13 if key.endswith("2g") else n in WIFI_5G_CHANNELS
        else:
            valid = False
    elif key.startswith("htmode_"):
        valid = text in wifi["bandwidth_options_" + key[-2:]]
    else:
        valid = text in ("0", "1")
    if not valid:
        raise ApiError(400, f"{key} is outside the supported values")
    return text


def put_wifi_settings(body):
    """Validate the whole request first, then apply it (wifi.rs::plan_wifi + apply)."""
    if body is None:
        raise ApiError(400, "invalid Wi-Fi settings: invalid JSON")
    if not isinstance(body, dict):
        raise ApiError(400, "expected a Wi-Fi settings object")
    wifi = STATE["wifi"]
    if not body or len(body) > len(WIFI_FIELDS) + 2:
        raise ApiError(400, "empty or oversized Wi-Fi update")
    planned = {}
    for key, value in body.items():
        if key in WIFI_GLOBAL:
            supported = wifi.get("wifi_onoff_supported" if key == "wifi_onoff" else "wifi6_supported")
            if not supported:
                raise ApiError(400, f"{key} is not supported by this firmware")
        elif key not in WIFI_FIELDS:
            raise ApiError(400, f"unknown Wi-Fi setting: {key}")
        if key.startswith("key_") and value == "••••••••":
            continue  # the dashboard echoes the mask for an untouched password
        planned[key] = wifi_value(key, value, wifi)
    for suffix in ("2g", "5g"):
        if "encryption_" + suffix in body or "key_" + suffix in body:
            effective = {f: planned.get(f + "_" + suffix, wifi[f + "_" + suffix]) for f in ("encryption", "key")}
            if effective["encryption"] != "none" and not _valid_wifi_key(effective["key"]):
                raise ApiError(400, f"key_{suffix} is outside the supported values")
    changed = False
    for key, text in planned.items():
        if wifi.get(key) != text:
            changed = True
        wifi[key] = text
        if key.startswith("key_"):
            wifi["has_key_" + key[-2:]] = text != ""
    # Runtime values follow the configuration: auto channel picks one, the
    # active width follows htmode (EHT40 -> "40 MHz").
    for band, fallback in (("2g", 6), ("5g", 44)):
        configured = str(wifi["channel_" + band])
        wifi["actual_channel_" + band] = fallback if configured in ("0", "auto", "") else int(configured)
        width = re.search(r"(\d+)$", wifi["htmode_" + band])
        if width:
            wifi["actual_bw_" + band] = f"{width.group(1)} MHz"
    return {"status": "ok", "changed": changed}


# ── USB (usb.rs) ──────────────────────────────────────────────────────────────

USB_FUNCTIONS = {
    "ecm": ["gsi.ecm", "mass_storage.0"],
    "rndis": ["gsi.rndis", "mass_storage.0"],
    "ncm": ["ncm.0", "mass_storage.0"],
}


def usb_defaults():
    usb = {
        "mode": "user",
        "active_mode": "ecm",
        "default_mode": "ecm",
        "ncm_persist_on_boot": False,
        "supported_modes": ["rndis", "ecm", "ncm"],
        "experimental_modes": ["ncm"],
        "mode_capabilities": [
            {"mode": "rndis", "supported": True, "experimental": False, "function": "gsi.rndis"},
            {"mode": "ecm", "supported": True, "experimental": False, "function": "gsi.ecm"},
            {"mode": "ncm", "supported": True, "experimental": True, "function": "ncm.0",
             "note": "configfs NCM exists, but ZTE's ubus USB switch does not expose it"},
        ],
        "configfs": {"present": True, "ncm": True, "gsi_ecm": True, "gsi_rndis": True},
        "usb_ids": {"vendor": "0x19d2", "product": "0x1405"},
        "connect": 1,
        "typec_cc": "cc1",
        "link": {
            "negotiated": "super-speed", "negotiated_label": "USB 3.0", "negotiated_mbps": 5000,
            "max": "super-speed-plus", "max_label": "USB 3.1 Gen2", "max_mbps": 10000,
            "at_full_speed": False,
        },
    }
    usb_apply_mode(usb, "ecm")
    return usb


def usb_apply_mode(usb, mode):
    """Composition, bridge membership and interfaces that go with an active mode."""
    iface = {"ecm": "ecm0", "rndis": "rndis0", "ncm": "ncm0"}[mode]
    usb["active_mode"] = mode
    usb["composition_functions"] = list(USB_FUNCTIONS[mode])
    usb["bridge"] = {"name": "br-lan", "members": ["wlan0", "wlan2", iface]}
    usb["interfaces"] = {
        "ecm0": mode == "ecm", "rndis0": mode == "rndis", "ncm0": mode == "ncm",
        "ncm_ifname": "ncm0" if mode == "ncm" else None,
    }


def usb_settle():
    """Apply a scheduled switch once its delay has elapsed."""
    pending = STATE["usb_pending"]
    if pending and time.time() >= pending["due"]:
        usb_apply_mode(STATE["usb"], pending["mode"])
        STATE["usb_pending"] = None


def get_usb_status():
    usb_settle()
    return STATE["usb"]


def usb_busy():
    usb_settle()
    return STATE["usb_pending"] is not None


def put_usb_mode(body):
    obj = need_object(body, None)
    mode = obj.get("mode")
    if not isinstance(mode, str):
        raise ApiError(400, "mode is required")
    usb = STATE["usb"]
    usb_settle()

    def schedule(target, data):
        if usb_busy():
            raise ApiError(409, "another USB change is in progress")
        STATE["usb_pending"] = {"mode": target, "due": time.time() + CONFIG["usb_switch_settle_s"]}
        return Reply(202, data)

    if mode == "ncm":
        if obj.get("confirm_experimental") is not True:
            raise ApiError(400, "NCM is experimental and disrupts USB. Retry with "
                                "confirm_experimental=true from a Wi-Fi management path.")
        return schedule("ncm", {
            "status": "scheduled", "mode": "ncm", "experimental": True, "delay_ms": 1000,
            "rollback": "reboot or switch back to ECM after reconnecting",
        })
    if mode == "ecm" and "ncm.0" in usb["composition_functions"]:
        return schedule("ecm", {"status": "scheduled", "mode": "ecm", "delay_ms": 1000})
    if mode not in ("ecm", "rndis"):
        raise ApiError(400, "unsupported USB mode")
    if usb_busy():
        raise ApiError(409, "another USB change is in progress")
    usb_apply_mode(usb, mode)
    return ok_result()


def put_usb_default(body):
    if usb_busy():
        raise ApiError(409, "another USB change is in progress")
    obj = need_object(body, None)
    mode = obj.get("mode")
    if mode == "ecm":
        pass
    elif mode == "ncm":
        if obj.get("confirm_experimental") is not True:
            raise ApiError(400, "NCM persistence is experimental. Retry with "
                                "confirm_experimental=true from a Wi-Fi management path.")
    elif isinstance(mode, str):
        raise ApiError(400, "mode must be ecm or ncm")
    else:
        raise ApiError(400, "mode is required")
    STATE["usb"]["default_mode"] = mode
    STATE["usb"]["ncm_persist_on_boot"] = mode == "ncm"
    return {"default_mode": mode, "ncm_persist_on_boot": mode == "ncm"}


def put_powerbank(body):
    obj = need_object(body, None)
    state = obj.get("state")
    if _is_int(state):
        STATE["charger"]["otg_powerbank_state"] = state
    return ok_result()


# ── Power: charger / charge control (device_ext.rs, charge_policy.rs) ────────

def charge_control_defaults():
    return {
        "available": True, "battery_available": True, "charger_available": True,
        "charging_stopped": False, "battery_status": "Charging", "capacity": 78,
        "charge_limit_enabled": False, "charge_limit": 100, "hysteresis": 5,
        "manual_override": False, "last_error": None,
    }


def get_charge_control():
    return STATE["charge_control"]


def put_charge_control(body):
    """charge_policy.rs::ChargeLimitEnforcer::update, then charge_control_get."""
    update = struct_body(body, "invalid charge control", {
        "charging_stopped": "bool", "charge_limit_enabled": "bool",
        "charge_limit": "u8", "hysteresis": "u8",
    }, optional=("charging_stopped", "charge_limit_enabled", "charge_limit", "hysteresis"))
    cc = STATE["charge_control"]
    policy_changed = any(update[k] is not None for k in ("charge_limit_enabled", "charge_limit", "hysteresis"))
    if not policy_changed and update["charging_stopped"] is None:
        cc["last_error"] = "at least one charge control field is required"
        raise ApiError(503, cc["last_error"])
    policy = {
        "enabled": cc["charge_limit_enabled"] if update["charge_limit_enabled"] is None else update["charge_limit_enabled"],
        "limit": cc["charge_limit"] if update["charge_limit"] is None else update["charge_limit"],
        "hysteresis": cc["hysteresis"] if update["hysteresis"] is None else update["hysteresis"],
    }
    if not 50 <= policy["limit"] <= 100 or not 1 <= policy["hysteresis"] <= 20:
        cc["last_error"] = "limit must be 50-100 and hysteresis must be 1-20"
        raise ApiError(503, cc["last_error"])
    capacity = cc["capacity"]
    if update["charging_stopped"] is not None:
        stopped = update["charging_stopped"]
    elif not policy["enabled"]:
        stopped = False
    elif cc["battery_status"] == "Discharging" and not cc["charging_stopped"]:
        stopped = False
    elif capacity >= policy["limit"]:
        stopped = True
    elif capacity <= max(policy["limit"] - policy["hysteresis"], 0):
        stopped = False
    else:
        stopped = cc["charging_stopped"]
    cc["charging_stopped"] = stopped
    cc["battery_status"] = "Not charging" if stopped else "Charging"
    cc["charge_limit_enabled"] = policy["enabled"]
    cc["charge_limit"] = policy["limit"]
    cc["hysteresis"] = policy["hysteresis"]
    if update["charging_stopped"] is not None:
        cc["manual_override"] = update["charging_stopped"]
    elif policy_changed:
        # The enforcer clears a manual override whenever limit config changes.
        cc["manual_override"] = False
    cc["last_error"] = None
    return cc


def get_charger():
    # Inverted firmware semantics: direct_power_supply_mode "enable" = charging STOPPED.
    mode = "enable" if STATE["charge_control"]["charging_stopped"] else "disable"
    return {**STATE["charger"], "direct_power_supply_mode": mode}


def battery_detail():
    status = battery_status()
    charging = status == "Charging"
    return {
        "available": True,
        "capacity": STATE["charge_control"]["capacity"], "status": status,
        "voltage_mv": 4210, "voltage_max_mv": 4500, "voltage_ocv_mv": 4190,
        "current_ma": 1450 if charging else 0, "power_mw": 6105 if charging else 0, "temperature_c": 33.0,
        "charge_type": "Fast" if charging else "None", "health": "Good", "cycle_count": 214,
        "charge_counter_mah": 7800, "charge_full_mah": 9410, "charge_full_design_mah": 10000,
        "time_to_full_secs": 3300 if charging else -1, "time_to_empty_secs": -1,
    }


def thermal_all():
    return {
        "available": True,
        "cpu_0": 61.2, "cpu_1": 60.8, "cpu_2": 59.7, "cpu_3": 60.1,
        "modem": 55.0, "modem_ss0": 52.0, "modem_ss1": 51.0, "modem_ss2": 50.0,
        "battery": 33.0, "usb": 38.0, "eth_phy": 44.0, "pmic": 49.0,
        "xo_therm": 35.0, "pa": 47.0, "sdr": 45.0,
    }


# ── SMS (sms.rs; list rows are zwrt_wms `zte_libwms_get_sms_data` shape) ─────

def ucs2_hex(text):
    """Stock WMS content encoding (sms.rs::encode_message): 4 hex digits per code point."""
    return "".join(f"{ord(c):04X}" for c in text)


def sms_defaults():
    # tag: 0 read, 1 unread, 2 sent. All content synthetic; numbers are from
    # ACMA's range reserved for fiction (0491 570 xxx), so none can be real.
    return {
        "next_id": 3722,
        "messages": [
            {"id": 3721, "number": "+61491570156", "content": "Your usage is at 80% of your plan.",
             "date": "2026-08-08 16:42:11", "tag": 0, "mem_store": 1},
            {"id": 3720, "number": "ExampleCo", "content": "Welcome to 5G. Your plan now includes 5G access.",
             "date": "2026-08-07 09:15:02", "tag": 1, "mem_store": 1},
            {"id": 3719, "number": "+61491570157", "content": "Are you coming over on the weekend?",
             "date": "2026-08-06 19:03:44", "tag": 1, "mem_store": 1},
            {"id": 3718, "number": "+61491570156", "content": "On my way, should be there in 20.",
             "date": "2026-08-06 18:40:12", "tag": 2, "mem_store": 1},
        ],
    }


def post_sms_list(body):
    page, per_page = 0, 500
    if body not in (None, {}):
        req = struct_body(body, "invalid SMS list request", {"page": "u32", "per_page": "u32"},
                          optional=("page", "per_page"))
        page = 0 if req["page"] is None else req["page"]
        per_page = 500 if req["per_page"] is None else req["per_page"]
    if not 1 <= per_page <= 500:
        raise ApiError(400, "per_page must be between 1 and 500")
    rows = sorted(STATE["sms"]["messages"], key=lambda m: m["id"], reverse=True)  # order by id desc
    rows = rows[page * per_page:(page + 1) * per_page]
    return {"messages": [{**m, "content": ucs2_hex(m["content"])} for m in rows]}


def sms_ids(body):
    obj = need_object(body, "invalid SMS ids")
    for key in obj:
        if key != "ids":
            raise ApiError(400, f"invalid SMS ids: unknown field `{key}`, expected `ids`")
    ids = obj.get("ids")
    if "ids" not in obj:
        raise ApiError(400, "invalid SMS ids: missing field `ids`")
    if not isinstance(ids, list) or not all(_is_int(i) and i >= 0 for i in ids):
        raise ApiError(400, "invalid SMS ids: invalid value for field `ids`, expected a list of u64")
    if not ids or len(ids) > 100 or 0 in ids:
        raise ApiError(400, "ids must contain between 1 and 100 positive message identifiers")
    return ids


def post_sms_send(body):
    req = struct_body(body, "invalid SMS", {"number": "str", "message": "str"})
    number, message = req["number"], req["message"]
    if not number or len(number) > 32 or not re.fullmatch(r"[0-9+*#]+", number):
        raise ApiError(400, "recipient must contain only digits, +, * or # and be at most 32 characters")
    if not 1 <= len(message) <= 160 or "\0" in message:
        raise ApiError(400, "message must contain 1 to 160 characters")
    sms = STATE["sms"]
    sms["messages"].append({
        "id": sms["next_id"], "number": number, "content": message,
        "date": time.strftime("%Y-%m-%d %H:%M:%S"), "tag": 2, "mem_store": 1,
    })
    sms["next_id"] += 1
    return {"result": "success"}


def post_sms_delete(body):
    ids = set(sms_ids(body))
    sms = STATE["sms"]
    sms["messages"] = [m for m in sms["messages"] if m["id"] not in ids]
    return {"result": "success"}


def post_sms_read(body):
    ids = set(sms_ids(body))
    for message in STATE["sms"]["messages"]:
        if message["id"] in ids:
            message["tag"] = 0
    return ok_result()


# ── APN (router.rs; zwrt_apn_object) ──────────────────────────────────────────

def apn_defaults():
    return {
        "mode": 1,
        "next_id": 3,
        # isEnable / profileId use the firmware's mixed encodings (agent test
        # `active_apn_profiles_are_detected_across_firmware_types`).
        "profiles": [
            {"profilename": "Example Internet", "wanapn": "internet.example", "username": "", "password": "",
             "pdpType": 3, "pppAuthMode": 0, "profileId": 1, "isEnable": "1"},
            {"profilename": "Example M2M", "wanapn": "m2m.example", "username": "", "password": "",
             "pdpType": 1, "pppAuthMode": 0, "profileId": 2, "isEnable": "0"},
        ],
    }


def get_apn_mode():
    return {"apn_mode": STATE["apn"]["mode"]}


def get_apn_profiles():
    return {"apnListArray": [dict(p) for p in STATE["apn"]["profiles"]]}


def put_apn_mode(body):
    req = struct_body(body, "invalid APN mode", {"apn_mode": "u8"})
    if req["apn_mode"] > 1:
        raise ApiError(400, "apn_mode must be 0 (automatic) or 1 (manual)")
    STATE["apn"]["mode"] = req["apn_mode"]
    return ok_result()


def apn_profile_id(body):
    req = struct_body(body, "invalid profile id", {"profileId": "str"})
    pid = req["profileId"]
    if not pid or len(pid) > 32 or not re.fullmatch(r"[A-Za-z0-9_-]+", pid):
        raise ApiError(400, "profileId must be a firmware profile identifier")
    return pid


def _find_profile(pid):
    return next((p for p in STATE["apn"]["profiles"] if str(p["profileId"]) == pid), None)


def _apn_enabled(profile):
    return str(profile["isEnable"]).lower() in ("1", "true")


def post_apn_add(body):
    req = struct_body(body, "invalid APN profile", {
        "profilename": "str", "wanapn": "str", "username": "str", "password": "str",
        "pdpType": "u8", "pppAuthMode": "u8",
    }, optional=("username", "password"))
    username, password = req["username"] or "", req["password"] or ""
    for field, value, limit, allow_empty in (
        ("profilename", req["profilename"], 64, False), ("wanapn", req["wanapn"], 100, False),
        ("username", username, 128, True), ("password", password, 128, True),
    ):
        if (not allow_empty and not value.strip()):
            raise ApiError(400, f"{field} is required")
        if len(value) > limit or any(ord(c) < 32 or ord(c) == 127 for c in value):
            raise ApiError(400, f"{field} is invalid or longer than {limit} characters")
    if not 1 <= req["pdpType"] <= 3:
        raise ApiError(400, "pdpType must be 1 (IPv4), 2 (IPv6), or 3 (IPv4v6)")
    if req["pppAuthMode"] > 3:
        raise ApiError(400, "pppAuthMode must be between 0 and 3")
    if req["pppAuthMode"] == 0 and (username or password):
        raise ApiError(400, "credentials require PAP, CHAP, or PAP/CHAP authentication")
    apn = STATE["apn"]
    apn["profiles"].append({
        "profilename": req["profilename"], "wanapn": req["wanapn"], "username": username,
        "password": password, "pdpType": req["pdpType"], "pppAuthMode": req["pppAuthMode"],
        "profileId": apn["next_id"], "isEnable": "0",
    })
    apn["next_id"] += 1
    return ok_result()


def post_apn_delete(body):
    pid = apn_profile_id(body)
    profile = _find_profile(pid)
    if profile is not None and _apn_enabled(profile):
        raise ApiError(409, "cannot delete the active APN profile")
    STATE["apn"]["profiles"] = [p for p in STATE["apn"]["profiles"] if str(p["profileId"]) != pid]
    return ok_result()


def post_apn_activate(body):
    """router.rs::router_apn_profiles_activate: manual mode first, then enable the profile."""
    pid = apn_profile_id(body)
    apn = STATE["apn"]
    previous_mode = apn["mode"]
    apn["mode"] = 1
    profile = _find_profile(pid)
    if profile is None:
        if previous_mode == 0:
            apn["mode"] = 0  # the agent restores automatic mode after a failed activation
        raise ApiError(503, f"ubus call failed: no APN profile {pid}")
    for other in apn["profiles"]:
        other["isEnable"] = "0"
    profile["isEnable"] = "1"
    return ok_result()


# ── Cell / band locks (cell.rs; zte_nwinfo_api) ──────────────────────────────

def _parse_band_list(value):
    parts = [p.strip() for p in value.split(",")]
    if not all(re.fullmatch(r"\+?\d+", p) and int(p) <= 255 for p in parts):
        raise ApiError(400, "band list must contain comma-separated numbers")
    bands = [int(p) for p in parts]
    if not bands or len(bands) > len(AGENT_NR_BANDS):
        raise ApiError(400, "select at least one band")
    if len(set(bands)) != len(bands):
        raise ApiError(400, "band list contains duplicates")
    return bands


def post_band_nr(body):
    req = struct_body(body, "invalid NR band selection", {"nr5g_type": "str", "nr5g_band": "str"})
    if req["nr5g_type"] != "SA":
        raise ApiError(400, "this firmware only supports NR band locking in SA mode")
    bands = _parse_band_list(req["nr5g_band"])
    if any(b not in AGENT_NR_BANDS for b in bands):
        raise ApiError(400, "NR selection contains a band not supported by U60 Pro firmware")
    # The firmware reads the SA lock back as the list it was given; the NSA lock
    # is a separate setting this call does not touch.
    STATE["netinfo"]["nr5g_sa_band_lock"] = req["nr5g_band"]
    return ok_result()


def post_band_lte(body):
    req = struct_body(body, "invalid LTE band selection", {
        "is_lte_band": "str", "lte_band_mask": "str", "is_gw_band": "str", "gw_band_mask": "str",
    })
    if req["is_lte_band"] != "1" or req["is_gw_band"] != "0" or req["gw_band_mask"] != "0":
        raise ApiError(400, "LTE band request does not match the firmware contract")
    if not re.fullmatch(r"\+?\d+", req["lte_band_mask"]) or int(req["lte_band_mask"]) >= 1 << 128:
        raise ApiError(400, "lte_band_mask must be a decimal bitmask")
    mask = int(req["lte_band_mask"])
    allowed = 0
    for band in AGENT_LTE_BANDS:
        allowed |= 1 << (band - 1)
    if mask == 0 or mask & ~allowed:
        raise ApiError(400, "LTE mask is empty or contains a band not supported by U60 Pro firmware")
    # Observed read-back: lowercase hex with a 0x prefix (e.g. '0x87e29a0e00df').
    STATE["netinfo"]["lte_band_lock"] = f"0x{mask:x}"
    return ok_result()


def post_band_reset(body):
    # nwinfo_rest_band_rat: restore the all-bands defaults for LTE and NR (SA and NSA).
    # Whether the real call also resets the preferred RAT (net_select) is not verified;
    # the mock leaves net_select alone.
    netinfo = STATE["netinfo"]
    netinfo["lte_band_lock"] = LTE_DEFAULT_LOCK
    netinfo["nr5g_sa_band_lock"] = NR_DEFAULT_LOCK
    netinfo["nr5g_nsa_band_lock"] = NR_DEFAULT_LOCK
    return ok_result()


def _lock_params(body, keys):
    obj = need_object(body, None)
    if any(not isinstance(obj.get(k), (str, int)) or isinstance(obj.get(k), bool) or str(obj.get(k)) == ""
           for k in keys):
        # The firmware (not the agent) rejects a lock request without its parameters.
        raise ApiError(503, "ubus call failed: invalid parameters")
    return [str(obj[k]) for k in keys]


def post_lock_nr(body):
    # Submitting a lock only records it. The modem re-selects asynchronously and
    # may refuse the cell, so the mock never changes the serving cell (PCI, band,
    # EARFCN) here: "lock accepted" and "now camped on that cell" are separate.
    pci, earfcn, band = _lock_params(body, ("lock_nr_pci", "lock_nr_earfcn", "lock_nr_cell_band"))
    STATE["netinfo"]["lock_nr_cell"] = f"{pci},{earfcn},{band}"
    return ok_result()


def post_lock_lte(body):
    pci, earfcn = _lock_params(body, ("lock_lte_pci", "lock_lte_earfcn"))
    STATE["netinfo"]["lock_lte_cell"] = f"{pci},{earfcn}"
    return ok_result()


def post_lock_reset(body):
    STATE["netinfo"]["lock_lte_cell"] = ""
    STATE["netinfo"]["lock_nr_cell"] = ""
    return ok_result()


def get_modem_capabilities():
    return {
        "network_modes": [{"value": v, "label": label} for v, label in NETWORK_MODES],
        "lte_bands": list(AGENT_LTE_BANDS),
        "nr_sa_bands": list(AGENT_NR_BANDS),
        "nr_nsa_band_lock_supported": False,
    }


def put_network_mode(body):
    req = struct_body(body, "invalid network mode", {"net_select": "str"})
    if req["net_select"] not in dict(NETWORK_MODES):
        raise ApiError(400, "network mode is not supported by U60 Pro firmware")
    # Only the preference changes. Re-registration onto another RAT takes the
    # modem a while and is not modelled: the serving cell stays as it was.
    STATE["netinfo"]["net_select"] = req["net_select"]
    return ok_result()


# ── Data usage reset day (handlers.rs::data_usage_reset_day_set) ─────────────

def put_reset_day(body):
    obj = need_object(body, None)
    day = obj.get("reset_day")
    if not (_is_int(day) and day >= 0):
        day = obj.get("clearday")
    day = day if _is_int(day) and day >= 0 else 0
    if not 1 <= day <= 31:
        raise ApiError(400, "reset_day must be between 1 and 31")
    usage = STATE["usage"]
    usage["reset_day"] = day
    usage["reset_enabled"] = 1  # the agent's setter always sends enable: 1
    # The billing cycle moves; the record of the last actual clear does not.
    usage["next_clear_date"] = cycle_dates(day)[1]
    return data_usage_payload()


# ── Router: DNS, LAN (router.rs) ─────────────────────────────────────────────

def dns_defaults():
    # Keys as returned after the agent strips the firmware's `wan_` prefix.
    return {
        "dns_mode": "manual",
        "prefer_dns_manual": "1.1.1.1", "standby_dns_manual": "1.0.0.1",
        "ipv6_wan_prefer_dns_manual": "2606:4700:4700::1111",
        "ipv6_wan_standby_dns_manual": "2606:4700:4700::1001",
    }


def lan_defaults():
    return {
        "ipaddr": "192.168.0.1", "netmask": "255.255.255.0", "dhcp_enabled": True,
        "dhcp_start": "192.168.0.2", "dhcp_end": "192.168.0.253",
        "lease_seconds": 86400,
    }


def put_dns(body):
    obj = need_object(body, None)
    STATE["dns"].update({k: v for k, v in obj.items() if k in STATE["dns"]})
    return ok_result()


def get_lan():
    return {"transition": {"pending": False, "last_error": None, "remaining_secs": None}, **STATE["lan"]}


def put_lan(body):
    """Simplified: the real agent answers 202 with a reconnect token and reverts the
    change unless /api/router/lan/confirm arrives within 120 s. The mock applies the
    change at once and reports `changed: false`, so the dashboard skips the reconnect
    handshake (which cannot reach a different address in a local demo)."""
    obj = need_object(body, "invalid LAN settings")
    expected = ("ipaddr", "netmask", "dhcp_enabled", "dhcp_start", "dhcp_end", "lease_seconds")
    for key in obj:
        if key not in expected:
            raise ApiError(400, f"invalid LAN settings: unknown field `{key}`")
    for key in expected:
        if key not in obj:
            raise ApiError(400, f"invalid LAN settings: missing field `{key}`")
    if not isinstance(obj["dhcp_enabled"], bool) or not _is_int(obj["lease_seconds"]):
        raise ApiError(400, "invalid LAN settings: invalid value")
    if not 60 <= obj["lease_seconds"] <= 604_800:
        raise ApiError(400, "lease_seconds must be between 60 and 604800")
    STATE["lan"].update(obj)
    return Reply(202, {"changed": False})


def post_lan_confirm(body):
    raise ApiError(409, "no LAN change is pending")


# ── TTL (server.rs) ───────────────────────────────────────────────────────────

def put_ttl_set(body):
    obj = need_object(body, None)
    ttl = obj.get("ttl")
    if not (_is_int(ttl) and 1 <= ttl <= 255):
        raise ApiError(400, "ttl must be 1-255")
    STATE["ttl"] = {"active": True, "ipv6_active": True, "ttl_value": ttl}
    return {"ttl": ttl, "ipv4": True, "ipv6": True}


def delete_ttl_clear():
    STATE["ttl"] = {"active": False, "ipv6_active": False, "ttl_value": 0}
    return Reply(200, envelope={"ok": True})


# ── System / AT console ──────────────────────────────────────────────────────

def system_top():
    # Mirrors agent/src/system.rs::ProcessListResult / ProcessEntry exactly.
    procs = [
        {"pid": 487, "name": "zte_topsw_tr069", "cpu_pct": 3.2, "rss_kb": 23300, "state": "sleeping", "is_bloat": True},
        {"pid": 611, "name": "zte_router", "cpu_pct": 2.4, "rss_kb": 9800, "state": "sleeping", "is_bloat": False},
        {"pid": 512, "name": "zte_mqtt_sdk_st", "cpu_pct": 1.8, "rss_kb": 11100, "state": "sleeping", "is_bloat": True},
        {"pid": 534, "name": "zte_topsw_nwinfo", "cpu_pct": 1.1, "rss_kb": 6400, "state": "sleeping", "is_bloat": False},
        {"pid": 702, "name": "hostapd", "cpu_pct": 0.9, "rss_kb": 4100, "state": "sleeping", "is_bloat": False},
        {"pid": 811, "name": "zte-agent", "cpu_pct": 0.6, "rss_kb": 2048, "state": "running", "is_bloat": False},
        {"pid": 850, "name": "uhttpd", "cpu_pct": 0.3, "rss_kb": 1900, "state": "sleeping", "is_bloat": False},
        {"pid": 1, "name": "init", "cpu_pct": 0.1, "rss_kb": 1200, "state": "sleeping", "is_bloat": False},
    ]
    bloat = [p for p in procs if p["is_bloat"]]
    return {
        "processes": procs,
        "total_count": 142,
        "bloat_count": len(bloat),
        "bloat_cpu_pct": round(sum(p["cpu_pct"] for p in bloat), 1),
        "bloat_rss_kb": sum(p["rss_kb"] for p in bloat),
    }


def post_kill_bloat(body):
    obj = need_object(body, None)
    if obj.get("all") is True:
        victims = None
    elif isinstance(obj.get("pids"), list):
        victims = [p for p in obj["pids"] if _is_int(p)]
        if not victims:
            raise ApiError(400, "pids array is empty")
    else:
        raise ApiError(400, "expected 'all' or 'pids'")
    # Fire-and-forget: the process list is not modified.
    killed = [p for p in system_top()["processes"] if p["is_bloat"] and (victims is None or p["pid"] in victims)]
    return {
        "killed": [{"pid": p["pid"], "name": p["name"]} for p in killed],
        "skipped": [],
        "freed_rss_kb": sum(p["rss_kb"] for p in killed),
    }


AT_BLOCKED_PREFIXES = ("AT+CFUN", "AT^", "AT$QCRMCALL", "AT+CLCK", "AT+CMGD", "AT+CMGF=1;+CMGS",
                       "AT+CGDCONT=", "AT+CGACT=")
AT_RESPONSES = {
    "AT": "OK",
    "ATI": "Manufacturer: ZTE\r\nModel: MU5250\r\nOK",
    "AT+CSQ": "+CSQ: 28,99\r\n\r\nOK",
    "AT+CGMI": "ZTE CORPORATION\r\n\r\nOK",
    "AT+CGMM": "MU5250\r\n\r\nOK",
}
AT_ALLOWED = set(AT_RESPONSES) | {
    "AT+COPS?", "AT+COPS=?", "AT+CGDCONT?", "AT+CREG?", "AT+CGREG?", "AT+CEREG?", "AT+CGPADDR",
    "AT+CGACT?", "AT+CLAC", "AT+CGSN", "AT+CGMR", 'AT+QENG="SERVINGCELL"', "AT+QNWINFO", "AT+QRSRP",
    "AT+QRSRQ", "AT+QINISTAT", "AT+QSPN", "AT+QCIDINCOMING", "AT+CGCONTRDP",
}


def post_at_send(body):
    obj = need_object(body, None)
    command = obj.get("command")
    if not isinstance(command, str) or not command:
        raise ApiError(400, "missing 'command'")
    upper = command.strip().upper()
    if upper.startswith(AT_BLOCKED_PREFIXES) or upper not in AT_ALLOWED:
        raise ApiError(403, "command not allowed. Only read-only AT commands are permitted.")
    return {"response": AT_RESPONSES.get(upper, "OK")}


# ── CSV loggers (logging.rs) ─────────────────────────────────────────────────

# Headers mirror agent/src/{signal,connection}_logger.rs::HEADER.
SIGNAL_LOG_CSV = (
    "timestamp,datetime,network_type,carrier,cell_id,lte_band,lte_pci,lte_earfcn,"
    "lte_rsrp,lte_rsrq,lte_sinr,lte_rssi,nr_band,nr_pci,nr_arfcn,nr_rsrp,nr_rsrq,"
    "nr_sinr,nr_rssi,lte_ca_bands,nr_ca_bands\n"
    "1754700000,2026-08-09T09:20:00,ENDC,Telstra,134479973,B8,312,3650,"
    "-71,-10.8,19.5,-62,n78,801,630912,-77,-9.2,23.5,-58,B8+B1,n78+n40\n"
    "1754700003,2026-08-09T09:20:03,ENDC,Telstra,134479973,B8,312,3650,"
    "-72,-10.9,19.1,-63,n78,801,630912,-78,-9.4,22.8,-59,B8+B1,n78+n40\n"
)

CONNECTION_LOG_CSV = (
    "timestamp,datetime,event_type,detail,old_value,new_value\n"
    "1754700012,2026-08-09T09:20:12,nr_band_change,NR band changed,n78,n40\n"
    "1754700190,2026-08-09T09:23:10,cell_handover,cell_id changed,134479973,134479981\n"
)


def logger_defaults():
    return {"signal": {"running": False, "started": None, "duration": 3600, "interval": 3},
            "connection": {"running": False, "started": None, "duration": 3600, "interval": 3}}


def logger_status(name, count_key):
    log = STATE["loggers"][name]
    elapsed = 0
    if log["started"] is not None:
        elapsed = min(int(time.time() - log["started"]), log["duration"])
        if elapsed >= log["duration"]:
            log["running"] = False
    return {
        count_key: elapsed // log["interval"] if log["started"] is not None else 0,
        "running": log["running"], "elapsed_secs": elapsed,
        "duration_secs": log["duration"], "interval_secs": log["interval"], "last_error": None,
        "max_bytes": 5 * 1024 * 1024, "flush_interval_secs": 30,
    }


def logger_start(name):
    def handler(body):
        req = struct_body(body, "", {"duration_secs": "u64", "interval_secs": "u64"},
                          optional=("duration_secs", "interval_secs"))
        duration = 3600 if req["duration_secs"] is None else req["duration_secs"]
        interval = 3 if req["interval_secs"] is None else req["interval_secs"]
        if not 1 <= duration <= 86400 or not 1 <= interval <= 60:
            raise ApiError(400, "duration must be 1–86400 seconds and interval 1–60 seconds")
        log = STATE["loggers"][name]
        logger_status(name, "x")  # settle a finished run
        if log["running"]:
            raise ApiError(409, "logger already running")
        log.update(running=True, started=time.time(), duration=duration, interval=interval)
        return {"duration_secs": duration, "interval_secs": interval}
    return handler


def logger_stop(name):
    def handler(body):
        STATE["loggers"][name]["running"] = False
        return Reply(200, envelope={"ok": True})
    return handler


def csv_download(name, text):
    def handler():
        return Reply(200, raw=("text/csv; charset=utf-8", text.encode()),
                     headers={"Content-Disposition": f'attachment; filename="{name}"',
                              "Cache-Control": "no-store"})
    return handler


# ── Mutable demo state ────────────────────────────────────────────────────────

STATE = {}


def initial_state(scenario):
    return {
        "scenario": scenario,
        "netinfo": netinfo_for(scenario),
        "usage": usage_defaults(),
        "charge_control": charge_control_defaults(),
        "charger": {"otg_powerbank_state": 0, "direct_power_supply_mode": "disable"},
        "ttl": {"active": False, "ipv6_active": False, "ttl_value": 0},
        "wifi": wifi_defaults(),
        "usb": usb_defaults(),
        "usb_pending": None,
        "dns": dns_defaults(),
        "lan": lan_defaults(),
        "apn": apn_defaults(),
        "sms": sms_defaults(),
        "loggers": logger_defaults(),
    }


def reset_state(scenario=None):
    """Restore the initial state (and clear the request log)."""
    scenario = scenario or CONFIG["scenario"]
    new = initial_state(scenario)  # validates the scenario before touching anything
    CONFIG["scenario"] = scenario
    STATE.clear()
    STATE.update(new)
    del REQUESTS[:]


reset_state()


def post_login(body):
    obj = need_object(body, None)
    if "password" not in obj and "pin" not in obj:
        raise ApiError(400, "missing 'password' or 'pin' field")
    return {"token": "demo-token"}


# ── Route tables (scripts/check-api-contract.py parses these; keep the layout) ─

ROUTES_PUT = {
    "/api/device/charge-control": put_charge_control,
    "/api/wifi/settings": put_wifi_settings,
    "/api/data-usage/reset-day": put_reset_day,
    "/api/modem/network-mode": put_network_mode,
    "/api/usb/mode": put_usb_mode,
    "/api/usb/default": put_usb_default,
    "/api/usb/powerbank": put_powerbank,
    "/api/ttl/set": put_ttl_set,
    "/api/router/dns": put_dns,
    "/api/router/lan": put_lan,
    "/api/router/apn/mode": put_apn_mode,
}


ROUTES_GET = {
    "/api/dashboard": dashboard_batch,
    "/api/network/clients": clients,
    "/api/device": lambda: dashboard_batch()["device"],
    "/api/cpu": lambda: dashboard_batch()["cpu"],
    "/api/memory": lambda: dashboard_batch()["memory"],
    "/api/wifi/status": lambda: STATE["wifi"],
    "/api/usb/status": get_usb_status,
    "/api/device/charger": get_charger,
    "/api/device/charge-control": get_charge_control,
    "/api/device/thermal/all": thermal_all,
    "/api/device/battery/detail": battery_detail,
    "/api/device/battery-info": lambda: {
        "available": True, "online": True, "low_power": False, "using_hw_fg_chip": True,
        "time_to_full_mins": 55, "time_to_empty_mins": 0,
    },
    "/api/modem/capabilities": get_modem_capabilities,
    "/api/sms/capabilities": lambda: {
        "available": True, "ready": True, "object": "zwrt_wms", "storage": "native",
    },
    "/api/router/dns": lambda: STATE["dns"],
    "/api/router/lan": get_lan,
    "/api/router/apn/mode": get_apn_mode,
    "/api/router/apn/profiles": get_apn_profiles,
    "/api/sim/info": lambda: {
        "sim_iccid": "", "sim_imsi": "",
        "sim_states": "SIM_READY", "mdm_mcc": "505", "mdm_mnc": "01",
    },
    "/api/sim/imei": lambda: {"imei": ""},
    "/api/ttl/status": lambda: STATE["ttl"],
    "/api/system/top": system_top,
    "/api/logger/signal/status": lambda: logger_status("signal", "samples"),
    "/api/logger/connection/status": lambda: logger_status("connection", "events"),
    "/api/logger/signal/download": csv_download("signal_log.csv", SIGNAL_LOG_CSV),
    "/api/logger/connection/download": csv_download("connection_log.csv", CONNECTION_LOG_CSV),
    "/api/at/port": lambda: {"port": "/dev/at_mdm0", "available": True},
}

ROUTES_POST = {
    "/api/auth/login": post_login,
    "/api/sms/list": post_sms_list,
    "/api/sms/send": post_sms_send,
    "/api/sms/delete": post_sms_delete,
    "/api/sms/read": post_sms_read,
    "/api/cell/lock/nr": post_lock_nr,
    "/api/cell/lock/lte": post_lock_lte,
    "/api/cell/lock/reset": post_lock_reset,
    "/api/cell/band/nr": post_band_nr,
    "/api/cell/band/lte": post_band_lte,
    "/api/cell/band/reset": post_band_reset,
    "/api/router/lan/confirm": post_lan_confirm,
    "/api/router/apn/profiles": post_apn_add,
    "/api/router/apn/profiles/delete": post_apn_delete,
    "/api/router/apn/profiles/activate": post_apn_activate,
    "/api/system/kill-bloat": post_kill_bloat,
    "/api/system/restart-agent": lambda body: Reply(200, envelope={"ok": True, "message": "Agent restarting in ~2 seconds"}),
    "/api/device/reboot": lambda body: {},
    "/api/device/shutdown": lambda body: {},
    "/api/at/send": post_at_send,
    "/api/logger/signal/start": logger_start("signal"),
    "/api/logger/signal/stop": logger_stop("signal"),
    "/api/logger/connection/start": logger_start("connection"),
    "/api/logger/connection/stop": logger_stop("connection"),
}

# agent/src/server.rs::DESTRUCTIVE_PATHS: rejected without `X-Confirm: true`.
DESTRUCTIVE_PATHS = ("/api/device/reboot", "/api/device/shutdown", "/api/system/kill-bloat")
# POSTs that only read; not recorded as mutations.
READ_ONLY_POSTS = ("/api/sms/list", "/api/auth/login")


class Handler(BaseHTTPRequestHandler):
    def _cors(self):
        origin = self.headers.get("Origin", "*")
        self.send_header("Access-Control-Allow-Origin", origin)
        self.send_header("Access-Control-Allow-Methods", "GET, POST, PUT, DELETE, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Authorization, Content-Type, X-Confirm")
        self.send_header("Access-Control-Max-Age", "86400")

    def _write(self, status, content_type, payload, extra=None):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        for name, value in (extra or {}).items():
            self.send_header(name, value)
        self._cors()
        self.send_header("Content-Length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def _send(self, payload, status=200):
        self._write(status, "application/json", json.dumps(payload).encode())

    def _read_body(self):
        """(parsed JSON or None, raw text). None = empty or not valid JSON."""
        length = int(self.headers.get("Content-Length") or 0)
        if length == 0:
            return None, ""
        raw = self.rfile.read(length).decode("utf-8", "replace")
        try:
            return json.loads(raw), raw
        except ValueError:
            return None, raw

    def _path(self):
        return self.path.split("?")[0]

    def _run(self, method, path, handler):
        """Run a route handler under the state lock and send its reply."""
        body, raw = self._read_body()
        mutating = method != "GET" and not (method == "POST" and path in READ_ONLY_POSTS)
        with LOCK:
            try:
                if handler is None:
                    raise ApiError(404, "not found")
                if path in DESTRUCTIVE_PATHS and self.headers.get("X-Confirm") != "true":
                    raise ApiError(400, "destructive action requires X-Confirm: true header")
                if method == "GET":
                    result = handler()
                elif method == "DELETE":
                    result = handler()
                else:
                    result = handler(body)
                reply = result if isinstance(result, Reply) else Reply(200, result)
            except ApiError as error:
                reply = Reply(error.status, envelope={"ok": False, "error": error.message})
            if mutating and len(REQUESTS) < MAX_RECORDED:
                REQUESTS.append({
                    "seq": len(REQUESTS) + 1, "method": method, "path": path,
                    "body": body if body is not None else (raw or None),
                    "status": reply.status,
                })
        if reply.raw is not None:
            return self._write(reply.status, reply.raw[0], reply.raw[1], reply.headers)
        envelope = reply.envelope if reply.envelope is not None else {"ok": True, "data": reply.data}
        return self._send(envelope, reply.status)

    def _mock_control(self, method, path):
        """Test hooks under /__mock/ (never part of the agent contract)."""
        if method == "GET" and path == "/__mock/requests":
            with LOCK:
                return self._send({"ok": True, "data": {"requests": list(REQUESTS)}})
        if method == "POST" and path == "/__mock/reset":
            body, _ = self._read_body()
            scenario = body.get("scenario") if isinstance(body, dict) else None
            try:
                with LOCK:
                    reset_state(scenario)
            except ValueError as error:
                return self._send({"ok": False, "error": str(error)}, 400)
            return self._send({"ok": True, "data": {"scenario": CONFIG["scenario"]}})
        return self._send({"ok": False, "error": "not found"}, 404)

    def do_OPTIONS(self):
        self.send_response(204)
        self._cors()
        self.end_headers()

    def do_GET(self):
        path = self._path()
        if path.startswith("/__mock/"):
            return self._mock_control("GET", path)
        return self._run("GET", path, ROUTES_GET.get(path))

    def do_POST(self):
        path = self._path()
        if path.startswith("/__mock/"):
            return self._mock_control("POST", path)
        return self._run("POST", path, ROUTES_POST.get(path))

    def do_PUT(self):
        path = self._path()
        return self._run("PUT", path, ROUTES_PUT.get(path))

    def do_DELETE(self):
        path = self._path()
        if path == "/api/ttl/clear":
            return self._run("DELETE", path, delete_ttl_clear)
        return self._run("DELETE", path, None)

    def log_message(self, fmt, *args):  # quiet
        pass


class MockServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def server_bind(self):
        # HTTPServer.server_bind() calls socket.getfqdn() on the bind address, a
        # reverse-DNS lookup that can stall for tens of seconds on a offline laptop.
        socketserver.TCPServer.server_bind(self)
        host, port = self.server_address[:2]
        self.server_name = str(host)
        self.server_port = port


def make_server(host="127.0.0.1", port=0):
    """Bind a mock agent (port 0 = ephemeral). Caller runs serve_forever()."""
    return MockServer((host, port), Handler)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=9090)
    ap.add_argument("--host", default="0.0.0.0")
    ap.add_argument("--scenario", choices=SCENARIOS, default=None,
                    help="radio scenario (default: $MOCK_SCENARIO or SA)")
    args = ap.parse_args()
    reset_state(args.scenario)
    srv = make_server(args.host, args.port)
    print(f"mock zte-agent ({CONFIG['scenario']}) listening on http://localhost:{args.port}")
    srv.serve_forever()


if __name__ == "__main__":
    main()
