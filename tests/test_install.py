"""Install fixtures never read keyboards, play audio or contact the network."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class PackInstallationTests(unittest.TestCase):
    def test_partial_pack_is_repaired_then_qualified_pack_is_skipped(self):
        with tempfile.TemporaryDirectory(prefix='keysounds setup ') as temporary:
            home = Path(temporary)
            repo = home / 'repo'
            repo.mkdir()
            for name in ('fetch-packs.sh', 'config', 'scripts'):
                source = ROOT / name
                shutil.copytree(source, repo / name) if source.is_dir() else shutil.copy2(source, repo / name)
            pack = repo / 'packs/nk-cream'
            pack.mkdir(parents=True)
            (pack / 'keep.txt').write_text('user data')
            bindir = home / 'bin'
            bindir.mkdir()
            git = bindir / 'git'
            git.write_text('''#!/usr/bin/python3
import os,sys
from pathlib import Path
if 'clone' in sys.argv:
    Path(os.environ['HOME'],'clones').write_text('once')
    p=Path(sys.argv[-1])/'src/audio/nk-cream'
    p.mkdir(parents=True)
    (p/'config.json').write_text('{"key_define_type":"multi","defines":{"1":"key.wav"}}')
    (p/'key.wav').write_bytes(b'audio fixture')
elif 'rev-parse' in sys.argv:print('022a03b2bef20e4f25ca03b9b287bdfd2144d58f')
''')
            git.chmod(0o755)
            env = {'HOME': str(home), 'PATH': str(bindir) + ':/usr/bin:/bin'}
            for attempt in range(2):
                result = subprocess.run(['bash', str(repo / 'fetch-packs.sh')], cwd='/tmp', env=env, capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                if attempt: self.assertIn('SKIP sound packs', result.stdout)
            self.assertEqual((pack / 'keep.txt').read_text(), 'user data')
            self.assertTrue((pack / 'key.wav').is_file())
            (pack / 'key.wav').unlink()
            result = subprocess.run(['python3', str(repo / 'scripts/check-packs.py'), str(repo / 'packs')])
            self.assertNotEqual(result.returncode, 0)
