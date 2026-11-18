"""The CLA configuration and the three CLA workflows: triggers, permissions and untrusted input."""

from __future__ import annotations

import importlib.util
import json
import re
import sys
import unittest
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location(
    "cla_check", ROOT / "scripts" / "cla_check.py"
)
assert SPEC is not None and SPEC.loader is not None
cla = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = cla
SPEC.loader.exec_module(cla)

WORKFLOWS = ROOT / ".github" / "workflows"
TRIGGER = WORKFLOWS / "cla.yml"
CHECK = WORKFLOWS / "cla-check.yml"
SIGN = WORKFLOWS / "cla-sign.yml"
CLA_JSON = ROOT / ".github" / "cla.json"


class ConfigTests(unittest.TestCase):
    config = cla.parse_config(json.loads(CLA_JSON.read_text(encoding="utf-8")))

    def test_signatures_live_in_the_separate_repository(self) -> None:
        self.assertEqual(
            (
                self.config.signatures_repo,
                self.config.signatures_branch,
                self.config.signatures_path,
            ),
            ("slopvac/cla-signers", "main", "signatures/v1/cla.json"),
        )

    def test_the_check_runs_belong_to_the_cla_app(self) -> None:
        self.assertEqual(self.config.app_id, 4293783)

    def test_allowlist_is_explicit_with_no_wildcard(self) -> None:
        self.assertEqual(
            self.config.allowlist,
            (
                "dependabot[bot]",
                "renovate[bot]",
                "github-actions[bot]",
                "sjorsr-release-bot[bot]",
                "srobroek",
                "web-flow",
            ),
        )
        self.assertFalse(any("*" in login for login in self.config.allowlist))

    def test_the_version_is_the_one_the_agreement_text_carries(self) -> None:
        self.assertEqual(self.config.version, "harmony-icla-1.2-opt1-v2")
        self.assertEqual(
            cla.signing_phrase(self.config.version),
            "I have read the slopvac CLA version harmony-icla-1.2-opt1-v2 and I hereby sign it.",
        )

    def test_the_configured_hash_is_the_hash_of_the_agreement_file(self) -> None:
        data = (ROOT / self.config.document).read_bytes()
        cla.verify_document(data, self.config)
        self.assertRegex(self.config.document_sha256, r"^[0-9a-f]{64}$")

    def test_contributing_shows_the_phrase_the_script_expects(self) -> None:
        text = (ROOT / "CONTRIBUTING.md").read_text(encoding="utf-8")
        blocks = re.findall(r"```text\n(.*?)\n```", text, re.DOTALL)
        self.assertEqual(blocks, [cla.signing_phrase(self.config.version)])

    def test_the_configured_document_exists(self) -> None:
        self.assertEqual(self.config.document, "CLA.md")
        self.assertTrue((ROOT / self.config.document).is_file())

    def test_the_phrase_names_the_configured_version(self) -> None:
        self.assertTrue(self.config.version)
        self.assertIn(self.config.version, cla.signing_phrase(self.config.version))


def load(path: Path) -> tuple[str, dict[str, Any]]:
    import yaml

    text = path.read_text(encoding="utf-8")
    return text, yaml.safe_load(text)


def steps_of(doc: dict[str, Any]) -> list[dict[str, Any]]:
    return [s for j in doc["jobs"].values() for s in j["steps"]]


class AllWorkflowsTests(unittest.TestCase):
    def test_the_cla_workflows_do_not_use_pull_request_target(self) -> None:
        for path in [TRIGGER, CHECK, SIGN]:
            self.assertNotIn(
                "pull_request_target", path.read_text(encoding="utf-8"), path.name
            )

    def test_cla_actions_are_pinned_by_full_sha(self) -> None:
        for path in [TRIGGER, CHECK, SIGN]:
            for line in re.findall(r"uses:\s*(\S+)", path.read_text(encoding="utf-8")):
                self.assertRegex(line, r"@[0-9a-f]{40}$", path.name)

    def test_no_cla_workflow_interpolates_expressions_in_run_steps(self) -> None:
        for path in [TRIGGER, CHECK, SIGN]:
            _, doc = load(path)
            for step in steps_of(doc):
                self.assertNotIn("${{", step.get("run", ""), path.name)

    def test_untrusted_event_data_reaches_scripts_only_through_the_event_file_or_env(
        self,
    ) -> None:
        # Only repository identity and the default branch may appear in an expression; anything
        # from the pull request, the comment or the triggering run is read by the script.
        allowed = {
            "github.event.repository.default_branch",
            "github.event.repository.name",
        }
        for path in [TRIGGER, CHECK, SIGN]:
            for expression in re.findall(
                r"\$\{\{\s*(.*?)\s*\}\}", path.read_text(encoding="utf-8")
            ):
                for ref in re.findall(r"github\.event\.[\w.]+", expression):
                    self.assertIn(ref, allowed, f"{path.name}: {expression}")

    def test_every_expression_in_an_env_value_is_a_token_or_a_secret(self) -> None:
        for path in [CHECK, SIGN]:
            _, doc = load(path)
            for step in steps_of(doc):
                for value in step.get("env", {}).values():
                    self.assertRegex(
                        str(value),
                        r"^\$\{\{ steps\.\w+\.outputs\.token \}\}$",
                        path.name,
                    )

    def test_the_check_run_name_is_cla_everywhere(self) -> None:
        self.assertEqual(cla.CHECK_NAME, "CLA")
        for path in [CHECK, SIGN]:
            self.assertNotIn("check_name", path.read_text(encoding="utf-8"))

    def test_no_cla_workflow_checks_out_pr_code(self) -> None:
        for path in [TRIGGER, CHECK, SIGN]:
            text = path.read_text(encoding="utf-8")
            for bad in [
                "pull_request.head",
                "head.sha",
                "head.ref",
                "refs/pull/",
                "github.head_ref",
                "merge_commit_sha",
                "github.event.workflow_run",
            ]:
                self.assertNotIn(bad, text, f"{path.name}: {bad}")


class TriggerWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text, cls.doc = load(TRIGGER)

    def test_triggers_are_pull_request_and_merge_group_only(self) -> None:
        on = self.doc[True]  # PyYAML reads the bare key `on` as True
        self.assertEqual(set(on), {"pull_request", "merge_group"})
        self.assertEqual(
            on["pull_request"]["types"], ["opened", "synchronize", "reopened"]
        )

    def test_workflow_is_named_cla_so_the_checker_can_follow_it(self) -> None:
        self.assertEqual(self.doc["name"], "CLA")

    def test_nothing_trusted_runs_here(self) -> None:
        self.assertEqual(self.doc["permissions"], {})
        self.assertEqual(len(self.doc["jobs"]), 1)
        job = next(iter(self.doc["jobs"].values()))
        self.assertEqual(job["permissions"], {})
        self.assertNotIn("secrets.", self.text)
        self.assertNotIn("actions/checkout", self.text)
        self.assertNotIn("uses", " ".join(s.keys().__str__() for s in job["steps"]))

    def test_no_job_is_named_cla(self) -> None:
        # only the app's check run may carry the required name "CLA"
        for key, job in self.doc["jobs"].items():
            self.assertNotEqual(job.get("name", key), "CLA")
        self.assertEqual(next(iter(self.doc["jobs"].values()))["name"], "CLA trigger")


class CheckWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text, cls.doc = load(CHECK)

    def test_it_runs_only_after_the_cla_trigger_workflow(self) -> None:
        on = self.doc[True]
        self.assertEqual(set(on), {"workflow_run"})
        self.assertEqual(on["workflow_run"]["workflows"], ["CLA"])
        self.assertEqual(on["workflow_run"]["types"], ["completed"])

    def test_it_checks_out_the_default_branch_only_without_credentials(self) -> None:
        checkouts = [
            s
            for s in steps_of(self.doc)
            if str(s.get("uses", "")).startswith("actions/checkout@")
        ]
        self.assertEqual(len(checkouts), 1)
        self.assertEqual(
            checkouts[0]["with"]["ref"], "${{ github.event.repository.default_branch }}"
        )
        self.assertIs(checkouts[0]["with"]["persist-credentials"], False)

    def test_permissions_are_read_only_and_the_token_can_write_checks(self) -> None:
        self.assertEqual(self.doc["permissions"], {"contents": "read"})
        mint = next(
            s
            for s in steps_of(self.doc)
            if str(s.get("uses", "")).startswith("actions/create-github-app-token@")
        )
        self.assertEqual(mint["with"]["client-id"], "${{ secrets.CLA_APP_CLIENT_ID }}")
        self.assertEqual(
            mint["with"]["private-key"], "${{ secrets.CLA_APP_PRIVATE_KEY }}"
        )
        self.assertEqual(mint["with"]["permission-checks"], "write")
        self.assertEqual(mint["with"]["permission-pull-requests"], "read")
        self.assertEqual(mint["with"]["permission-contents"], "read")
        perms = {k for k in mint["with"] if k.startswith("permission-")}
        self.assertEqual(
            perms,
            {"permission-checks", "permission-pull-requests", "permission-contents"},
        )

    def test_the_script_gets_its_token_from_the_mint_step(self) -> None:
        run = next(s for s in steps_of(self.doc) if "run" in s)
        self.assertIn("scripts/cla_check.py", run["run"])
        self.assertIn("steps.", run["env"]["GITHUB_TOKEN"])


class SignWorkflowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.text, cls.doc = load(SIGN)

    def test_it_runs_on_comments_only_with_edits_and_deletes(self) -> None:
        on = self.doc[True]
        self.assertEqual(set(on), {"issue_comment"})
        self.assertEqual(on["issue_comment"]["types"], ["created", "edited", "deleted"])

    def test_sign_job_has_no_shared_concurrency_slot(self) -> None:
        # a fixed group lets a newer pending run replace a queued revocation
        for job in self.doc["jobs"].values():
            self.assertNotIn("concurrency", job)
        self.assertNotIn("concurrency", self.doc)

    def test_checkouts_do_not_persist_credentials(self) -> None:
        for step in steps_of(self.doc):
            if str(step.get("uses", "")).startswith("actions/checkout"):
                self.assertIs(step["with"]["persist-credentials"], False)
                self.assertEqual(
                    step["with"]["ref"], "${{ github.event.repository.default_branch }}"
                )

    def test_app_tokens_use_secrets_and_the_client_id_input(self) -> None:
        mints = [
            s
            for s in steps_of(self.doc)
            if str(s.get("uses", "")).startswith("actions/create-github-app-token@")
        ]
        self.assertEqual(len(mints), 2)
        for step in mints:
            self.assertEqual(
                step["with"]["client-id"], "${{ secrets.CLA_APP_CLIENT_ID }}"
            )
            self.assertEqual(
                step["with"]["private-key"], "${{ secrets.CLA_APP_PRIVATE_KEY }}"
            )
            self.assertNotIn("app-id", step["with"])

    def test_signatures_token_is_scoped_to_the_signatures_repo(self) -> None:
        step = next(s for s in steps_of(self.doc) if s.get("id") == "signatures")
        self.assertEqual(step["with"]["owner"], "slopvac")
        self.assertEqual(step["with"]["repositories"], "cla-signers")
        self.assertEqual(
            {k for k in step["with"] if k.startswith("permission-")},
            {"permission-contents"},
        )
        self.assertEqual(step["with"]["permission-contents"], "write")

    def test_repo_token_writes_checks_and_nothing_else(self) -> None:
        step = next(s for s in steps_of(self.doc) if s.get("id") == "repo")
        perms = {k: v for k, v in step["with"].items() if k.startswith("permission-")}
        self.assertEqual(
            perms, {"permission-checks": "write", "permission-pull-requests": "read"}
        )
