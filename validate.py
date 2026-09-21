#!/usr/bin/env python3
"""Standard-library-only fixture manager and selection/result oracle."""
import argparse
import json
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parent
CATALOG = json.loads((ROOT / 'scenarios.json').read_text())
SCENARIOS = {s['id']: s for s in CATALOG['scenarios']}
MARKER = '.fixture-workspace.json'


def write_json(path, data):
    Path(path).write_text(json.dumps(data, indent=2) + '\n')


def prepare(tool, destination, git=False):
    destination = Path(destination).resolve()
    if destination.exists():
        raise ValueError(f'Refusing to overwrite {destination}; choose a new directory')
    shutil.copytree(ROOT / 'projects' / tool, destination,
                    ignore=shutil.ignore_patterns('target', 'build', '.gradle', '.git'))
    write_json(destination / MARKER, {'tool': tool, 'scenario': 'baseline'})
    with (destination / '.gitignore').open('a') as f:
        f.write(MARKER + '\n')
    if git:
        for args in [['init', '-q'], ['add', '.'],
                     ['-c', 'user.name=Fixture Runner', '-c', 'user.email=fixtures@example.invalid',
                      'commit', '--no-gpg-sign', '-qm', 'Fixture baseline']]:
            subprocess.run(['git', *args], cwd=destination, check=True)
    return destination


def apply_scenario(workspace, name):
    workspace = Path(workspace).resolve()
    marker = json.loads((workspace / MARKER).read_text())
    if marker['scenario'] != 'baseline':
        raise ValueError('Scenarios must start from a fresh baseline workspace')
    edits = []
    for change in SCENARIOS[name]['changes']:
        if change.get('tool', marker['tool']) != marker['tool']:
            continue
        path = (workspace / change['path']).resolve()
        if workspace not in path.parents:
            raise ValueError('Invalid mutation path')
        if 'create' in change:
            if path.exists():
                raise ValueError(f'Creation target already exists: {path}')
            edits.append((path, change['create']))
        elif change.get('delete'):
            if not path.is_file():
                raise ValueError(f'Deletion target is absent: {path}')
            edits.append((path, None))
        else:
            text = path.read_text()
            if text.count(change['before']) != 1:
                raise ValueError(f'Mutation needs one exact match: {path}')
            edits.append((path, text.replace(change['before'], change['after'])))
    for path, text in edits:
        if text is None:
            path.unlink()
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(text)
    marker['scenario'] = name
    write_json(workspace / MARKER, marker)


def inventory(name):
    result = set(CATALOG['baseline_tests'])
    if name != 'baseline':
        result.update(SCENARIOS[name]['required'])
    return result


def check_selection(name, payload, exact=False):
    available = inventory(name)
    required = set(SCENARIOS[name]['required'])
    mode = payload.get('mode')
    listed = payload.get('tests', [])
    if mode not in ('ALL', 'SUBSET', 'NONE', 'MODULES') or not isinstance(listed, list):
        raise ValueError('Expected mode ALL/SUBSET/NONE/MODULES and a tests array')
    if any(not isinstance(x, str) for x in listed) or len(listed) != len(set(listed)):
        raise ValueError('Test IDs must be unique strings')
    if mode in ('ALL', 'NONE') and listed:
        raise ValueError('ALL and NONE must have an empty tests array')
    if mode == 'SUBSET' and not listed:
        raise ValueError('Use NONE for an empty selection')
    selected = available if mode == 'ALL' else set(listed) if mode == 'SUBSET' else set()
    if mode == 'MODULES':
        modules = payload.get('modules')
        known_modules = {test.split(':', 1)[0] for test in available}
        if (listed or not isinstance(modules, list) or not modules
                or any(not isinstance(m, str) or m not in known_modules for m in modules)
                or len(modules) != len(set(modules))):
            raise ValueError('MODULES requires unique known modules and no tests array entries')
        selected = {test for test in available if test.split(':', 1)[0] in modules}
    unknown = selected - available
    missed = required - selected
    extra = selected - required
    return {'ok': not unknown and not missed and (not exact or not extra),
            'missing': sorted(missed), 'unknown': sorted(unknown),
            'extra': sorted(extra), 'selected': sorted(selected),
            'required': sorted(required), 'exact': exact}


def read_reports(workspace, tool):
    executed, failed, skipped = set(), set(), set()
    cases = 0
    for module in ('pricing', 'checkout', 'runtime'):
        for suite, folder in [('unit', 'surefire-reports' if tool == 'maven' else 'test'),
                              ('integration', 'failsafe-reports' if tool == 'maven' else 'integrationTest')]:
            base = workspace / module / ('target' if tool == 'maven' else 'build/test-results') / folder
            for path in base.glob('TEST-*.xml'):
                tree = ET.parse(path)
                for case in tree.iter('testcase'):
                    test_id = f'{module}:{suite}:{case.attrib["classname"]}'
                    if case.find('skipped') is not None:
                        skipped.add(test_id)
                        continue
                    cases += 1
                    executed.add(test_id)
                    if case.find('failure') is not None or case.find('error') is not None:
                        failed.add(test_id)
    return dict(executed=sorted(executed), failed=sorted(failed), skipped=sorted(skipped), cases=cases)


def run_full(tool, scenario, executable, output_dir, selector=None):
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=f'impact-{tool}-{scenario}-') as tmp:
        workspace = prepare(tool, Path(tmp) / 'project', git=selector is not None)
        if scenario != 'baseline':
            apply_scenario(workspace, scenario)
        if tool == 'maven':
            args = [executable, '-B', '-ntp', 'clean', 'verify', '-Dmaven.test.failure.ignore=true']
        else:
            args = [executable, '--no-daemon', '--console=plain', 'clean', 'check',
                    '-PfixtureIgnoreFailures=true']
        selection_path = output_dir / f'{tool}-{scenario}-selection.json'
        if selector:
            args = [selector, 'run', '--workspace', str(workspace),
                    '--executable', executable, '--output', str(selection_path),
                    *(['--full'] if scenario == 'baseline' else ['--base', 'HEAD']),
                    '--', args[-1]]
        log_path = output_dir / f'{tool}-{scenario}.log'
        with log_path.open('w') as log:
            process = subprocess.run(args, cwd=workspace, stdout=log, stderr=subprocess.STDOUT)
        reports = read_reports(workspace, tool)
        expected = set() if scenario == 'baseline' else set(SCENARIOS[scenario]['expected_failures'])
        actual = set(reports['failed'])
        selected = inventory(scenario)
        selection_ok = True
        if selector and selection_path.exists():
            payload = json.loads(selection_path.read_text())
            if scenario != 'baseline':
                checked = check_selection(scenario, payload)
                selection_ok = checked['ok']
                selected = set(checked['selected'])
            else:
                selection_ok = payload['mode'] == 'ALL'
        elif selector:
            selection_ok = False
        missing_tests = selected - set(reports['executed'])
        unexpected_tests = set(reports['executed']) - selected
        expected_cases = len(selected) + ('checkout:unit:example.ParameterizedCheckoutTest' in selected)
        ok = (selection_ok and process.returncode == 0 and actual == expected and not missing_tests
              and not unexpected_tests and reports['cases'] == expected_cases and not reports['skipped'])
        result = dict(tool=tool, scenario=scenario, ok=ok, build_exit=process.returncode, selection_ok=selection_ok,
                      expected_failures=sorted(expected), expected_cases=expected_cases, missing_tests=sorted(missing_tests),
                      unexpected_tests=sorted(unexpected_tests), **reports)
        write_json(output_dir / f'{tool}-{scenario}.json', result)
        print(f'{tool:6} {scenario:24} {"PASS" if ok else "FAIL"} '
              f'({reports["cases"]} cases, {len(actual)} failing classes)', flush=True)
        if not ok:
            print(f'Inspect {log_path}', file=sys.stderr)
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    commands.add_parser('list')
    p = commands.add_parser('prepare')
    p.add_argument('--tool', required=True, choices=['maven', 'gradle'])
    p.add_argument('--dest', required=True)
    p.add_argument('--git', action='store_true')
    p = commands.add_parser('apply')
    p.add_argument('scenario', choices=SCENARIOS)
    p.add_argument('--workspace', required=True)
    p = commands.add_parser('check-selection')
    p.add_argument('scenario', choices=SCENARIOS)
    p.add_argument('--actual', required=True)
    p.add_argument('--exact', action='store_true', help='Reject additional selected classes')
    p = commands.add_parser('verify')
    p.add_argument('--tool', choices=['maven', 'gradle', 'both'], default='both')
    p.add_argument('--scenario', default='baseline', choices=['baseline', 'all', *SCENARIOS])
    p.add_argument('--maven', default='mvn')
    p.add_argument('--gradle', default='gradle')
    p.add_argument('--selector', help='Rust binary: validate selection and actual filtered execution')
    p.add_argument('--output', default=str(ROOT / 'validation-results'))
    p = commands.add_parser('reports')
    p.add_argument('--workspace', required=True)
    p.add_argument('--tool', required=True, choices=['maven', 'gradle'])
    args = parser.parse_args()
    if args.command == 'list':
        for scenario in SCENARIOS.values():
            print(f'{scenario["id"]:24} {scenario["description"]}')
    elif args.command == 'prepare':
        print(prepare(args.tool, args.dest, args.git))
    elif args.command == 'apply':
        apply_scenario(args.workspace, args.scenario)
        print(f'Applied {args.scenario}')
    elif args.command == 'check-selection':
        result = check_selection(args.scenario, json.loads(Path(args.actual).read_text()), args.exact)
        print(json.dumps(result, indent=2))
        return 0 if result['ok'] else 1
    elif args.command == 'reports':
        print(json.dumps(read_reports(Path(args.workspace), args.tool), indent=2))
    elif args.command == 'verify':
        tools = ['maven', 'gradle'] if args.tool == 'both' else [args.tool]
        scenarios = ['baseline', *SCENARIOS] if args.scenario == 'all' else [args.scenario]
        results = []
        for tool in tools:
            exe = shutil.which(getattr(args, tool))
            if exe is None:
                raise ValueError(f'{tool} executable not found: {getattr(args, tool)}')
            for scenario in scenarios:
                results.append(run_full(tool, scenario, exe, Path(args.output).resolve(),
                                        str(Path(args.selector).resolve()) if args.selector else None))
        return 0 if all(r['ok'] for r in results) else 1
    return 0

if __name__ == '__main__':
    try:
        sys.exit(main())
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(f'Error: {error}', file=sys.stderr)
        sys.exit(2)
