import http.client
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time

base = Path('/tmp/neton-lan-20260918')
source = Path('/tmp/geario-arena-20260918/geario-http/bench-http')
binary = source/'target/release/arena-baseline'
results = base/'geario-results'
results.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory(prefix='geario-runtime-') as directory:
    root = Path(directory)
    shutil.copy2(binary, root/'server')
    libs = subprocess.check_output(['ldd', str(binary)], text=True)
    if 'not found' in libs:
        raise RuntimeError(libs)
    for path in set(re.findall(r'/\S+', libs)):
        dest = root/path.lstrip('/')
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, dest, follow_symlinks=True)
    archive = subprocess.Popen(['tar', '-C', directory, '-cf', '-', '.'], stdout=subprocess.PIPE)
    subprocess.run(['docker', 'import', '--change', 'CMD ["/server"]',
                    '--change', 'WORKDIR /', '-', 'httparena-geario-rust'],
                   stdin=archive.stdout, check=True)
    archive.stdout.close()
    if archive.wait():
        raise RuntimeError('archive failed')

entry = base/'frameworks/geario-rust'
entry.mkdir(exist_ok=True)
(entry/'meta.json').write_text(json.dumps(dict(display_name='geario-http Rust',
    language='Rust', tests=['baseline'], enabled=True)))
(entry/'build.sh').write_text('#!/bin/sh\nexec docker image inspect httparena-geario-rust >/dev/null\n')
(entry/'build.sh').chmod(0o755)

name = 'geario-baseline-check'
subprocess.run(['docker', 'run', '-d', '--name', name, '--network', 'host',
    '--security-opt', 'seccomp=unconfined', '--ulimit', 'memlock=-1:-1',
    'httparena-geario-rust'], check=True)
try:
    for attempt in range(30):
        try:
            c = http.client.HTTPConnection('127.0.0.1', 8080, timeout=3)
            c.request('GET', '/baseline11?a=13&b=42')
            r = c.getresponse()
            assert r.status == 200 and r.read() == b'55'
            break
        except ConnectionRefusedError:
            time.sleep(0.2)
    else:
        raise RuntimeError('not ready')
    for method, path, body, chunked, expected in [
        ('GET', '/baseline11?a=-5&b=123', None, False, b'118'),
        ('GET', '/baseline11?%61=11&b=2', None, False, b'13'),
        ('POST', '/baseline11?a=13&b=42', b'20', False, b'75'),
        ('POST', '/baseline11?a=13&b=42', iter([b'2', b'0']), True, b'75'),
        ('POST', '/baseline11?a=-10&b=3', iter([b'1', b'9']), True, b'12'),
    ]:
        c.request(method, path, body=body, encode_chunked=chunked)
        r = c.getresponse()
        actual = r.read()
        assert r.status == 200 and actual == expected, (method, path, r.status, actual)
        print('PASS', method, path, 'chunked=', chunked, actual, flush=True)
    c.close()
    pid = subprocess.check_output(['docker', 'inspect', '-f', '{{.State.Pid}}', name], text=True).strip()
    fds = subprocess.run(['ls', '-l', f'/proc/{pid}/fd'], capture_output=True, text=True)
    (results/'runtime-fds.txt').write_text(fds.stdout + fds.stderr)
finally:
    subprocess.run(['docker', 'rm', '-f', name], check=True)

env = os.environ.copy()
env.update(SKIP_TUNE='true', THREADS='16', GCANNON_CPUS='0-15', RUNS='3', DURATION='5s',
           GCANNON=str(base/'neton-lan-remote-gcannon.py'), GCANNON_MODE='native')
for turn in range(3):
    path = results/f'baseline-{turn}.log'
    print(f'Starting group {turn}', flush=True)
    with path.open('w') as log:
        subprocess.run(['bash', 'scripts/benchmark.sh', 'geario-rust', 'baseline'],
            cwd=base, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=240, check=True)
    content = path.read_text()
    if 'Templates: 3' not in content or 'Per-template-ok:' not in content:
        raise RuntimeError('Missing three-template evidence')
    print(content[-1800:], flush=True)
