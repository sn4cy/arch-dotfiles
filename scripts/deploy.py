#!/usr/bin/env python3
"""Deploy only reviewed files. Backups stay outside the repository."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile

REPO = Path(__file__).resolve().parents[1]

def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def deploy(home, build=None):
    home = home.resolve()
    home.mkdir(parents=True, exist_ok=True)
    backup = home / '.local/state/arch-dotfiles/backups' / datetime.datetime.now().strftime('%Y%m%dT%H%M%S.%f')
    sources = [(p, p.relative_to(REPO / 'home')) for p in sorted((REPO / 'home').rglob('*')) if p.is_file()]
    if build:
        sources += [(build / 'fast-alt-tab', Path('.local/bin/fast-alt-tab')),
                    (build / 'cargo/release/floating-dock', Path('.local/bin/floating-dock')),
                    (REPO / 'src/dock/rust/helpers/grim-v1.5.0/build/grim', Path('.local/libexec/floating-dock/grim'))]
    # Check all build results before changing any user files.
    for src, _ in sources:
        if not src.is_file():
            raise FileNotFoundError(src)
    entries = []
    backup.mkdir(parents=True, mode=0o700)
    os.chmod(backup, 0o700)
    (backup / "manifest.json").write_text("[]\n")
    for src, rel in sources:
        dst = home / rel
        if dst.is_file() and not dst.is_symlink() and digest(src) == digest(dst):
            continue
        existed = dst.exists() or dst.is_symlink()
        if existed:
            old = backup / 'files' / rel
            old.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(dst), str(old))
        entry = {'path': str(rel), 'existed': existed, 'sha256': digest(src)}
        entries.append(entry)
        # Keep a record of each changed file alongside its backup.
        (backup / 'manifest.json').write_text(json.dumps(entries, indent=2) + '\n')
        dst.parent.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(dir=dst.parent, delete=False) as tmp:
            temporary = Path(tmp.name)
        try:
            shutil.copyfile(src, temporary)
            executable = rel.parts[:2] == ('.local', 'bin') or rel.parts[:2] == ('.local', 'libexec') or src.suffix == '.sh'
            temporary.chmod(0o755 if executable else 0o644)
            temporary.replace(dst)
        finally:
            temporary.unlink(missing_ok=True)
    print(f'Deployed {len(entries)} files. Backup: {backup}')
    return backup

def restore(home, backup):
    home = home.resolve()
    backup = backup.resolve()
    backup.relative_to(home / '.local/state/arch-dotfiles/backups')
    entries = json.loads((backup / 'manifest.json').read_text())
    for entry in entries:
        rel = Path(entry['path'])
        if rel.is_absolute() or '..' in rel.parts:
            raise ValueError('Unsafe backup path')
        dst = home / rel
        # Preserve edits made after installation rather than silently discarding them.
        if dst.exists() or dst.is_symlink():
            if dst.is_symlink() or not dst.is_file() or digest(dst) != entry['sha256']:
                raise RuntimeError(f'File changed after installation: {rel}')
    for entry in reversed(entries):
        dst = home / entry['path']
        dst.unlink(missing_ok=True)
        if entry['existed']:
            shutil.move(str(backup / 'files' / entry['path']), str(dst))
    print('Restored user files. Installed packages and system services remain.')

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--home', type=Path, default=Path.home())
    parser.add_argument('--build', type=Path)
    parser.add_argument('--restore', type=Path)
    args = parser.parse_args()
    if args.restore:
        restore(args.home, args.restore)
    else:
        deploy(args.home, args.build)
