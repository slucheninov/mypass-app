#!/usr/bin/env bash
# Production Windows build, including the UI and separately built native messaging sidecar.
set -euo pipefail
cd "$(dirname "$0")/.."
for tool in cargo pnpm python3; do
  command -v "$tool" >/dev/null || { echo "Error: missing $tool" >&2; exit 1; }
done
case "$(uname -s)" in MINGW*|MSYS*) ;; *) echo 'Error: Windows Git Bash required' >&2; exit 1 ;; esac
# Windows is unsigned. Never invoke Apple signing or consume signing credentials.
unset MYPASS_SIGNING_IDENTITY APPLE_SIGNING_IDENTITY APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD
unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PASSWORD
bash scripts/check-version.sh
bash scripts/check-host-deps.sh --target x86_64-pc-windows-msvc
bash scripts/build-host.sh --release --target x86_64-pc-windows-msvc
pnpm install --frozen-lockfile
pnpm tauri build --no-bundle --no-sign --target x86_64-pc-windows-msvc
python3 - <<'PY'
import json, shutil, subprocess, sys
from pathlib import Path
sys.path.insert(0, 'scripts')
from importlib.util import spec_from_file_location, module_from_spec
spec = spec_from_file_location('windows_package', 'scripts/windows-package.py')
m = module_from_spec(spec)
spec.loader.exec_module(m)
target = Path(json.loads(subprocess.check_output(['cargo', 'metadata', '--format-version', '1', '--no-deps']))['target_directory'])
folder = target / m.TARGET / 'release'
source = Path('src-tauri/binaries') / f'mypass-native-host-{m.TARGET}.exe'
built = m.source_bytes(folder / 'mypass-host.exe')
m.require(m.source_bytes(source) == built, 'staged sidecar differs from cargo host')
# --no-bundle does not promise to stage externalBin beside the app. Stage it explicitly.
shutil.copyfile(source, folder / 'mypass-native-host.exe')
m.require(m.source_bytes(folder / 'mypass-native-host.exe') == built, 'packaging sidecar differs from cargo host')
m.pe(built, 3)
m.pe(m.source_bytes(folder / 'mypass-app.exe'), 2)
index = Path('dist/index.html').read_text()
m.require('assets/' in index and any(Path('dist/assets').glob('*.js')), 'production UI missing (placeholder refused)')
print('Windows x64 production app, UI and host provenance verified')
PY
