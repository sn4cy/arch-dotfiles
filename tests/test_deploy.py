import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('deploy', Path(__file__).resolve().parents[1] / 'scripts/deploy.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class DeploymentTests(unittest.TestCase):
    def test_backup_idempotence_and_restore(self):
        with tempfile.TemporaryDirectory(prefix='dotfiles home ') as directory:
            home = Path(directory)
            (home / '.bashrc').write_text('original settings\n')
            target = home / '.config/ghostty/config'
            target.parent.mkdir(parents=True)
            original = home / 'original-ghostty'
            original.write_text('existing symlink target\n')
            target.symlink_to(original)
            first = module.deploy(home)
            self.assertEqual((first / 'files/.bashrc').read_text(), 'original settings\n')
            self.assertTrue((first / 'files/.config/ghostty/config').is_symlink())
            self.assertEqual(original.read_text(), 'existing symlink target\n')
            self.assertEqual((home / '.config/hypr/scripts/lock.sh').stat().st_mode & 0o777, 0o755)
            second = module.deploy(home)
            self.assertEqual(json.loads((second / 'manifest.json').read_text()), [])
            (home / '.bashrc').write_text('user edited after setup\n')
            with self.assertRaises(RuntimeError):
                module.restore(home, first)
            self.assertEqual((home / '.bashrc').read_text(), 'user edited after setup\n')
            (home / '.bashrc').write_bytes((module.REPO / 'home/.bashrc').read_bytes())
            module.restore(home, first)
            self.assertEqual((home / '.bashrc').read_text(), 'original settings\n')
            self.assertTrue(target.is_symlink())
            self.assertFalse((home / '.config/hypr/hyprland.lua').exists())

    def test_missing_build_does_not_change_home(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            (home / '.bashrc').write_text('keep\n')
            with self.assertRaises(FileNotFoundError):
                module.deploy(home, home / 'missing')
            self.assertEqual((home / '.bashrc').read_text(), 'keep\n')
            self.assertFalse((home / '.config').exists())

    def test_unsafe_manifest_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            backup = home / '.local/state/arch-dotfiles/backups/test'
            backup.mkdir(parents=True)
            (backup / 'manifest.json').write_text(json.dumps([{'path':'../../escape'}]))
            with self.assertRaises(ValueError):
                module.restore(home, backup)

if __name__ == '__main__':
    unittest.main()
