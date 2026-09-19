#!/usr/bin/env python3
"""Regression contract: Nolune has no passive screen capture stack."""

from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]


class NoContinuousScreenRecordingTest(unittest.TestCase):
    def assert_tokens_absent(self, relative_path: str, tokens: tuple[str, ...]) -> None:
        source = (ROOT / relative_path).read_text()
        for token in tokens:
            with self.subTest(path=relative_path, token=token):
                self.assertNotIn(token, source)

    def test_legacy_capture_protocol_is_absent_from_production_files(self) -> None:
        contracts = {
            "desktop/src-tauri/src/computer_use_bridge.rs": (
                "ScreenFrame",
                '"screen_frame"',
                "start_recording",
                "stop_recording",
            ),
            "desktop/src-tauri/src/lib.rs": ("screen_recorder",),
            "server/src/services/machine_registry.rs": (
                "ScreenFrame",
                "screen_frame",
                "screen_recording_allowed",
            ),
            "server/src/routes/machine_agents.rs": (
                "live-frame",
                "save_screen_observation",
                "screen_frame",
            ),
            "server/src/services/tools/mod.rs": (
                "CollectScreenRecordingTool",
                "SaveScreenObservationTool",
                "tools::screen",
            ),
            "server/src/services/heartbeat.rs": ("builtin_observer",),
        }
        for path, tokens in contracts.items():
            self.assert_tokens_absent(path, tokens)

    def test_recording_routes_and_overlays_are_absent(self) -> None:
        self.assert_tokens_absent(
            "client/src/routes/[slug]/+layout.svelte",
            ("/observations", "/live", "Observations", "Live screen"),
        )
        for overlay in (
            "client/src/routes/overlay/[slug]/+page.svelte",
            "desktop/src/routes/overlay/+page.svelte",
        ):
            self.assert_tokens_absent(
                overlay,
                ("recording", "pip-rec", "rec-pulse", ">REC<", " REC "),
            )

        removed_paths = (
            ROOT / "desktop/src-tauri/src/screen_recorder.rs",
            ROOT / "server/src/services/tools/screen.rs",
            ROOT / "client/src/lib/components/observations/ObservationsView.svelte",
            ROOT / "client/src/routes/[slug]/observations/+page.svelte",
            ROOT / "client/src/routes/[slug]/live/+page.svelte",
        )
        for path in removed_paths:
            with self.subTest(path=path):
                self.assertFalse(path.exists())

    def test_explicit_computer_use_and_remote_tools_remain(self) -> None:
        desktop_bridge = (ROOT / "desktop/src-tauri/src/computer_use_bridge.rs").read_text()
        server_tools = (ROOT / "server/src/services/tools/mod.rs").read_text()
        self.assertIn('"screenshot" =>', desktop_bridge)
        self.assertIn("ComputerUseTool::new", server_tools)
        self.assertIn("RemoteBashTool::new", server_tools)
        self.assertIn("RemoteFilesTool::new", server_tools)


if __name__ == "__main__":
    unittest.main()
