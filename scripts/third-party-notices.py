#!/usr/bin/env python3
"""Collect locally available Cargo dependency licenses for distribution payloads."""
import json, pathlib, subprocess, sys, tempfile, os
# Cargo builds fetch only their target dependencies; metadata also needs other platforms.
metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--format-version', '1']))
output = pathlib.Path(sys.argv[1])
output.parent.mkdir(parents=True, exist_ok=True)
with tempfile.NamedTemporaryFile(mode='w', encoding='utf-8', dir=output.parent, delete=False) as result:
    result.write('Tetherust Cargo dependency license notices\nIncludes build and platform-specific dependencies.\n\n')
    for package in sorted(metadata['packages'], key=lambda p: (p['name'], p['version'])):
        if package['name'] == 'tetherust':
            continue
        result.write('\n' + '=' * 72 + '\n' + package['name'] + ' ' + package['version'] + '\n')
        result.write('License expression: ' + (package['license'] or 'See included license file') + '\n')
        if package.get('repository'):
            result.write('Source: ' + package['repository'] + '\n')
        root = pathlib.Path(package['manifest_path']).parent
        files = {p for p in root.iterdir() if p.is_file() and p.name.upper().startswith(('LICENSE', 'LICENCE', 'COPYING', 'NOTICE', 'COPYRIGHT'))}
        if package.get('license_file'):
            files.add(root / package['license_file'])
        for path in sorted(files):
            if path.is_file():
                result.write('\n--- ' + path.name + ' ---\n' + path.read_text(encoding='utf-8', errors='replace') + '\n')
os.chmod(result.name, 0o644)
os.replace(result.name, output)
print(output)
