"""Black-box checks against the independent fixture oracle; build the Rust binary first."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import validate as v

BINARY = Path(os.environ.get('IMPACT_BIN', v.ROOT / 'target/debug/java-test-impact')).resolve()


def select(workspace, base='HEAD'):
    return json.loads(subprocess.check_output([
        str(BINARY), 'select', '--workspace', str(workspace), '--base', base], text=True))


def git(workspace, *args):
    return subprocess.check_output(['git', '-c', 'commit.gpgsign=false', '-C', str(workspace), *args], text=True).strip()


class SelectorTests(unittest.TestCase):
    def test_all_scenarios_meet_independent_safety_oracle(self):
        for tool in ('maven', 'gradle'):
            for name in v.SCENARIOS:
                with self.subTest(tool=tool, scenario=name), tempfile.TemporaryDirectory() as tmp:
                    workspace = v.prepare(tool, Path(tmp) / 'project', git=True)
                    v.apply_scenario(workspace, name)
                    actual = select(workspace)
                    self.assertTrue(v.check_selection(name, actual)['ok'], actual)
                    if name == 'docs-only':
                        self.assertEqual(actual['mode'], 'NONE')
                    elif name == 'tax-transitive':
                        self.assertEqual(actual['modules'], ['checkout', 'pricing'])
                    elif name == 'reflection':
                        self.assertEqual(actual['modules'], ['runtime'])

    def test_commits_staging_renames_untracked_files_and_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            workspace = v.prepare('maven', Path(tmp) / 'project', git=True)
            baseline = git(workspace, 'rev-parse', 'HEAD')
            self.assertEqual(select(workspace)['mode'], 'NONE')
            self.assertEqual(select(workspace, 'missing-revision')['mode'], 'ALL')
            v.apply_scenario(workspace, 'tax-transitive')
            git(workspace, 'add', '.')
            self.assertEqual(select(workspace)['modules'], ['checkout', 'pricing'])
            git(workspace, '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid',
                'commit', '-qm', 'Change tax')
            self.assertEqual(select(workspace, baseline)['modules'], ['checkout', 'pricing'])
            self.assertEqual(select(workspace)['mode'], 'NONE')
            source = workspace / 'pricing/src/main/java/example/UnusedDiscount.java'
            source.rename(workspace / 'runtime/src/main/java/example/UnusedDiscount.java')
            self.assertEqual(select(workspace)['modules'], ['checkout', 'pricing', 'runtime'])
            (workspace / 'unknown config.txt').write_text('changed')
            self.assertEqual(select(workspace)['mode'], 'ALL')

    def test_merge_base_includes_earlier_branch_commits(self):
        with tempfile.TemporaryDirectory() as tmp:
            workspace = v.prepare('gradle', Path(tmp) / 'project', git=True)
            baseline = git(workspace, 'rev-parse', 'HEAD')
            v.apply_scenario(workspace, 'tax-transitive')
            git(workspace, 'add', '.')
            git(workspace, '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid',
                'commit', '-qm', 'Feature')
            git(workspace, 'branch', 'feature')
            git(workspace, 'checkout', '-q', '--detach', baseline)
            (workspace / 'README.md').write_text('New main-branch docs')
            git(workspace, 'add', '.')
            git(workspace, '-c', 'user.name=Test', '-c', 'user.email=test@example.invalid',
                'commit', '-qm', 'Main advanced')
            main = git(workspace, 'rev-parse', 'HEAD')
            git(workspace, 'checkout', '-q', 'feature')
            self.assertEqual(select(workspace, main)['modules'], ['checkout', 'pricing'])


if __name__ == '__main__':
    unittest.main()
