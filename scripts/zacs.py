#!/usr/bin/env python3
"""zacs.py — minimal CWMP (TR-069) ACS for rogue-ACS enumeration of the MU5250.

Usage:
  python3 scripts/zacs.py [port] [bind]           # serve (default 7547, 0.0.0.0)
  python3 scripts/zacs.py cmd 'GPN Device. 1'     # queue an RPC from another shell

Queue file:  logs/zacs/queue.txt   (one RPC per line)
             GPN <path> <0|1>      GetParameterNames (1=NextLevel)
             GPV <name[,name...]>  GetParameterValues
             SPV <name=value>      SetParameterValues (USE WITH CARE)
             X <verbatim-soap-body-inside-cwmp:Body>
Results:     logs/zacs/results.txt (appended); raw XML in logs/zacs/raw/
"""
import sys, os, re, time, socket, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOGDIR = os.path.join(os.path.dirname(__file__), "..", "logs", "zacs")
os.makedirs(LOGDIR, exist_ok=True)
QUEUE = os.path.join(LOGDIR, "queue.txt")
RESULTS = os.path.join(LOGDIR, "results.txt")
RAW = os.path.join(LOGDIR, "raw")
os.makedirs(RAW, exist_ok=True)
_lock = threading.Lock()

def log(msg):
    line = f"[{time.strftime('%H:%M:%S')}] {msg}"
    with _lock:
        with open(RESULTS, "a") as f:
            f.write(line + "\n")
    print(line, flush=True)

def queue_rpc(cmd):
    with _lock:
        with open(QUEUE, "a") as f:
            f.write(cmd.strip() + "\n")

def pop_rpc():
    with _lock:
        if not os.path.exists(QUEUE):
            return None
        lines = open(QUEUE).read().splitlines()
        lines = [l for l in lines if l.strip()]
        if not lines:
            return None
        open(QUEUE, "w").write("\n".join(lines[1:]) + ("\n" if len(lines) > 1 else ""))
        return lines[0]

def esc(s):
    return s.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")

def build_envelope(ns, body, rpc_id="zacs1"):
    return (f'<?xml version="1.0" encoding="UTF-8"?>'
            f'<soap-env:Envelope xmlns:soap-env="http://schemas.xmlsoap.org/soap/envelope/" '
            f'xmlns:soap-enc="http://schemas.xmlsoap.org/soap/encoding/" '
            f'xmlns:xsd="http://www.w3.org/2001/XMLSchema" '
            f'xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" '
            f'xmlns:cwmp="{ns}">'
            f'<soap-env:Header><cwmp:ID soap-env:mustUnderstand="1">{rpc_id}</cwmp:ID></soap-env:Header>'
            f'<soap-env:Body>{body}</soap-env:Body></soap-env:Envelope>')

def cmd_to_body(cmd):
    if cmd.startswith("GPN "):
        _, path, nl = cmd.split(None, 2)
        return (f'<cwmp:GetParameterNames><ParameterPath>{esc(path)}</ParameterPath>'
                f'<NextLevel>{nl.strip()}</NextLevel></cwmp:GetParameterNames>')
    if cmd.startswith("GPV "):
        names = cmd[4:].split(",")
        arr = "".join(f"<string>{esc(n.strip())}</string>" for n in names)
        return (f'<cwmp:GetParameterValues><ParameterNames soap-enc:arrayType="xsd:string[{len(names)}]">'
                f'{arr}</ParameterNames></cwmp:GetParameterValues>')
    if cmd.startswith("SPV "):
        kv = cmd[4:]
        name, _, value = kv.partition("=")
        return (f'<cwmp:SetParameterValues><ParameterList soap-enc:arrayType="cwmp:ParameterValueStruct[1]">'
                f'<ParameterValueStruct><Name>{esc(name.strip())}</Name>'
                f'<Value xsi:type="xsd:string">{esc(value.strip())}</Value></ParameterValueStruct>'
                f'</ParameterList><ParameterKey>zacs</ParameterKey></cwmp:SetParameterValues>')
    if cmd.startswith("X "):
        return cmd[2:]
    return None

def read_body(handler):
    te = handler.headers.get("Transfer-Encoding", "")
    if "chunked" in te.lower():
        data = b""
        while True:
            line = handler.rfile.readline().strip()
            try:
                n = int(line, 16)
            except ValueError:
                break
            if n == 0:
                handler.rfile.readline()
                break
            data += handler.rfile.read(n)
            handler.rfile.readline()
        return data
    n = int(handler.headers.get("Content-Length") or 0)
    return handler.rfile.read(n) if n else b""

class ACS(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def _send(self, code, body=b"", ctype="text/xml; charset=utf-8"):
        self.send_response(code)
        if body:
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
        else:
            self.send_header("Content-Length", "0")
        self.end_headers()
        if body:
            self.wfile.write(body)

    def do_GET(self):
        self._send(200, b"zacs up\n", "text/plain")

    def do_POST(self):
        body = read_body(self)
        ts = time.strftime("%H%M%S")
        if body:
            with open(os.path.join(RAW, f"in-{ts}.xml"), "wb") as f:
                f.write(body)
        text = body.decode("utf-8", "replace")
        m = re.search(r'xmlns:cwmp="([^"]+)"', text)
        ns = m.group(1) if m else "urn:dslforum-org:cwmp-1-0"

        if "<cwmp:Inform>" in text or ":Inform>" in text and "InformResponse" not in text:
            dev = re.search(r"<Manufacturer>(.*?)</Manufacturer>.*?<OUI>(.*?)</OUI>.*?<ProductClass>(.*?)</ProductClass>.*?<SerialNumber>(.*?)</SerialNumber>", text, re.S)
            events = re.findall(r"<EventCode>(.*?)</EventCode>", text)
            params = re.findall(r"<Name>(.*?)</Name>\s*<Value[^>]*>(.*?)</Value>", text, re.S)
            log(f"INFORM device={dev.groups() if dev else '?'} events={events}")
            for name, val in params:
                log(f"  inform param {name} = {val[:100]}")
            self._send(200, build_envelope(ns, "<cwmp:InformResponse><MaxEnvelopes>1</MaxEnvelopes></cwmp:InformResponse>").encode())
            return

        if "GetParameterNamesResponse" in text:
            entries = re.findall(r"<Name>(.*?)</Name>\s*<Writable>(\d)</Writable>", text)
            log(f"GPN response: {len(entries)} names")
            for name, w in entries:
                log(f"  {'W' if w == '1' else 'r'} {name}")
            self._respond_next(ns)
            return

        if "GetParameterValuesResponse" in text:
            entries = re.findall(r"<Name>(.*?)</Name>\s*<Value[^>]*>(.*?)</Value>", text, re.S)
            log(f"GPV response: {len(entries)} values")
            for name, val in entries:
                log(f"  {name} = {val[:200]}")
            self._respond_next(ns)
            return

        if "SetParameterValuesResponse" in text:
            log("SPV response: " + text[:400])
            self._respond_next(ns)
            return

        if "Fault" in text:
            log("FAULT: " + re.sub(r"\s+", " ", text)[:400])
            self._respond_next(ns)
            return

        if not body:
            self._respond_next(ns)
            return

        log("UNHANDLED POST: " + text[:300])
        self._respond_next(ns)

    def _respond_next(self, ns):
        cmd = pop_rpc()
        if cmd:
            body = cmd_to_body(cmd)
            if body:
                log(f"-> issuing: {cmd}")
                self._send(200, build_envelope(ns, body).encode())
                return
            log(f"bad queued cmd ignored: {cmd}")
        self._send(204)

if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "cmd":
        queue_rpc(" ".join(sys.argv[2:]))
        print("queued")
        sys.exit(0)
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 7547
    bind = sys.argv[2] if len(sys.argv) > 2 else "0.0.0.0"
    srv = ThreadingHTTPServer((bind, port), ACS)
    srv.timeout = 1
    log(f"zacs listening on {bind}:{port} (results: {RESULTS})")
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
