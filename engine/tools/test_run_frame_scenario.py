"""CPU-only validation of bounded scenario launch controls; never starts a GPU."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("scenario_runner", Path(__file__).with_name("run-frame-scenario.py"))
runner = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(runner)


class PlayControls(unittest.TestCase):
    def test_wind_limits_match_native_cli_and_omission_preserves_defaults(self):
        self.assertEqual(runner.play_controls(), ([], {}, None))
        args, metadata, _ = runner.play_controls(0.8, 128.06921080000052)
        self.assertEqual(args, ["--wind", "0.8", "--wind-time", "128.06921080000052"])
        self.assertEqual(metadata["wind"], 0.8)
        for wind, time in [(float("nan"), None), (-0.1, None), (1.01, None),
                           (None, float("inf")), (None, -1), (None, 1e9 + 1)]:
            with self.subTest(wind=wind, time=time), self.assertRaises(ValueError):
                runner.play_controls(wind, time)

    def test_script_bytes_are_snapshotted_before_the_source_can_change(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "input with spaces.json"
            original = b'[{"frame":1,"key":"toggle"},{"frame":1,"pointer":[956,958]}]'
            path.write_bytes(original)
            args, metadata, snapshot = runner.play_controls(debug_ui_script=path)
            path.write_text("[]")
            self.assertEqual(args, [])
            self.assertEqual(snapshot, original)
            self.assertEqual(metadata["debug_ui_script"]["sha256"], runner.hashlib.sha256(original).hexdigest())
            self.assertEqual(metadata["debug_ui_script"]["events"], 2)

    def test_rejects_malformed_or_unbounded_replay_events(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "script.json"
            for events in [{}, [{"frame": True, "key": "toggle"}],
                           [{"frame": 4097, "key": "toggle"}],
                           [{"frame": 2, "key": "toggle"}, {"frame": 1, "key": "close"}],
                           [{"frame": 1, "key": "unknown"}],
                           [{"frame": 1, "key": "toggle", "pointer_up": True}],
                           [{"frame": 1, "pointer": [float("nan"), 0]}],
                           [{"frame": 1, "pointer_down": False}],
                           [{"frame": 1, "key": "toggle"}] * 257]:
                path.write_text(json.dumps(events))
                with self.subTest(events=events[:2] if isinstance(events, list) else events), self.assertRaises(ValueError):
                    runner.play_controls(debug_ui_script=path)
            path.write_bytes(b" " * 65_537)
            with self.assertRaises(ValueError):
                runner.play_controls(debug_ui_script=path)


if __name__ == "__main__":
    unittest.main()
