#!/usr/bin/env python3
"""Verify/extract the Windows ZIP and exercise the shipped host with real framing.

Run only on an ephemeral Windows CI profile with no running MyPass application.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import zipfile
from release import check_version, require


def check_frame(data):
    require(len(data) >= 4, 'host response missing frame prefix')
    length = struct.unpack_from('<I', data)[0]
    require(0 < length <= 1024 * 1024 and len(data) == length + 4, 'host response framing mismatch')
    require(json.loads(data[4:]) == {'type': 'error', 'message': 'app_not_running'},
            'host must report app_not_running without the app')


def main():
    spec = importlib.util.spec_from_file_location('windows_package', Path(__file__).with_name('windows-package.py'))
    package = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(package)
    archive = Path(sys.argv[1]).resolve()
    source = Path(sys.argv[2]).resolve()
    ver = check_version()
    require(archive.name == f'MyPass-{ver}-windows-x64.zip', 'archive name mismatch')
    expected = f'{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n'
    require(archive.with_suffix('.zip.sha256').read_bytes() == expected.encode('ascii'), 'checksum mismatch')
    package.verify_archive(archive, source, ver)
    with tempfile.TemporaryDirectory() as folder:
        with zipfile.ZipFile(archive) as stream:
            stream.extractall(folder)
        request = json.dumps({'type': 'getCredentials', 'token': '', 'url': 'https://example.com', 'pairing': 'ci-smoke'}).encode()
        result = subprocess.run([str(Path(folder) / 'mypass-native-host.exe')],
                                input=struct.pack('<I', len(request)) + request,
                                capture_output=True, timeout=15, check=True)
        require(not result.stderr, 'host wrote unexpected stderr')
        check_frame(result.stdout)
    print('extracted Windows host: valid app_not_running frame, exit 0')


if __name__ == '__main__':
    main()
