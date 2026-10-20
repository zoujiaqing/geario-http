"""Summarize all measured rounds, not only the best run of each group."""
import json
from pathlib import Path
import re
import statistics

root = Path(__file__).parent
summary = {}
for name in ('geario-native-h1', 'geario-hyper-h1'):
    rounds, cpu, memory, best, anomalies = [], [], [], [], []
    files = sorted(root.glob(f'baseline-*-{name}.log'))
    assert len(files) == 2, f'{name}: expected two groups'
    for path in files:
        text = path.read_text()
        assert text.count('Templates: 3') == 3, path
        counts = re.findall(r'^  Per-template: ([\d,]+)', text, re.M)
        ok = re.findall(r'^  Per-template-ok: ([\d,]+)', text, re.M)
        assert len(counts) == len(ok) == 3, path
        for index, (completed, successful) in enumerate(zip(counts, ok)):
            if completed != successful:
                anomalies.append(dict(file=path.name, round=index + 1,
                                      completed=completed, successful=successful))
        statuses = re.findall(r'Status codes: 2xx=(\d+), 3xx=(\d+), 4xx=(\d+), 5xx=(\d+)', text)
        assert len(statuses) == 3 and all(int(a) > 0 and (b, c, d) == ('0', '0', '0')
                                        for a, b, c, d in statuses), path
        rounds += [float(n) * 1000 for n in re.findall(r'Throughput: ([\d.]+)K req/s', text)]
        cpu += [float(n) for n in re.findall(r'\[info\] CPU ([\d.]+)%', text)]
        memory += [int(n) for n in re.findall(r'\[info\] CPU [\d.]+% \| Mem (\d+)MiB', text)]
        best += [int(n) for n in re.findall(r'=== Best: (\d+) req/s', text)]
    assert len(rounds) == len(cpu) == len(memory) == 6
    summary[name] = dict(rounds_rps=rounds, median_rps=statistics.median(rounds),
                         cpu_percent=cpu, memory_mib=memory, official_group_best=best,
                         counter_anomalies=anomalies)
summary['native_advantage_percent'] = 100 * (
    summary['geario-native-h1']['median_rps'] / summary['geario-hyper-h1']['median_rps'] - 1)
print(json.dumps(summary, indent=2))
