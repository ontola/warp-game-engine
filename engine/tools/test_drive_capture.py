import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch

from PIL import Image

spec = importlib.util.spec_from_file_location("drive_capture", Path(__file__).with_name("drive-capture.py"))
capture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(capture)


class RunnerIsolation(unittest.TestCase):
    def test_baseline_and_candidate_have_separate_cargo_outputs(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "drive.json").write_text("{}")
            with patch.dict(capture.os.environ, {"CARGO_TARGET_DIR": "target"}), patch.object(capture.subprocess, "run") as run:
                capture.replay(root, None, root/"capture", root, True)
                baseline = Path(run.call_args.kwargs["env"]["CARGO_TARGET_DIR"])
                capture.replay(capture.ROOT, None, root/"capture", root, True)
                candidate = Path(run.call_args.kwargs["env"]["CARGO_TARGET_DIR"])
                self.assertEqual(baseline, candidate / "warp-compare-baseline")


class PixelInspection(unittest.TestCase):
    def test_marked_capture_uses_original_pixel_without_opening_window(self):
        from argparse import Namespace
        from io import StringIO
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "marked.json"
            path.write_text(json.dumps({"capture": {"size": [100, 50]}, "player_point": [70, 30]}))
            args = Namespace(capture=path, pixel=None, preview=False)
            with patch("sys.stdout", new_callable=StringIO) as out:
                capture.inspect(args)
                self.assertEqual(json.loads(out.getvalue())["player_point"], [70, 30])
            args.pixel = [10, 10]
            with patch("sys.stdout", new_callable=StringIO) as out:
                capture.inspect(args)
                self.assertNotIn("player_point", json.loads(out.getvalue()))

    def test_internal_resolution_and_split_view_origin(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            ids = Image.frombytes("RGBA", (2, 2), struct.pack("<4I", 0xffffffff, 0x80000000, 133, 0))
            ids.save(root / "ids.png")
            (root / "pairs").write_bytes(struct.pack("<4I", 9, 44, 7, 33))
            view = {"rect": [6, 0, 4, 4], "size": [2, 2], "ids": "ids.png", "pairs": "pairs"}
            meta = {"visibility": {"views": [view], "owners": {"7": {"mesh": "tree"}},
                                   "clusters": {"33": {"lod_depth": 4}}}}
            path = root / "feedback.json"
            path.write_text(json.dumps(meta))
            viewer = capture.Inspector(path)
            self.assertEqual(viewer.pick(7, 3), {"instance": 7, "cluster": 33, "triangle": 5,
                                                "mesh": "tree", "lod_depth": 4})
            self.assertEqual(viewer.pick(8, 1)["source"], "voxel visibility")
            self.assertEqual(viewer.pick(7, 0)["source"], "no opaque source triangle")
            self.assertEqual(viewer.pick(5, 0)["source"], "outside captured viewports")


if __name__ == "__main__":
    unittest.main()
