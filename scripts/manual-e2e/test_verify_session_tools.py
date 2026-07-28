import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

import verify_session_tools as verify


class SessionToolIntegrityTests(unittest.TestCase):
    def test_external_source_audit_checks_executed_tools_without_leaking_arguments(
        self,
    ) -> None:
        findings = verify.external_source_findings(
            12,
            "shell",
            {"command": "curl https://example.test/solution"},
        )

        self.assertEqual(
            {finding["kind"] for finding in findings},
            {"network_cli", "url"},
        )
        self.assertTrue(all("invocation_sha256" in finding for finding in findings))
        self.assertNotIn("example.test", str(findings))

    def test_local_commands_and_file_tools_remain_valid(self) -> None:
        self.assertEqual(
            verify.external_source_findings(
                4,
                "shell",
                {"command": "python -m unittest discover -v"},
            ),
            [],
        )
        self.assertEqual(
            verify.external_source_findings(
                5,
                "read_file",
                {"path": "reservations/service.py"},
            ),
            [],
        )

    def test_external_tool_names_are_rejected(self) -> None:
        findings = verify.external_source_findings(
            7,
            "mcp__github__search_code",
            {"query": "known solution"},
        )

        self.assertEqual([finding["kind"] for finding in findings], ["external_tool"])


if __name__ == "__main__":
    unittest.main()
