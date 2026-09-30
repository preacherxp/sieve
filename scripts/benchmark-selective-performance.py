#!/usr/bin/env python3
"""Compare a full build, ineffective module selection, and opt-in class selection."""
import argparse
import json
from pathlib import Path
import shutil
import statistics
import subprocess
import tempfile
import time
import xml.etree.ElementTree as ET


def main():
    repo = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sieve", default=str(repo / "target/release/sieve"))
    parser.add_argument("--maven", default="mvn")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--delay-ms", type=int, default=6000)
    parser.add_argument("--output", type=Path, default=repo / "validation-results/selective-performance")
    args = parser.parse_args()
    if args.runs < 1 or args.delay_ms < 0:
        parser.error("--runs must be positive and --delay-ms nonnegative")
    sieve = shutil.which(args.sieve)
    maven = shutil.which(args.maven)
    if not sieve or not maven:
        parser.error("build Sieve and provide an available Maven executable")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    summary = output / "summary.json"
    summary.unlink(missing_ok=True)

    def execute(command, root, name):
        started = time.perf_counter()
        with (output / f"{name}.log").open("w") as log:
            log.write(json.dumps(command) + "\n")
            log.flush()
            result = subprocess.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT)
        return result.returncode, time.perf_counter() - started

    def checked(command, root, name):
        code, seconds = execute(command, root, name)
        if code:
            raise RuntimeError(f"{name} failed ({code}); see {output / (name + '.log')}")
        return seconds

    def reports(root):
        cases = []
        failures = []
        for path in sorted((root / "target/surefire-reports").glob("TEST-*.xml")):
            for case in ET.parse(path).getroot().iter("testcase"):
                name = f"{case.attrib['classname']}#{case.attrib['name']}"
                if case.find("skipped") is not None:
                    raise AssertionError(f"unexpected skipped test: {name}")
                cases.append(name)
                if case.find("failure") is not None or case.find("error") is not None:
                    failures.append(name)
        return sorted(cases), sorted(failures)

    price_case = "example.PriceTest#totalIncludesTax"
    all_cases = [price_case, "example.SlowUnrelatedTest#expensiveIndependentSetup"]
    conditions = ["native_full", "module_selected", "class_selected"]
    samples = {condition: [] for condition in conditions}
    with tempfile.TemporaryDirectory(prefix="sieve-performance-") as temp:
        roots = {}
        for condition in conditions:
            root = Path(temp) / condition
            shutil.copytree(repo / "samples/selective-performance", root,
                            ignore=shutil.ignore_patterns("target", "impact.json"))
            checked([sieve, "init", "--workspace", str(root), "--executable", maven], root, f"{condition}-init")
            config_path = root / "impact.json"
            config = json.loads(config_path.read_text())
            assert not config.get("class_level", False), "class selection must be opt-in"
            if condition == "class_selected":
                config["class_level"] = True
                config_path.write_text(json.dumps(config) + "\n")
            for command in [["git", "init", "-q"], ["git", "add", "."],
                            ["git", "-c", "user.name=Sample", "-c", "user.email=sample@example.invalid",
                             "-c", "commit.gpgsign=false", "commit", "-qm", "baseline"]]:
                checked(command, root, f"{condition}-git")
            source = root / "src/main/java/example/Price.java"
            source.write_text(source.read_text().replace("price + tax", "tax + price"))
            roots[condition] = root

        def run(condition, label, base="HEAD", broken=False):
            root = roots[condition]
            selection_path = output / f"{condition}-{label}-selection.json"
            if condition == "native_full":
                command = [maven, "-B", "-ntp", "clean", "verify"]
            else:
                command = [sieve, "run", "--workspace", str(root), "--base", base,
                           "--executable", maven, "--output", str(selection_path), "--"]
            command += [f"-Dsample.delay.ms={args.delay_ms}"]
            code, seconds = execute(command, root, f"{condition}-{label}")
            if (code != 0) != broken:
                raise RuntimeError(f"unexpected exit {code}; see {output / (condition + '-' + label + '.log')}")
            cases, failures = reports(root)
            expected = [price_case] if condition == "class_selected" and base == "HEAD" else all_cases
            assert cases == expected, (condition, cases, expected)
            assert failures == ([price_case] if broken else []), (condition, failures)
            mode = "ALL"
            if condition != "native_full":
                selection = json.loads(selection_path.read_text())
                mode = selection["mode"]
                expected_mode = "ALL" if base != "HEAD" else (
                    "SUBSET" if condition == "class_selected" else "MODULES")
                assert mode == expected_mode, selection
                if mode == "SUBSET":
                    assert selection["tests"] == ["example.PriceTest"], selection
            print(f"{condition} {label}: {seconds:.3f}s, {mode}, {len(cases)} tests", flush=True)
            return {"seconds": seconds, "mode": mode, "executed": cases, "failed": failures, "exit": code}

        # Warm each condition once, then alternate order to reduce cache/order bias.
        for condition in conditions:
            run(condition, "warmup")
        for index in range(args.runs):
            order = conditions if index % 2 == 0 else list(reversed(conditions))
            for condition in order:
                samples[condition].append(run(condition, str(index + 1)))

        fallback = run("class_selected", "missing-base", base="refs/heads/missing-sample-base")
        failure_checks = {}
        for condition in ["native_full", "class_selected"]:
            source = roots[condition] / "src/main/java/example/Price.java"
            source.write_text(source.read_text().replace("tax + price", "tax + price + 1"))
            failure_checks[condition] = run(condition, "failure", broken=True)

    medians = {name: statistics.median(row["seconds"] for row in rows) for name, rows in samples.items()}
    result = {
        "delay_ms": args.delay_ms, "runs": args.runs,
        "maven": subprocess.check_output([maven, "-version"], text=True),
        "sieve": sieve, "samples": samples, "median_seconds": medians,
        "class_saving_vs_native_percent": 100 * (1 - medians["class_selected"] / medians["native_full"]),
        "class_saving_vs_module_percent": 100 * (1 - medians["class_selected"] / medians["module_selected"]),
        "missing_base": fallback, "failure_checks": failure_checks,
    }
    summary.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"median_seconds": medians, "summary": str(summary)}, indent=2))


if __name__ == "__main__":
    main()
