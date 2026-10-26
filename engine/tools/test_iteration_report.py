"""CPU-only artifact/measurement tests; no game or subprocess launch."""
import importlib.util
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import tempfile
import unittest


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), Path(__file__).with_name(name + ".py"))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


report = module("iteration-report")
runner = module("run-frame-scenario")


class IterationEvidence(unittest.TestCase):
    def write(self, path, value):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value), encoding="utf-8")

    def test_cumulative_startup_events_and_nested_constructor_are_never_added(self):
        constructor = {"kind": "renderer_startup", "total_cpu_ms": 200, "sections": [{"name": "textures", "cpu_ms": 150}]}
        result = report.startup_summary([
            {"kind": "cli_startup", "event": "pre_window", "elapsed_ms": 100, "partition_end_ms": 100, "startup_complete": False},
            constructor, constructor,
            {"kind": "cli_startup", "event": "first_scene_frame", "elapsed_ms": 500, "partition_end_ms": 500, "startup_complete": True},
            {"kind": "cli_startup", "event": "run_end", "elapsed_ms": 50000, "partition_end_ms": 500, "startup_complete": True}])
        self.assertEqual(result["cli_startup_ms"], 500)
        self.assertEqual(result["first_renderer_cpu_ms"], 200)
        self.assertEqual(len(result["renderer_constructors"]), 1)
        self.assertNotIn("total", result)

    def test_unresolved_startup_does_not_claim_first_scene_time(self):
        result = report.startup_summary([{"kind": "cli_startup", "event": "pre_window", "elapsed_ms": 20, "partition_end_ms": 20, "startup_complete": False}])
        self.assertIsNone(result["cli_startup_ms"])
        self.assertIsNone(result["first_renderer_cpu_ms"])

    def test_dev_workflows_keep_build_and_startup_scopes_separate(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "run-a" / "iteration.json"
            self.write(path, {"status": "visible_first_frame", "build_seconds": 2.5, "startup_seconds": 4,
                              "arguments": ["play", "--views", "1", "--state-out", "A.json"]})
            row = report.load_run(path, "dev")
            self.assertEqual(row["timing_ms"]["build"], 2500)
            self.assertEqual(row["timing_ms"]["startup_to_visible_poll"], 4000)
            self.assertIsNone(row["timing_ms"]["first_renderer_constructor"])
            self.assertEqual(row["cache_snapshot"], {})

    def test_same_config_different_output_paths_match_but_actual_size_change_separates(self):
        with tempfile.TemporaryDirectory() as d:
            rows = []
            for name, size in [("a", [3840, 2160]), ("b", [3840, 2160]), ("c", [3840, 2019])]:
                path = Path(d) / name / "iteration.json"
                self.write(path, {"arguments": ["play", "--views", "1", "--state-out", name + ".json"]})
                (path.parent / "frame-profile.jsonl").write_text(json.dumps({"kind": "gpu_frame", "output_size": size, "views": 1}) + "\n")
                rows.append(report.load_run(path, "dev"))
            self.assertEqual(rows[0]["configuration_id"], rows[1]["configuration_id"])
            self.assertNotEqual(rows[0]["configuration_id"], rows[2]["configuration_id"])

    def test_profile_keeps_cache_snapshots_and_actual_capture_cost_not_repeated_sums(self):
        with tempfile.TemporaryDirectory() as d:
            directory = Path(d)
            records = [{"kind": "cpu_frame", "frame_id": 1, "sections_ms": {"diagnostic_readback": 2300, "physics": 4},
                        "foliage_sun": {"resident_pages": 151, "updated_pages": 8}},
                       {"kind": "cpu_frame", "frame_id": 2, "sections_ms": {"physics": 4},
                        "foliage_sun": {"resident_pages": 151, "updated_pages": 0}},
                       {"kind": "gpu_frame", "views": 1, "output_size": [3840, 2019]}]
            (directory / "frames.jsonl").write_text("\n".join(json.dumps(row) for row in records))
            result = report.profile_evidence(directory, [])
            self.assertEqual(result["capture_readback_ms"], 2300)
            self.assertEqual(result["cache_records"], 2)
            self.assertEqual(result["cache_last"]["foliage_sun"]["resident_pages"], 151)
            self.assertEqual(result["cache_last"]["foliage_sun"]["updated_pages"], 0)

    def test_explicit_roots_deduplicate_and_do_not_average_workflows(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            self.write(root / "a" / "iteration.json", {"build_seconds": 1, "profile": "dev"})
            self.write(root / "b" / "iteration.json", {"build_seconds": 99, "profile": "release"})
            self.write(root / "c" / "run.json", {"scenario_sha256": "abc", "exit_code": 0, "completed_expected_frames": True, "completed_expected_captures": True})
            result = report.collect([root, root], [root])
            self.assertEqual(len(result["runs"]), 3)
            self.assertEqual(len(result["configuration_groups"]), 3)
            self.assertNotIn("mean_build_ms", result)

    def test_html_escapes_logged_data(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "a" / "iteration.json"
            self.write(path, {"status": "<script>bad</script>"})
            page = report.render_html(report.collect([Path(d)], []))
            self.assertNotIn("<script>", page)
            self.assertIn("&lt;script&gt;", page)

    def test_capture_parser_distinguishes_preview_dimensions_from_render_output(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "test.jpg"
            path.write_bytes(b'\xff\xd8\xff\xe0\x00\x04xx\xff\xc0\x00\x08\x08' + struct.pack(">HH", 900, 1600) + b'\x00')
            self.assertEqual(runner.image_size(path), [1600, 900])
            path = Path(d) / "test.png"
            path.write_bytes(b'\x89PNG\r\n\x1a\n' + b'\x00\x00\x00\x0dIHDR' + struct.pack(">II", 3840, 2160))
            self.assertEqual(runner.image_size(path), [3840, 2160])

    def test_startup_log_retains_both_kinds_while_ignoring_partial_records(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "stderr.log"
            path.write_text('hello\n{"kind":"cli_startup","event":"pre_window"}\n{"kind":"renderer_startup","total_cpu_ms":4}\n{"kind":')
            self.assertEqual(len(runner.startup_records(path)), 2)

    def test_override_registry_matches_renderer_without_scanning_source_directories(self):
        source = (Path(__file__).resolve().parents[1] / "crates/render/src/shaders.rs").read_text(encoding="utf-8")
        registry = source.split("const BUILT_IN:", 1)[1].split("];", 1)[0]
        names = re.findall(r'\("([^"/]+\.wgsl)",\s*include_str!', registry)
        self.assertEqual(sorted(names), sorted(runner.SHADER_OVERRIDE_NAMES))
        self.assertEqual(len(names), len(set(names)))

    def test_same_directory_content_change_is_detected_even_when_mtime_unchanged(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "shade.wgsl"
            path.write_bytes(b"// first\r\n")
            env = {"KERN_SHADER_DIR": d}
            first = runner.shader_provenance(env, Path(d), "exe-a")
            self.assertEqual(first, runner.shader_provenance(env, Path(d), "exe-a"))
            times = (path.stat().st_atime_ns, path.stat().st_mtime_ns)
            path.write_bytes(b"// other\r\n")
            os.utime(path, ns=times)
            changed = runner.shader_provenance(env, Path(d), "exe-a")
            self.assertNotEqual(first["content_sha256"], changed["content_sha256"])
            shade = next(f for f in changed["files"] if f["name"] == "shade.wgsl")
            self.assertEqual(shade["sha256"], hashlib.sha256(b"// other\r\n").hexdigest())
            self.assertTrue(changed["verified"])

    def test_missing_directory_partial_fallback_and_unrelated_files_are_explicit(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            absent = runner.shader_provenance({"KERN_SHADER_DIR": "absent"}, root, "exe-a")
            embedded = runner.shader_provenance({}, root, "exe-a")
            self.assertEqual(absent["directory_status"], "missing_or_not_directory")
            self.assertEqual(absent["content_sha256"], embedded["content_sha256"])
            self.assertTrue(all(f["source"] == "embedded" for f in absent["files"]))
            self.assertNotEqual(embedded["content_sha256"], runner.shader_provenance({}, root, "exe-b")["content_sha256"])
            (root / "shade.wgsl").write_bytes(b"// only override\n")
            partial = runner.shader_provenance({"KERN_SHADER_DIR": d}, root, "exe-a")
            (root / "unrelated.wgsl").write_bytes(b"\xff do not inspect")
            (root / "private.txt").write_text("unrelated", encoding="utf-8")
            (root / "nested").mkdir()
            (root / "nested/shade.wgsl").write_bytes(b"nested not part of the renderer")
            self.assertEqual(partial, runner.shader_provenance({"KERN_SHADER_DIR": d}, root, "exe-a"))
            self.assertEqual(sum(f["source"] == "override" for f in partial["files"]), 1)
            self.assertEqual(sum(f["source"] == "embedded" for f in partial["files"]), len(runner.SHADER_OVERRIDE_NAMES) - 1)

    def test_invalid_or_oversized_override_is_unverified_not_claimed_embedded(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "shade.wgsl").write_bytes(b"\xff")
            with (root / "far_grass.wgsl").open("wb") as large:
                large.truncate(runner.SHADER_FILE_LIMIT + 1)
            snapshot = runner.shader_provenance({"KERN_SHADER_DIR": d}, root, "exe")
            self.assertFalse(snapshot["verified"])
            bad = {f["name"]: f for f in snapshot["files"] if f["source"] == "unverified"}
            self.assertEqual(bad["shade.wgsl"]["reason"], "UnicodeDecodeError")
            self.assertEqual(bad["far_grass.wgsl"]["reason"], "over_size_limit")
            self.assertTrue(all(f["sha256"] is None for f in bad.values()))

    def test_iteration_groups_distinguish_shader_contents_with_same_executable_and_path(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            shader = root / "shade.wgsl"
            shader.write_text("// A", encoding="utf-8")
            env = {"KERN_SHADER_DIR": d}
            snapshot = runner.shader_provenance(env, root, "same-executable")
            rows = []
            for name, snapshot in [("a", snapshot), ("repeat", snapshot)]:
                path = root / name / "run.json"
                self.write(path, {"scenario_sha256": "same-case", "executable_sha256": "same-executable",
                    "renderer_environment": env, "shader_provenance": snapshot, "shader_provenance_after": snapshot,
                    "shader_provenance_stable": True})
                rows.append(report.load_run(path, "scenario"))
            shader.write_text("// B", encoding="utf-8")
            snapshot = runner.shader_provenance(env, root, "same-executable")
            path = root / "b/run.json"
            self.write(path, {"scenario_sha256": "same-case", "executable_sha256": "same-executable",
                "renderer_environment": env, "shader_provenance": snapshot, "shader_provenance_after": snapshot,
                "shader_provenance_stable": True})
            rows.append(report.load_run(path, "scenario"))
            self.assertEqual(rows[0]["configuration_id"], rows[1]["configuration_id"])
            self.assertNotEqual(rows[0]["configuration_id"], rows[2]["configuration_id"])

    def test_old_or_changed_shader_runs_cannot_silently_group_as_verified(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            rows = []
            for name in ("old-a", "old-b", "changed-a", "changed-b"):
                path = root / name / "run.json"
                manifest = {"scenario_sha256": "same-case", "renderer_environment": {"KERN_SHADER_DIR": "same-path"}}
                if name.startswith("changed"):
                    manifest.update(shader_provenance={"content_sha256": "a"}, shader_provenance_after={"content_sha256": "b"}, shader_provenance_stable=False)
                self.write(path, manifest)
                rows.append(report.load_run(path, "scenario"))
            self.assertEqual(len({r["configuration_id"] for r in rows}), 4)
            self.assertTrue(all(r["warnings"] for r in rows))


if __name__ == "__main__":
    unittest.main()
