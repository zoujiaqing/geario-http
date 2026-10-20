import hashlib
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
results = base/'geario-h1-ab-results'
results.mkdir(exist_ok=True)
variants = {'geario-native-h1': 'arena-baseline', 'geario-hyper-h1': 'arena-hyper'}
manifest = {}

def prepare(framework, executable):
    binary = source/'target/release'/executable
    manifest[framework] = {'sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
    with tempfile.TemporaryDirectory(prefix='geario-h1-runtime-') as directory:
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
                        '--change', 'WORKDIR /', '-', f'httparena-{framework}'],
                       stdin=archive.stdout, check=True)
        archive.stdout.close()
        if archive.wait():
            raise RuntimeError('archive failed')
    entry = base/'frameworks'/framework
    entry.mkdir(exist_ok=True)
    (entry/'meta.json').write_text(json.dumps(dict(display_name=framework,
        language='Rust', tests=['baseline'], enabled=True)))
    (entry/'build.sh').write_text(f'#!/bin/sh\nexec docker image inspect httparena-{framework} >/dev/null\n')
    (entry/'build.sh').chmod(0o755)

def preflight(framework):
    name = 'geario-h1-comparison-check'
    subprocess.run(['docker', 'run', '-d', '--name', name, '--network', 'host',
        '--security-opt', 'seccomp=unconfined', '--ulimit', 'memlock=-1:-1',
        '--ulimit', 'nofile=1048576:1048576', f'httparena-{framework}'], check=True)
    rows = []
    try:
        for _ in range(30):
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
            ('GET', '/baseline11?a=13&b=42', None, False, b'55'),
            ('GET', '/baseline11?a=-5&b=123', None, False, b'118'),
            ('GET', '/baseline11?%61=11&b=2', None, False, b'13'),
            ('POST', '/baseline11?a=13&b=42', b'20', False, b'75'),
            ('POST', '/baseline11?a=13&b=42', iter([b'2', b'0']), True, b'75'),
            ('POST', '/baseline11?a=-10&b=3', iter([b'1', b'9']), True, b'12'),
        ]:
            c.request(method, path, body=body, encode_chunked=chunked)
            r = c.getresponse()
            actual = r.read()
            assert r.status == 200 and actual == expected, (framework, method, actual)
            headers = {k.lower(): v for k, v in r.getheaders() if k.lower() != 'date'}
            rows.append(dict(method=method, path=path, chunked=chunked,
                version=r.version, headers=headers, body=actual.decode()))
        c.close()
        pid = subprocess.check_output(['docker', 'inspect', '-f', '{{.State.Pid}}', name], text=True).strip()
        fds = subprocess.check_output(['ls', '-l', f'/proc/{pid}/fd'], text=True)
        assert 'io_uring' in fds, 'io_uring not observed'
        (results/f'{framework}-fds.txt').write_text(fds)
        (results/f'{framework}-startup.log').write_text(subprocess.check_output(
            ['docker', 'logs', name], stderr=subprocess.STDOUT, text=True))
    finally:
        subprocess.run(['docker', 'rm', '-f', name], check=True)
    return rows

for framework, executable in variants.items():
    prepare(framework, executable)
checks = {framework: preflight(framework) for framework in variants}
(results/'preflight.json').write_text(json.dumps(checks, indent=2))
assert checks['geario-native-h1'] == checks['geario-hyper-h1'], 'Response semantics differ'
(results/'artifacts.json').write_text(json.dumps(manifest, indent=2))
print('Both io_uring servers passed identical response and keepalive checks', flush=True)

env = os.environ.copy()
env.update(SKIP_TUNE='true', THREADS='16', GCANNON_CPUS='0-15', RUNS='3', DURATION='5s',
           GCANNON=str(base/'neton-lan-remote-gcannon.py'), GCANNON_MODE='native')
for turn, framework in enumerate(['geario-native-h1', 'geario-hyper-h1', 'geario-hyper-h1', 'geario-native-h1']):
    path = results/f'baseline-{turn}-{framework}.log'
    print(f'Starting {turn} {framework}', flush=True)
    with path.open('w') as log:
        subprocess.run(['bash', 'scripts/benchmark.sh', framework, 'baseline'],
            cwd=base, env=env, stdout=log, stderr=subprocess.STDOUT, timeout=240, check=True)
    content = path.read_text()
    assert content.count('Templates: 3') == 3, 'Missing templates'
    assert content.count('Per-template-ok:') == 3, 'Missing success counts'
    print(content[-700:], flush=True)
