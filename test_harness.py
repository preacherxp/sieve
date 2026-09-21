import json
import tempfile
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path
import validate as v


class HarnessTests(unittest.TestCase):
    def test_every_mutation_applies_to_both_builds(self):
        for tool in ('maven', 'gradle'):
            for name in v.SCENARIOS:
                with self.subTest(tool=tool, scenario=name), tempfile.TemporaryDirectory() as tmp:
                    root = v.prepare(tool, Path(tmp) / 'project')
                    v.apply_scenario(root, name)
                    self.assertEqual(json.loads((root / v.MARKER).read_text())['scenario'], name)
                    if tool == 'maven':
                        for pom in root.rglob('pom.xml'):
                            ET.parse(pom)
                    with self.assertRaises(ValueError):
                        v.apply_scenario(root, name)

    def test_sources_match(self):
        for path in (v.ROOT / 'projects/maven').rglob('*'):
            if path.is_file() and '/src/' in path.as_posix():
                relative = path.relative_to(v.ROOT / 'projects/maven')
                self.assertEqual(path.read_bytes(), (v.ROOT / 'projects/gradle' / relative).read_bytes())

    def test_oracle_rejects_missing_test(self):
        required = v.SCENARIOS['tax-transitive']['required']
        self.assertFalse(v.check_selection('tax-transitive', {'mode': 'SUBSET', 'tests': required[:-1]})['ok'])
        self.assertTrue(v.check_selection('tax-transitive', {'mode': 'SUBSET', 'tests': required}, True)['ok'])

    def test_conservative_selection_and_exact_mode(self):
        self.assertTrue(v.check_selection('tax-transitive', {'mode': 'ALL'})['ok'])
        self.assertFalse(v.check_selection('tax-transitive', {'mode': 'ALL'}, True)['ok'])
        self.assertTrue(v.check_selection('docs-only', {'mode': 'NONE'}, True)['ok'])

    def test_invalid_selection(self):
        for payload in [{'mode': 'bad'}, {'mode': 'NONE', 'tests': ['x']},
                        {'mode': 'SUBSET', 'tests': []}, {'mode': 'SUBSET', 'tests': ['x', 'x']}]:
            with self.assertRaises(ValueError):
                v.check_selection('unused', payload)
        self.assertFalse(v.check_selection('unused', {'mode': 'SUBSET', 'tests': ['bogus']})['ok'])
        for modules in [[], ['bogus'], ['pricing', 'pricing'], [None], 'pricing']:
            with self.assertRaises(ValueError):
                v.check_selection('unused', {'mode': 'MODULES', 'modules': modules})
        selected = {'mode': 'MODULES', 'modules': ['pricing', 'checkout']}
        self.assertTrue(v.check_selection('tax-transitive', selected)['ok'])
        self.assertFalse(v.check_selection('tax-transitive', selected, exact=True)['ok'])

    def test_reports_count_parameterized_cases_and_failures(self):
        for tool, folder in [('maven', 'target/surefire-reports'), ('gradle', 'build/test-results/test')]:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                reports = root / 'checkout' / folder
                reports.mkdir(parents=True)
                (reports / 'TEST-example.xml').write_text('''<testsuite>
                  <testcase classname="example.ParameterizedCheckoutTest" name="one"/>
                  <testcase classname="example.ParameterizedCheckoutTest" name="two"><failure/></testcase>
                  <testcase classname="example.SkippedTest" name="skip"><skipped/></testcase>
                </testsuite>''')
                actual = v.read_reports(root, tool)
                self.assertEqual(actual['cases'], 2)
                self.assertEqual(actual['failed'], ['checkout:unit:example.ParameterizedCheckoutTest'])
                self.assertEqual(len(actual['executed']), 1)

if __name__ == '__main__':
    unittest.main()
