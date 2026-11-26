"""通过标准输入在 Linux 上临时执行的只读采集器，不安装文件或常驻进程。"""
import base64
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from datetime import datetime, timezone


def command(argv):
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=5)
        return result.returncode, result.stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        return -1, ""


def cpu_ticks():
    values = list(map(int, Path('/proc/stat').read_text().splitlines()[0].split()[1:]))
    return sum(values[:8]), values[3] + values[4]


class LocalHealthRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        target = urllib.parse.urlsplit(newurl)
        if (target.scheme not in ('http', 'https')
                or target.hostname not in ('127.0.0.1', 'localhost', '::1')
                or target.username is not None):
            return None
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def collect(config):
    first = cpu_ticks()
    time.sleep(0.25)
    second = cpu_ticks()
    elapsed = second[0] - first[0]
    memory = {}
    for line in Path('/proc/meminfo').read_text().splitlines():
        name, value = line.split(':', 1)
        memory[name] = int(value.strip().split()[0]) * 1024
    disks = []
    for mount in ('/', '/data'):
        if not Path(mount).exists():
            continue
        stat = os.statvfs(mount)
        total = stat.f_blocks * stat.f_frsize
        used = (stat.f_blocks - stat.f_bfree) * stat.f_frsize
        disks.append({'mount': mount, 'total': total, 'used': used,
                      'available': stat.f_bavail * stat.f_frsize,
                      'used_pct': round(used / (used + stat.f_bavail * stat.f_frsize) * 100, 2)
                      if used + stat.f_bavail * stat.f_frsize else None})
    rx = tx = 0
    for line in Path('/proc/net/dev').read_text().splitlines()[2:]:
        name, fields = line.split(':', 1)
        # 仅统计物理/虚拟主网卡，避免 Docker 网桥重复累计。
        if name.strip().startswith(('eth', 'ens', 'enp', 'eno', 'wlan', 'wlp')):
            values = fields.split()
            rx += int(values[0])
            tx += int(values[8])
    services = []
    for name in config.get('services', []):
        code, status = command(['systemctl', 'is-active', '--', name])
        services.append({'name': name, 'status': status or 'unknown', 'ok': code == 0})
    containers = []
    docker_code, output = command(['docker', 'ps', '-a', '--format', '{{json .}}'])
    if docker_code == 0:
        for line in output.splitlines():
            item = json.loads(line)
            containers.append({k.lower(): item.get(k) for k in ('Names', 'State', 'Status', 'Image')})
    health = []
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), LocalHealthRedirect())
    for item in config.get('health_urls', []):
        start = time.monotonic()
        try:
            with opener.open(item['url'], timeout=4) as response:
                code = response.status
            health.append({'name': item['name'], 'ok': 200 <= code < 400,
                           'code': code, 'latency_ms': round((time.monotonic() - start) * 1000)})
        except Exception:
            health.append({'name': item['name'], 'ok': False, 'code': None})
    os_name = 'Linux'
    for line in Path('/etc/os-release').read_text().splitlines():
        if line.startswith('PRETTY_NAME='):
            os_name = line.split('=', 1)[1].strip('"')
    code, cron = command(['crontab', '-l'])
    tasks = []
    if code == 0:
        for item in cron.splitlines():
            fields = item.strip().split(maxsplit=5)
            if not fields or fields[0].startswith('#'):
                continue
            if (fields[0].startswith('@') and len(fields) >= 2) or (
                len(fields) == 6 and fields[0][0] in '0123456789*'
            ):
                tasks.append({'name': f'计划任务 {len(tasks) + 1}',
                              'configured': True, 'execution_verified': False})
            if len(tasks) >= 32:
                break
    return {
        'measured_at': datetime.now(timezone.utc).isoformat(),
        'hostname': socket.gethostname(), 'os': os_name, 'vcpus': os.cpu_count(),
        'boot_id': Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
        'uptime_seconds': float(Path('/proc/uptime').read_text().split()[0]),
        'cpu_pct': round(max(0, min(100, (1 - (second[1] - first[1]) / elapsed) * 100)), 2) if elapsed else None,
        'memory_total': memory['MemTotal'], 'memory_used': memory['MemTotal'] - memory['MemAvailable'],
        'memory_pct': round((1 - memory['MemAvailable'] / memory['MemTotal']) * 100, 2),
        'swap_total': memory['SwapTotal'], 'swap_used': memory['SwapTotal'] - memory['SwapFree'],
        'load': list(os.getloadavg()), 'disks': disks,
        'net_rx_total': rx, 'net_tx_total': tx, 'services': services,
        'containers': containers, 'containers_available': docker_code == 0,
        'health': health, 'tasks': tasks,
    }


if __name__ == '__main__':
    settings = json.loads(base64.b64decode(sys.argv[1])) if len(sys.argv) > 1 else {}
    print(json.dumps(collect(settings), ensure_ascii=False))
