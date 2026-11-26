#!/usr/bin/env python3
"""Compile WGSL modules the way Firefox does on Windows: naga's HLSL output,
then DXC, one entry point at a time. A shader that passes Chrome (Tint) can
still fail here, as the terrain shade pass did on 26 November (an
array<mat2x2<f32>> that DXC has no mul() for: Zen drew no room).

    python3 engine/tools/hlsl-check.py SHADER_DIR [--naga naga] [--dxc dxc] [--report out.json]

SHADER_DIR holds .wgsl files, as engine/tools/play-smoke.cjs --shaders
writes them: every module the browser host compiled in the room and each
game. naga is naga-cli at the naga version in Cargo.lock (cargo install
naga-cli --version X --locked); dxc is Microsoft's DirectXShaderCompiler
(Linux release). Exits 1 when any module fails.
"""
import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ENTRY = re.compile(r'@(compute|vertex|fragment)\b[^{;]*?\bfn\s+([A-Za-z_][A-Za-z0-9_]*)', re.S)
PROFILE = {'compute': 'cs', 'vertex': 'vs', 'fragment': 'ps'}


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('dir', type=Path)
    parser.add_argument('--naga', default='naga')
    parser.add_argument('--dxc', default='dxc')
    parser.add_argument('--shader-model', default='6_0', help='HLSL shader model (default 6_0)')
    parser.add_argument('--report', type=Path)
    args = parser.parse_args()
    files = sorted(args.dir.glob('*.wgsl'))
    if not files:
        sys.exit(f'{args.dir}: no .wgsl files')
    sm = args.shader_model.replace('_', '')
    results = []
    with tempfile.TemporaryDirectory() as tmp:
        for wgsl in files:
            source = wgsl.read_text()
            entries = ENTRY.findall(source)
            hlsl = Path(tmp) / (wgsl.stem + '.hlsl')
            run = subprocess.run([args.naga, str(wgsl), str(hlsl), '--shader-model', sm], capture_output=True, text=True)
            if run.returncode != 0 or not hlsl.exists():
                results.append({'name': wgsl.name, 'ok': False, 'stage': 'naga', 'detail': (run.stderr or run.stdout).strip()[-1500:]})
                continue
            failures = []
            for stage, name in entries:
                out = subprocess.run([args.dxc, '-T', f'{PROFILE[stage]}_{args.shader_model}', '-E', name, str(hlsl), '-Fo', str(Path(tmp) / 'out.dxil')],
                                     capture_output=True, text=True)
                if out.returncode != 0:
                    failures.append(f'{stage} {name}: ' + (out.stderr or out.stdout).strip()[-1200:])
            results.append({'name': wgsl.name, 'ok': not failures, 'stage': 'dxc', 'entries': len(entries), 'detail': '\n'.join(failures)})
    for r in results:
        if not r['ok']:
            print(f"FAIL {r['name']} ({r['stage']}):\n{r['detail']}\n")
    bad = [r for r in results if not r['ok']]
    print(f'{len(results) - len(bad)} of {len(results)} WGSL modules compile through naga HLSL and DXC (SM {args.shader_model})')
    if args.report:
        args.report.write_text(json.dumps(results, indent=1))
    sys.exit(1 if bad else 0)


if __name__ == '__main__':
    main()
