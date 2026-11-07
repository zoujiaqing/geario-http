#!/usr/bin/env python3
"""Check production dependency resolution without dev-feature unification."""
import json
import pathlib
import subprocess
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
CASES = [
    ("geario-http", "hyper-rt", False),
    ("geario-http", "hyper-full", False),
    ("geario-http-ffi", "server", False),
    ("geario-http-ffi", "client", False),
    ("geario-http-ffi", "server,client", False),
    ("geario-http-ffi", "native-h1", True),
]
NATIVE_ONLY = {"ntex-httparse", "atoi_simd", "encoding_rs", "mime", "serde_urlencoded"}

for package, features, native in CASES:
    with tempfile.TemporaryDirectory(prefix="geario-features-") as directory:
        path = pathlib.Path(directory)
        # A standalone consumer keeps workspace/dev defaults out of the graph.
        dependency_path = json.dumps(str(ROOT / package))
        feature_list = json.dumps(features.split(","))
        (path / "Cargo.toml").write_text(
            '[package]\nname="feature-boundary-probe"\nversion="0.0.0"\nedition="2024"\n'
            '[workspace]\n[dependencies]\n'
            f'{package}={{path={dependency_path},default-features=false,features={feature_list}}}\n'
        )
        (path / "src").mkdir()
        (path / "src/lib.rs").write_text("")
        data = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--offline", "--format-version", "1", "--manifest-path", str(path / "Cargo.toml")],
            text=True,
        ))
        nodes = {node["id"]: node for node in data["resolve"]["nodes"]}
        reachable = set()
        def visit(node_id):
            if node_id in reachable:
                return
            reachable.add(node_id)
            for dep in nodes[node_id]["deps"]:
                if any(kind["kind"] != "dev" for kind in dep["dep_kinds"]):
                    visit(dep["pkg"])
        visit(data["resolve"]["root"])
        packages = {p["id"]: p for p in data["packages"]}
        names = {packages[p]["name"] for p in reachable}
        http_id = next(p for p in reachable if packages[p]["name"] == "geario-http")
        selected = nodes[http_id]["features"]
        assert ("http1" in selected) == native, (package, features, selected)
        if not native:
            assert not names & NATIVE_ONLY, (package, features, names & NATIVE_ONLY)
        if package == "geario-http-ffi" and features == "server":
            assert "rustls" not in names, "plaintext server unexpectedly links TLS"
        if package == "geario-http-ffi" and features == "native-h1":
            assert "hyper" not in names, "native-only server unexpectedly links hyper"
        print(f"PASS {package}: {features}")
