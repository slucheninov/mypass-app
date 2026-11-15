#!/usr/bin/env python3
"""Unsigned Windows ZIP producer and fail-closed extraction/provenance verifier."""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import zipfile
from release import ROOT, check_version, extension_files, require, verify_zip

TARGET = 'x86_64-pc-windows-msvc'


def pe(data, subsystem):
    require(len(data) >= 64 and data[:2] == b'MZ', 'missing PE DOS header')
    offset = struct.unpack_from('<I', data, 60)[0]
    require(offset + 24 <= len(data) and data[offset:offset + 4] == b'PE\0\0', 'missing PE signature')
    require(struct.unpack_from('<H', data, offset + 4)[0] == 0x8664, 'PE must be x64')
    size = struct.unpack_from('<H', data, offset + 20)[0]
    require(size >= 112 and offset + 24 + size <= len(data), 'truncated PE optional header')
    require(struct.unpack_from('<H', data, offset + 24)[0] == 0x20b, 'PE must be PE32+')
    require(struct.unpack_from('<H', data, offset + 92)[0] == subsystem, 'unexpected PE subsystem')


def source_bytes(path):
    require(path.is_file() and not path.is_symlink(), f'missing regular input: {path}')
    data = path.read_bytes()
    require(data, f'empty input: {path}')
    return data


def payloads(source, ver, root=ROOT):
    # This exact name is consumed by locate_bundled_host; the registrar installs mypass-host.exe.
    app = source_bytes(source / 'mypass-app.exe')
    host = source_bytes(source / 'mypass-native-host.exe')
    pe(app, 2)  # GUI app; host retains console subsystem for native messaging pipes.
    pe(host, 3)
    install = source_bytes(root / 'scripts/package/INSTALL-windows.txt').decode('ascii')
    require('\r' not in install, 'INSTALL source must use LF')
    files = {'MyPass.exe': app, 'mypass-native-host.exe': host,
             'INSTALL.txt': install.replace('\n', '\r\n').encode('ascii')}
    for name in ('LICENSE', 'THIRD_PARTY_NOTICES.md'):
        files[name] = source_bytes(root / name)
    for path, rel in extension_files(root / 'extension'):
        files['extension/' + rel] = source_bytes(path)
    require(json.loads(files['extension/manifest.json'])['version'] == ver, 'extension version mismatch')
    return files


def verify_archive(path, source, ver, root=ROOT):
    expected = payloads(source, ver, root)
    with zipfile.ZipFile(path) as archive:
        names = archive.namelist()
        require(len(names) == len(set(names)), 'ZIP duplicate entries')
        for info in archive.infolist():
            name = info.filename
            parts = PurePosixPath(name)
            require(name and not parts.is_absolute() and '..' not in parts.parts
                    and '\\' not in name and ':' not in name and str(parts) == name,
                    'ZIP unsafe relative entry')
            require(stat.S_IFMT(info.external_attr >> 16) in (0, stat.S_IFREG), 'ZIP nonregular entry')
        require(set(names) == set(expected), 'ZIP exact file set mismatch')
        for name, data in expected.items():
            require(hashlib.sha256(archive.read(name)).digest() == hashlib.sha256(data).digest(),
                    f'ZIP byte provenance mismatch: {name}')
        # Extract only after every entry has passed the path/type/content checks.
        with tempfile.TemporaryDirectory() as temp:
            archive.extractall(temp)
            extracted = Path(temp)
            require({p.relative_to(extracted).as_posix() for p in extracted.rglob('*') if p.is_file()}
                    == set(expected), 'extracted file set mismatch')
            for name, data in expected.items():
                require(hashlib.sha256((extracted / name).read_bytes()).digest()
                        == hashlib.sha256(data).digest(), f'extracted hash mismatch: {name}')


def tool(name):
    # Native Windows lookup can find the WSL bash launcher before Git Bash.
    if name == 'bash' and os.name == 'nt':
        system = os.environ.get('SystemRoot', r'C:\Windows').lower()
        for entry in os.environ.get('PATH', '').split(os.pathsep):
            candidate = Path(entry) / 'bash.exe'
            if not str(candidate).lower().startswith(system) and candidate.is_file():
                return str(candidate)
        raise ValueError('Git Bash is required (WSL launcher is not supported)')
    found = shutil.which(name)
    require(found, f'required command not found: {name}')
    return found


def build():
    # Through Git Bash, pnpm's .cmd launcher and shell scripts resolve consistently on Windows.
    bash = tool('bash')
    tool('cargo')
    tool('pnpm')
    require(os.name == 'nt', 'production Windows build requires Windows')
    env = dict(os.environ)
    for key in tuple(env):
        if key.startswith('APPLE_') or key in ('MYPASS_SIGNING_IDENTITY', 'TAURI_SIGNING_PRIVATE_KEY',
                                              'TAURI_SIGNING_PRIVATE_KEY_PASSWORD'):
            env.pop(key, None)
    subprocess.run([bash, 'scripts/build-windows.sh'], cwd=ROOT, env=env, check=True)


def package(args):
    ver = check_version(args.version)
    require(args.no_build == (args.stage_from is not None), '--stage-from requires --no-build and vice versa')
    if args.release:
        require(not args.no_build, 'release must build its own binaries')
        tag = subprocess.check_output(['git', 'describe', '--exact-match', '--tags'], cwd=ROOT, text=True).strip()
        require(tag == f'v{ver}', f'release requires exact tag v{ver}')
        require(not subprocess.check_output(['git', 'status', '--porcelain', '--untracked-files=normal'],
                                            cwd=ROOT, text=True).strip(), 'release requires a clean tree')
    if args.no_build:
        source = args.stage_from.resolve()
    else:
        build()
        metadata = subprocess.check_output([tool('cargo'), 'metadata', '--format-version', '1', '--no-deps'],
                                           cwd=ROOT, text=True)
        source = Path(json.loads(metadata)['target_directory']) / TARGET / 'release'
    files = payloads(source, ver)
    args.out.mkdir(parents=True, exist_ok=True)
    archive_path = args.out / f'MyPass-{ver}-windows-x64.zip'
    with tempfile.TemporaryDirectory() as temp:
        temp = Path(temp)
        # Reuse the authoritative production exclusions and extension ZIP verifier.
        subprocess.run([tool('bash'), 'scripts/package-extension.sh', '--out', str(temp),
                        '--stage-dir', str(temp / 'extension')], cwd=ROOT, check=True)
        verify_zip(temp / f'MyPass-chrome-extension-{ver}.zip', ver, temp / 'extension')
        for path in (temp / 'extension').rglob('*'):
            if path.is_file():
                require(files['extension/' + path.relative_to(temp / 'extension').as_posix()] == path.read_bytes(),
                        'extension staging mismatch')
        staged = temp / archive_path.name
        with zipfile.ZipFile(staged, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            for name, data in sorted(files.items()):
                info = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
                info.create_system = 3
                info.external_attr = 0o100644 << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, data)
        verify_archive(staged, source, ver)
        checksum = f'{hashlib.sha256(staged.read_bytes()).hexdigest()}  {archive_path.name}\n'
        shutil.copyfile(staged, archive_path)
        archive_path.with_suffix('.zip.sha256').write_bytes(checksum.encode('ascii'))
    print(archive_path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, default=ROOT / 'dist')
    parser.add_argument('--version')
    parser.add_argument('--no-build', action='store_true')
    parser.add_argument('--stage-from', type=Path)
    parser.add_argument('--release', action='store_true', help='require clean exact version tag and build from source')
    package(parser.parse_args())


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError, zipfile.BadZipFile) as error:
        sys.exit(f'windows-package: {error}')
