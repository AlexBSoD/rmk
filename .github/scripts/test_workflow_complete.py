"""Exercise full/skip/failure graphs, including event-specific exclusions."""
import copy
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

from workflow_complete import OWNERS, verify


def graph(profile, event='pull_request', esp='["esp32s3"]'):
    result = {job: {'result': 'success', 'outputs': {}} for job in OWNERS[profile]}
    result['plan'] = {'result': 'success', 'outputs': {'run': 'true'}}
    if profile == 'tests':
        result['discover']['outputs']['esp'] = esp
        if not json.loads(esp) or json.loads(esp)[0] is None:
            result['build-esp']['result'] = 'skipped'
        if event != 'pull_request':
            result['bloat-report']['result'] = 'skipped'
    return result


class CompleteTests(unittest.TestCase):
    def test_full_profiles_and_event_exclusions(self):
        for profile in OWNERS:
            for event in ('pull_request', 'push', 'workflow_dispatch'):
                for esp in ('[]', '[null]', '["esp32s3"]'):
                    with self.subTest(profile=profile, event=event, esp=esp):
                        self.assertTrue(verify(profile, graph(profile, event, esp), event))

    def test_every_required_job_rejects_failure_cancelled_and_skip(self):
        for profile in OWNERS:
            for job in OWNERS[profile]:
                for status in ('failure', 'cancelled', 'skipped'):
                    with self.subTest(profile=profile, job=job, status=status):
                        needs = graph(profile)
                        needs[job]['result'] = status
                        with self.assertRaises(ValueError):
                            verify(profile, needs, 'pull_request')

    def test_skip_never_proves_full(self):
        for profile in OWNERS:
            needs = graph(profile)
            needs['plan']['outputs']['run'] = 'false'
            for job in OWNERS[profile]:
                needs[job]['result'] = 'skipped'
            self.assertFalse(verify(profile, needs, 'push'))
            needs['build']['result'] = 'success'
            with self.assertRaises(ValueError):
                verify(profile, needs, 'push')

    def test_plan_failure_and_missing_decision_fail_closed(self):
        for status in ('failure', 'cancelled', 'skipped'):
            needs = graph('tests')
            needs['plan']['result'] = status
            with self.assertRaises(ValueError):
                verify('tests', needs, 'push')
        for value in ('', None, 'yes'):
            needs = graph('tests')
            needs['plan']['outputs']['run'] = value
            with self.assertRaises(ValueError):
                verify('tests', needs, 'push')

    def test_missing_graph_dependency_rejected(self):
        needs = graph('tests')
        del needs['test']
        with self.assertRaises(ValueError):
            verify('tests', needs, 'pull_request')

    def test_esp_discovery_is_not_silently_defaulted(self):
        for value in ('', 'null', '{}', 'not json'):
            needs = graph('tests')
            needs['discover']['outputs']['esp'] = value
            with self.assertRaises((ValueError, TypeError)):
                verify('tests', needs, 'pull_request')

    def test_only_exact_graph_condition_permits_skip(self):
        needs = graph('tests', 'push', '[]')
        self.assertTrue(verify('tests', needs, 'push'))
        for job in ('bloat-report', 'build-esp'):
            changed = copy.deepcopy(needs)
            changed[job]['result'] = 'failure'
            with self.assertRaises(ValueError):
                verify('tests', changed, 'push')

    def test_cli_emits_full_only_on_verified_success(self):
        script = pathlib.Path(__file__).with_name('workflow_complete.py')
        with tempfile.TemporaryDirectory() as tmp:
            output = pathlib.Path(tmp) / 'output'
            summary = pathlib.Path(tmp) / 'summary'
            env = dict(os.environ, CI_NEEDS=json.dumps(graph('firmware')),
                       GITHUB_EVENT_NAME='push', GITHUB_OUTPUT=str(output),
                       GITHUB_STEP_SUMMARY=str(summary))
            result = subprocess.run([sys.executable, str(script), 'firmware'], env=env, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output.read_text(), 'full=true\n')
            self.assertIn('Full owning graph succeeded', summary.read_text())
            output.unlink()
            needs = graph('firmware')
            needs['build']['result'] = 'skipped'
            env['CI_NEEDS'] = json.dumps(needs)
            result = subprocess.run([sys.executable, str(script), 'firmware'], env=env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())
            needs['plan']['outputs']['run'] = 'false'
            for job in OWNERS['firmware']:
                needs[job]['result'] = 'skipped'
            env['CI_NEEDS'] = json.dumps(needs)
            result = subprocess.run([sys.executable, str(script), 'firmware'], env=env, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output.read_text(), 'full=false\n')
            self.assertIn('No new full coverage', summary.read_text())


if __name__ == '__main__':
    unittest.main()
