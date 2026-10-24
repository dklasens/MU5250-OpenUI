#!/usr/bin/env python3
"""Deploy locally prepared components through a verified management transport.

Credentials are rendered before writes. A transaction snapshots the previous
installation and remains available for explicit recovery after success.
"""
import argparse
import getpass
import gzip
import ipaddress
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import selectors
import subprocess
import sys
import tarfile
import tempfile
import time
import uuid
from device_identity import device_identity

ROOT = Path(__file__).resolve().parents[1]
q = __import__('shlex').quote


def run(args, data=None, timeout=60, raw=False):
    process = subprocess.Popen(args, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, start_new_session=True)
    deadline = time.monotonic() + timeout
    output, errors = bytearray(), bytearray()
    remaining = memoryview(data or b'')
    selector = selectors.DefaultSelector()
    for pipe, events in [(process.stdout, selectors.EVENT_READ), (process.stderr, selectors.EVENT_READ)]:
        os.set_blocking(pipe.fileno(), False)
        selector.register(pipe, events)
    if remaining:
        os.set_blocking(process.stdin.fileno(), False)
        selector.register(process.stdin, selectors.EVENT_WRITE)
    else:
        process.stdin.close()
    try:
        while selector.get_map() or process.poll() is None:
            if time.monotonic() >= deadline: raise RuntimeError('Management operation timed out')
            for key, _ in selector.select(min(0.02, max(0, deadline - time.monotonic()))):
                pipe = key.fileobj
                if pipe is process.stdin:
                    try:
                        count = os.write(pipe.fileno(), remaining[:65536])
                        remaining = remaining[count:]
                    except BrokenPipeError:
                        remaining = memoryview(b'')
                    except BlockingIOError:
                        continue
                    if not remaining:
                        selector.unregister(pipe); pipe.close()
                else:
                    try: chunk = os.read(pipe.fileno(), 65536)
                    except BlockingIOError: continue
                    if not chunk:
                        selector.unregister(pipe); pipe.close()
                    else:
                        destination = output if pipe is process.stdout else errors
                        if len(destination) + len(chunk) > 2 * 1024 * 1024:
                            raise RuntimeError('Unexpectedly large management response')
                        destination.extend(chunk)
        if raw:
            return process.returncode, output.decode(errors='replace'), errors.decode(errors='replace')
        if process.returncode:
            # Commands and output can contain credentials. Never print them.
            raise RuntimeError('Management operation failed')
        return output.decode('utf-8', errors='replace').strip()
    except BaseException:
        try: os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError: pass
        try: process.wait(timeout=0.5)
        except subprocess.TimeoutExpired: pass
        raise
    finally:
        selector.close()
        for pipe in [process.stdin, process.stdout, process.stderr]:
            if not pipe.closed: pipe.close()


class Transport:
    def __init__(self, gateway, adb_serial=None):
        self.gateway = gateway
        self.serial = adb_serial
        if adb_serial:
            self.command = ['adb', '-s', adb_serial, 'shell']
        else:
            known = Path.home() / '.ssh/known_hosts.d/zte'
            known.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            self.command = ['ssh', '-p', '2222', '-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5',
                            '-o', 'ServerAliveInterval=5', '-o', 'ServerAliveCountMax=2',
                            '-o', 'StrictHostKeyChecking=accept-new', '-o', f'UserKnownHostsFile={known}', f'root@{gateway}']

    def shell(self, command, data=None):
        if self.serial and data is not None:
            # Legacy adbd cannot signal stdin EOF reliably. Transfer input via
            # the sync protocol, then let the device shell read a regular file.
            source = '/data/local/tmp/open-u60-input-' + uuid.uuid4().hex
            self.push(data, source)
            try:
                return self.shell(f'({command}) < {q(source)}')
            finally:
                self.shell(f'rm -f {q(source)}')
        marker = '__U60_RESULT__'
        output = run([*self.command, f"({command}); code=$?; printf '\\n{marker}%s\\n' \"$code\""], data)
        body, separator, status = output.rpartition(marker)
        if not separator or status.strip() != '0':
            raise RuntimeError('The modem rejected a deployment operation')
        return body.strip()

    def push(self, data, destination, executable=False):
        staged = destination + '.new'
        if self.serial:
            with tempfile.NamedTemporaryFile(prefix='open-u60-transfer-') as source:
                source.write(data)
                source.flush()
                run(['adb', '-s', self.serial, 'push', source.name, staged])
        else:
            self.shell(f'umask 077; cat > {q(staged)}', data)
        checksum = hashlib.sha256(data).hexdigest()
        self.shell(f"set -e; test \"$(sha256sum {q(staged)} | awk '{{print $1}}')\" = {checksum}; "
                   f"chmod {'700' if executable else '600'} {q(staged)}; mv -f {q(staged)} {q(destination)}")

    def identity(self):
        output = self.shell("set -e; uname -m; id -u; ubus call zwrt_web device_info '{}'", None)
        arch, uid, info = output.split('\n', 2)
        if arch.strip() != 'aarch64' or uid.strip() != '0':
            raise RuntimeError('Expected a root shell on the aarch64 U60 Pro')
        return device_identity(json.loads(info))

    def api(self, path, body=None, token=None, mobile=False):
        command = ['/usr/bin/curl', '--fail', '--silent', '--show-error', '--connect-timeout', '5', '--max-time', '10']
        if body is not None:
            command += ['-H', 'Content-Type: application/json', '--data-binary', '@-']
        if token: command += ['-H', f'Authorization: Bearer {token}']
        if mobile: command += ['-A', 'Mozilla/5.0 (iPhone) Mobile/15E148']
        address = self.shell('uci -q get zwrt_router.network.lan_ipaddr')
        ip = ipaddress.IPv4Address(address)
        if not any(ip in ipaddress.IPv4Network(network) for network in ['10.0.0.0/8', '172.16.0.0/12', '192.168.0.0/16']):
            raise ValueError('Firmware LAN address is not private IPv4')
        command += [f'http://{address}:9090{path}']
        response = json.loads(self.shell(' '.join(map(q, command)), json.dumps(body).encode() if body is not None else None))
        if not response.get('ok'): raise RuntimeError('Agent verification failed')
        return response.get('data', {})


# Must match HELPER_PROTOCOL in agent/src/esim.rs and PROTOCOL in esim-helper/src/main.rs.
ESIM_HELPER_PROTOCOL = 2


def aarch64_executable(data, what):
    if len(data) < 64 or data[:6] != b'\x7fELF\x02\x01' or int.from_bytes(data[18:20], 'little') != 183:
        raise ValueError(f'{what} must be an aarch64 ELF64 little-endian executable')
    return data


def verify_esim_helper(transport):
    # The version request touches no card; it proves the helper runs and speaks the agent's protocol.
    answer = json.loads(transport.shell(f"printf '%s\\n' '{{\"protocol\":{ESIM_HELPER_PROTOCOL},\"op\":\"version\"}}' | /data/zte-esim"))
    if not answer.get('ok') or answer.get('data', {}).get('protocol') != ESIM_HELPER_PROTOCOL:
        raise RuntimeError('The eSIM helper did not answer its version request')


def verify_credentials(transport, password, pin):
    token = transport.api('/api/auth/login', {'password': password})['token']
    device = transport.api('/api/device', token=token)
    if device.get('auth', {}).get('pin_enabled') is not bool(pin):
        raise RuntimeError('Agent PIN state does not match the requested configuration')
    if pin:
        token = transport.api('/api/auth/login', {'pin': pin}, mobile=True)['token']
        transport.api('/api/device', token=token)


class LimitedReader:
    def __init__(self, reader, limit): self.reader, self.remaining = reader, limit
    def read(self, size=-1):
        data = self.reader.read(min(size, self.remaining + 1) if size >= 0 else self.remaining + 1)
        self.remaining -= len(data)
        if self.remaining < 0: raise ValueError('Archive stream exceeds expansion limit')
        return data


def read_limited(path):
    with Path(path).open('rb') as file:
        return LimitedReader(file, 64 * 1024 * 1024).read()


def validate_dashboard(path):
    total, names = 0, set()
    with gzip.open(path, 'rb') as decoded, tarfile.open(fileobj=LimitedReader(decoded, 65 * 1024 * 1024), mode='r|') as archive:
        for entry in archive:
            normalized = Path(entry.name)
            if normalized.is_absolute() or '..' in normalized.parts or '\\' in entry.name or not (entry.isfile() or entry.isdir()):
                raise ValueError('Dashboard archive contains an unsafe path or file type')
            name = normalized.as_posix()
            if name in names or len(names) >= 10000: raise ValueError('Duplicate or excessive dashboard entries')
            names.add(name)
            total += entry.size
            if entry.size > 16 * 1024 * 1024 or total > 64 * 1024 * 1024: raise ValueError('Dashboard archive exceeds size limits')
    if 'index.html' not in names: raise ValueError('Dashboard index.html missing')
    return total


PACKAGES = {
    'dropbear': ('https://downloads.openwrt.org/releases/23.05.4/targets/armsr/armv8/packages/dropbear_2022.82-6_aarch64_generic.ipk', '4fadd1b8529f22fb5d64ee27159d11f4feb68224657953d298a1acf85a83a5c0'),
    'uhttpd': ('https://downloads.openwrt.org/releases/23.05.4/packages/aarch64_generic/base/uhttpd_2023-06-25-34a8a74d-2_aarch64_generic.ipk', 'bd3f010e71a5ea2ef6405e44dbe8c9e697454ce954c197f177ff0c13b9cf5991'),
}
def fetch(url, path):
    run(['curl', '--fail', '--silent', '--show-error', '--location', '--proto', '=https',
         '--connect-timeout', '15', '--max-time', '180', '--max-filesize', str(64 * 1024 * 1024), url, '-o', str(path)], timeout=185)
    if path.stat().st_size > 64 * 1024 * 1024: raise ValueError('Download exceeds 64 MiB')
    return path.read_bytes()


def package_file(data, wanted):
    import io
    with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as archive:
        entries = [e for e in archive if Path(e.name).as_posix() == wanted]
        if len(entries) != 1 or not entries[0].isfile() or entries[0].size > 8 * 1024 * 1024:
            raise ValueError(f'Invalid package entry: {wanted}')
        return archive.extractfile(entries[0]).read()


def prepare_hardening(bundle=None, destination=None):
    programs = {}
    with tempfile.TemporaryDirectory() as temporary:
        for name, (url, checksum) in PACKAGES.items():
            print(f'Preparing verified {name} package…')
            data = read_limited(Path(bundle) / f'{name}.ipk') if bundle else fetch(url, Path(temporary) / f'{name}.ipk')
            if hashlib.sha256(data).hexdigest() != checksum: raise ValueError(f'{name} package integrity check failed')
            if destination:
                Path(destination).mkdir(parents=True, exist_ok=True, mode=0o700)
                (Path(destination) / f'{name}.ipk').write_bytes(data)
            payload = package_file(data, 'data.tar.gz')
            if name == 'dropbear':
                executable = package_file(payload, 'usr/sbin/dropbear')
                programs.update({key: executable for key in ['dropbear', 'dbclient', 'dropbearkey']})
            else: programs['dashboard-uhttpd'] = package_file(payload, 'usr/sbin/uhttpd')
    return programs


def harden(transport, programs, identity):
    key = Path.home() / '.ssh/id_ed25519'
    public = key.with_name(key.name + '.pub')
    key.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    if not key.exists() and not public.exists(): run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-f', str(key)])
    if not key.exists() or not public.exists(): raise RuntimeError('The local SSH key pair is incomplete')
    public_key = public.read_text().strip()
    if '\n' in public_key or not public_key.startswith('ssh-ed25519 '): raise ValueError('Invalid SSH public key')
    transport.shell('mkdir -p /data/bin /data/dropbear /etc/dropbear; chmod 700 /etc/dropbear /data/dropbear')
    for name, data in programs.items(): transport.push(data, f'/data/bin/{name}', True)
    transport.shell(f"set -e; (grep -qFx {q(public_key)} /etc/dropbear/authorized_keys 2>/dev/null || printf '%s\\n' {q(public_key)} >> /etc/dropbear/authorized_keys); chmod 600 /etc/dropbear/authorized_keys; "
                    "for k in ed25519 rsa; do f=/etc/dropbear/dropbear_${k}_host_key; [ -s \"$f\" ] || /data/bin/dropbearkey -t $k -f \"$f\" >/dev/null; done; "
                    "cp /etc/dropbear/authorized_keys /etc/dropbear/dropbear_*_host_key /data/dropbear/; chmod 600 /data/dropbear/*")
    for source, target in [('start-dropbear.sh', 'start_dropbear.sh'), ('start-dashboard.sh', 'start_dashboard.sh'), ('update-rc-local.sh', 'open-u60-rc-update.sh')]:
        transport.push((ROOT / 'scripts/device' / source).read_bytes(), f'/data/local/tmp/{target}', True)
    transport.shell("set -e; mkdir -p /data/www; if uci -q get uhttpd.dashboard >/dev/null; then uci -q delete uhttpd.dashboard; uci commit uhttpd; /etc/init.d/uhttpd restart; fi; "
                    "sh /data/local/tmp/open-u60-rc-update.sh --remove-debug 'sh /data/local/tmp/start_dropbear.sh' 'sh /data/local/tmp/start_dashboard.sh'")
    transport.shell("set -e; ubus call zwrt_zte_dm set_update_mode '{\"dm_update_mode\":\"0\"}'; test \"$(uci get zwrt_zte_dm.dm_update.dm_update_mode)\" = 0")
    transport.shell('sh /data/local/tmp/start_dropbear.sh; sh /data/local/tmp/start_dashboard.sh')
    transport.shell("set -e; root=/data/www; if [ -L /data/www.current ]; then root=$(readlink -f /data/www.current); fi; printf READY > \"$root/.installer-health\"; chmod 644 \"$root/.installer-health\"; test \"$(/usr/bin/curl --fail --silent --show-error --connect-timeout 5 --max-time 10 http://127.0.0.1:8080/.installer-health)\" = READY; rm -f \"$root/.installer-health\"")
    time.sleep(1)
    ssh = Transport(transport.gateway)
    ssh.command[1:1] = ['-i', str(key), '-o', 'IdentitiesOnly=yes']
    if ssh.identity() != identity: raise RuntimeError('SSH reached a different device')
    command = ssh.command[:1] + ['-vv', '-o', 'PubkeyAuthentication=no', '-o', 'NumberOfPasswordPrompts=0'] + ssh.command[1:] + ['true']
    status, _, diagnostic = run(command, timeout=15, raw=True)
    offered = re.findall(r'Authentications that can continue:\s*([^\r\n]+)', diagnostic)
    if status == 0 or not offered or any(method in ['password', 'keyboard-interactive'] for methods in offered for method in methods.split(',')):
        raise RuntimeError('SSH key-only authentication could not be verified')
    print('Key-only SSH verified; firmware auto-update is off.')


def prepare_only(args):
    if not args.bundle_output: raise ValueError('--bundle-output is required for preparation')
    prepare_hardening(args.bundle, args.bundle_output)
    if args.release_agent:
        wanted = {'zte-agent': args.release_agent}
        if args.release_esim_helper: wanted['zte-esim'] = args.release_esim_helper
        with tempfile.TemporaryDirectory() as work:
            root = Path(work)
            if args.bundle:
                sums = (Path(args.bundle) / 'sha256sums.txt').read_text()
                binaries = {name: read_limited(Path(args.bundle) / name) for name in wanted}
            else:
                release = json.loads(fetch('https://api.github.com/repos/dklasens/MU5250-OpenUI/releases/latest', root / 'release.json'))
                urls = {item['name']: item['browser_download_url'] for item in release['assets']}
                sums = fetch(urls['sha256sums.txt'], root / 'sha256sums.txt').decode()
                binaries = {name: fetch(urls[name], root / name) for name in wanted}
            checksums = {parts[1].lstrip('*'): parts[0] for line in sums.splitlines() if len(parts := line.split()) == 2}
            for name, binary in binaries.items():
                if hashlib.sha256(binary).hexdigest() != checksums.get(name): raise ValueError(f'{name} release checksum mismatch')
                destination = Path(wanted[name]); destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(binary)
            print('Agent release checksums verified before unlock.')
    print('All deployment dependencies prepared.')


def deploy(args):
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9.-]{0,252}', args.gateway): raise ValueError('Invalid gateway')
    if not args.agent and not args.dashboard and not args.harden: raise ValueError('Select at least one component')
    # Finish all local preparation before contacting or mutating a device.
    prepared = {}
    expanded = 0
    if args.agent:
        data = aarch64_executable(read_limited(args.agent), 'Agent')
        # The helper ships with the agent so the two are snapshotted and rolled back together.
        if not args.esim_helper: raise ValueError('--esim-helper is required with --agent')
        prepared['esim_helper'] = aarch64_executable(read_limited(args.esim_helper), 'eSIM helper')
        password = os.environ.get('ZTE_AGENT_PASSWORD') or getpass.getpass('Agent password: ')
        pin = os.environ.get('ZTE_AGENT_PIN', '')
        spec = importlib.util.spec_from_file_location('startup', ROOT / 'scripts/render-agent-startup.py')
        startup = importlib.util.module_from_spec(spec); spec.loader.exec_module(startup)
        script = startup.render(password, pin).encode()
        run(['sh', '-n'], script)
        prepared['agent'] = data
        prepared['startup'] = script
    if args.dashboard:
        expanded = validate_dashboard(args.dashboard)
        prepared['dashboard'] = read_limited(args.dashboard)
        if len(prepared['dashboard']) > 64 * 1024 * 1024: raise ValueError('Dashboard archive exceeds 64 MiB')
    programs = prepare_hardening(args.bundle) if args.harden else {}
    if args.auto_adb and not args.adb_serial:
        devices = [line.split()[0] for line in run(['adb', 'devices']).splitlines()[1:] if line.endswith('\tdevice')]
        if len(devices) > 1: raise RuntimeError('Multiple ADB devices connected; select one with --adb-serial')
        if devices: args.adb_serial = devices[0]
    transport = Transport(args.gateway, args.adb_serial)
    identity = transport.identity()
    if args.identity_file and json.loads(Path(args.identity_file).read_text()) != identity:
        raise RuntimeError('The management connection is not the modem selected for unlock')
    print(f"Verified {identity['model']} ({identity['firmware']})")
    # Count snapshot plus rollback-copy space, including legacy dashboard files.
    preflight = transport.shell("set -e; for tool in sh sha256sum tar cp mv readlink awk df du sync ubus uci; do command -v \"$tool\" >/dev/null; done; "
                                "test -x /usr/bin/curl; sh -n /etc/rc.local; test -w /data; test -w /etc; "
                                "test ! -e /data/local/tmp/open-u60-transactions/active; test ! -d /data/local/tmp/open-u60-transactions/lock; "
                                "df -Pk /data | awk 'END {print $4}'; "
                                "df -Pk /tmp | awk 'END {print $4}'; df -Pk /etc | awk 'END {print $4}'; "
                                "(du -sk /data/zte-agent /data/zte-esim /data/www /data/www.current /data/bin /data/dropbear /data/local/tmp/start_*.sh /etc/dropbear /etc/rc.local /etc/config/uhttpd 2>/dev/null || true) | awk '{n+=$1} END {print n+0}'")
    free, tmp_free, etc_free, backup = map(int, preflight.splitlines())
    payload = sum(map(len, prepared.values())) + sum(map(len, programs.values()))
    required = payload + expanded + 2 * backup * 1024 + 16 * 1024 * 1024
    if free * 1024 < required or tmp_free * 1024 < payload + 16 * 1024 * 1024 or etc_free < 2048:
        raise RuntimeError('Insufficient storage for deployment and rollback')
    if args.dashboard and not args.harden: transport.shell('test -x /data/bin/dashboard-uhttpd && test -x /data/local/tmp/start_dashboard.sh')
    if args.dry_run:
        print('Dry run passed: identity, local artifacts and storage checked; no deployment writes.')
        return
    if transport.identity() != identity: raise RuntimeError('Device identity changed; deployment stopped')
    transaction = str(uuid.uuid4())
    transaction_script = '/data/local/tmp/open-u60-transaction.sh'
    transport.shell('mkdir -p /data/local/tmp')
    transport.push((ROOT / 'scripts/device/stop-owned-listener.sh').read_bytes(), '/data/local/tmp/stop_open_u60_listener.sh', True)
    transport.push((ROOT / 'scripts/device/transaction.sh').read_bytes(), transaction_script, True)
    transport.shell(f'sh -n {transaction_script}')
    action = f'sh {transaction_script} {{}} {transaction} {identity["fingerprint"]}'
    transport.shell(action.format('begin'))
    recovery = action.format('restore')
    print(f'Recovery command for this device: {recovery}')
    try:
        if args.harden: harden(transport, programs, identity)
        if args.agent:
            # Complete and hash-check both staged files before stopping the process.
            transport.push(prepared['agent'], '/data/zte-agent.staged', True)
            transport.push(prepared['esim_helper'], '/data/zte-esim.staged', True)
            transport.push(prepared['startup'], '/data/local/tmp/start_zte_agent.sh.staged', True)
            transport.shell('set -e; sh -n /data/local/tmp/start_zte_agent.sh.staged; killall zte-agent 2>/dev/null || true')
            # Wait for a helper the old agent started: it holds the card lock until it exits.
            transport.shell('for i in 1 2 3 4 5 6 7 8 9 10; do pidof zte-esim >/dev/null || break; sleep 1; done')
            transport.shell('set -e; mv /data/zte-agent.staged /data/zte-agent; mv /data/zte-esim.staged /data/zte-esim; '
                            'mv /data/local/tmp/start_zte_agent.sh.staged /data/local/tmp/start_zte_agent.sh')
            verify_esim_helper(transport)
            transport.push((ROOT / 'scripts/device/update-rc-local.sh').read_bytes(), '/data/local/tmp/open-u60-rc-update.sh', True)
            transport.shell("sh /data/local/tmp/open-u60-rc-update.sh 'sh /data/local/tmp/start_zte_agent.sh'")
            transport.shell('sh /data/local/tmp/start_zte_agent.sh')
            for attempt in range(5):
                time.sleep(1)
                try:
                    verify_credentials(transport, password, pin)
                    break
                except (RuntimeError, KeyError):
                    if attempt == 4: raise
        if args.dashboard:
            target = f'/data/open-u60-dashboards/{transaction}'
            transport.push(prepared['dashboard'], '/data/local/tmp/dashboard-dist.tar.gz')
            transport.shell(f'set -e; mkdir -p {target}; tar xzf /data/local/tmp/dashboard-dist.tar.gz -C {target}; '
                            f'test -s {target}/index.html; cp {target}/index.html {target}/mobile.html; '
                            f'chmod -R a+rX {target}; '
                            f'rm -f /data/www.current.new; ln -s {target} /data/www.current.new; mv -Tf /data/www.current.new /data/www.current; '
                            'rm -f /data/local/tmp/dashboard-dist.tar.gz')
            # Deploy the root-pointer-aware service script with the dashboard.
            transport.push((ROOT / 'scripts/device/start-dashboard.sh').read_bytes(), '/data/local/tmp/start_dashboard.sh', True)
            transport.shell('sh /data/local/tmp/start_dashboard.sh')
            time.sleep(1)
            transport.shell("/usr/bin/curl --fail --silent --show-error --connect-timeout 5 --max-time 10 http://127.0.0.1:8080/ | grep -q '<div id=\"root\"></div>'")
        if transport.identity() != identity: raise RuntimeError('Device identity changed during deployment')
        manifest = {'format_version': 1, 'device': identity, 'source': 'local terminal deployment',
                    'files': {name: hashlib.sha256(data).hexdigest() for name, data in prepared.items()}}
        transport.push(json.dumps(manifest, indent=2).encode(), '/data/open-u60-manifest.json')
        transport.shell(action.format('complete'))
    except BaseException:
        try:
            if transport.identity() != identity: raise RuntimeError('Device identity changed')
            transport.shell(recovery)
            print('Previous installation restored.', file=sys.stderr)
        except Exception:
            print(f'Automatic recovery did not finish. Reconnect to the same modem and run: {recovery}', file=sys.stderr)
        raise
    print('Deployment verified. Recovery snapshot retained on the modem.')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--agent')
    parser.add_argument('--esim-helper', help='zte-esim binary deployed with the agent')
    parser.add_argument('--prepare-only', action='store_true')
    parser.add_argument('--bundle-output')
    parser.add_argument('--release-agent')
    parser.add_argument('--release-esim-helper')
    parser.add_argument('--identity-file')
    parser.add_argument('--harden', action='store_true')
    parser.add_argument('--bundle', help='Directory containing the pinned Dropbear and uhttpd IPKs')
    parser.add_argument('--auto-adb', action='store_true')
    parser.add_argument('--dashboard')
    parser.add_argument('--gateway', default=os.environ.get('ZTE_GATEWAY', '192.168.0.1'))
    parser.add_argument('--adb-serial')
    parser.add_argument('--dry-run', action='store_true')
    try:
        args = parser.parse_args()
        if args.prepare_only: prepare_only(args)
        else: deploy(args)
    except (ValueError, RuntimeError, OSError, KeyError) as error:
        print(f'Deployment stopped: {error}', file=sys.stderr)
        return 1
    return 0

if __name__ == '__main__': sys.exit(main())
