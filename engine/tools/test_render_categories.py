"""CPU-only controlled omission comparison contracts; never starts a renderer."""
import importlib.util
import json
import os
from pathlib import Path
import struct
import tempfile
import unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("render_categories", Path(__file__).with_name("render-category-report.py"))
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value), encoding="utf-8")


def fixture(directory, category=None, missing_gpu=()):
    directory.mkdir(parents=True)
    provenance = {"verified": True, "content_sha256": "same-shaders"}
    capture = {"format": "png", "max_width": 0, "quality": 85}
    manifest = {"exit_code": 0, "completed_expected_frames": True, "completed_expected_captures": True,
        "resolution_verified": True, "visible": True, "fullscreen": True, "executable_sha256": "same-executable",
        "scenario_sha256": "same-case", "expected_frames": 8, "actual_output_sizes": [[8, 8]], "capture_config": capture,
        "renderer_environment": {"KERN_DIAGNOSTIC_HIDE": category or "none", "KERN_ADAPTER": "same GPU"},
        "shader_provenance_stable": True, "shader_provenance": provenance, "shader_provenance_after": provenance,
        "captures": [{"frame_id": 8}]}
    save(directory / "run.json", manifest)
    diagnostic = {"enabled": True, "mask": 0 if category is None else 1 << report.CATEGORIES.index(category),
        "categories": [] if category is None else [category], "scope": "color_geometry_only",
        "retained": ["shadows", "GI", "physics", "effects"], "dedicated_passes": {"grass_enabled": category != "grass", "near_grass_present": True, "far_grass_present": True, "transparency_present": True},
        "source_counts": [{"category": name, "instances": 12, "active_instances": 10, "referenced_clusters": 200} for name in report.CATEGORIES],
        "views": [{"view_index": i, "frame_id": 8, "render_sequence": 8,
            "viewport": [(i % 2) * 4, (i // 2) * 4, 4, 4], "input_size": [4, 4], "output_size": [8, 8], "uncovered_pixels": 11,
            "categories": [{"category": name, "instances": 5, "clusters": 20, "triangles": 80, "submitted_vertices": 240, "visibility_pixels": 1} for name in report.CATEGORIES]} for i in range(4)]}
    if category is not None:
        for view in diagnostic["views"]:
            view["uncovered_pixels"] += 1
            row = next(row for row in view["categories"] if row["category"] == category)
            row.update({key: 0 for key in ("instances", "clusters", "triangles", "submitted_vertices", "visibility_pixels")})
    views = [{"position": [i, 2, 3], "target": [i, 1, 0], "fov_y": 1., "rect": [(i % 2) * 4, (i // 2) * 4, 4, 4]} for i in range(4)]
    metadata = {"frame_id": 8, "race_frame": 108, "round": 1, "seed": 1, "stage_noise_seed": 3, "stage_detail_seed": 4,
        "section": "route", "section_frame": 8, "source_output_size": [8, 8], "capture_size": [8, 8],
        "views": views, "player_positions": [[i, 0, 0] for i in range(4)], "sun_direction": [0, 1, 0],
        "sun_lux": 1000, "sky_lux": 200, "ev100": 14, "render_diagnostic": diagnostic}
    save(directory / "captures/frame-000008.json", metadata)
    (directory / "captures/frame-000008.png").write_bytes(b'\x89PNG\r\n\x1a\n' + b'\x00\x00\x00\x0dIHDR' + struct.pack('>II', 8, 8))
    records = [{"kind": "frame_profile_header", "render_diagnostic": {k: diagnostic[k] for k in ("enabled", "mask", "categories", "scope")}}]
    for frame in range(1, 9):
        records.append({"kind": "cpu_frame", "frame_id": frame, "call_id": frame, "race_frame": 100 + frame,
            "target_tick": 100 + frame, "round": 1, "section": "route", "section_frame": frame, "warmup": frame <= 2,
            "views": 4, "output_size": [8, 8], "render_state_hash": f"state-{frame}", "phase": "gameplay",
            "diagnostic_sync": frame in (4, 8), "presented": True, "start_ms": frame * 10, "end_ms": frame * 10 + 4,
            "cpu_frame_ms": 4, "sections_ms": {"physics": 1, "render_encode_submit": 3}})
        if frame not in missing_gpu:
            records.append({"kind": "gpu_frame", "frame_id": frame, "views": 4, "output_size": [8, 8],
                "gpu_pass_ms": [["raster", 2 if category is None else 3], ["shade", 3 if category is None else 1]],
                "covered_interval_ms": 6 if category is None else 4})
    records.append({"kind": "frame_profile_footer", "writer_dropped_records": 0})
    (directory / "frames.jsonl").write_text("\n".join(json.dumps(row) for row in records) + "\n", encoding="utf-8")
    return directory


class Categories(unittest.TestCase):
    def test_matches_frame_ids_excludes_warmup_and_readback_and_retains_negative_pass_delta(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            a = report.load_run(fixture(root / "a"), None)
            b = report.load_run(fixture(root / "b", "trees"), "trees")
            compared = report.compare_runs(a, b, "trees")
            section = compared["sections"][0]
            self.assertEqual(section["matched_frame_ids"], [3, 5, 6, 7])
            self.assertEqual(section["gpu_covered_interval"]["baseline_minus_excluded"]["median_ms"], 2)
            raster = next(p for p in section["gpu_passes"] if p["name"] == "raster")
            self.assertEqual(raster["baseline_minus_excluded"]["median_ms"], -1)
            self.assertEqual(compared["render_state_hash_frames"], 8)
            self.assertEqual(len(compared["excluded"]["captures"][0]["data"]["render_diagnostic"]["views"]), 4)
            self.assertNotIn("total_category_cost", compared)

    def test_missing_gpu_frame_is_not_zero_or_unpaired_timing(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            a = report.load_run(fixture(root / "a"), None)
            b = report.load_run(fixture(root / "b", "grass", missing_gpu=(6,)), "grass")
            section = report.compare_runs(a, b, "grass")["sections"][0]
            self.assertEqual(section["matched_frame_ids"], [3, 5, 7])
            self.assertEqual(section["missing_or_invalid_gpu_pairs"], 1)
            self.assertEqual(section["gpu_covered_interval"]["baseline_minus_excluded"]["n"], 3)

    def test_paired_delta_is_not_difference_of_unpaired_medians(self):
        value = report.delta([10, 1, 10], [9, 0, 2])
        self.assertEqual(value["baseline_minus_excluded"]["median_ms"], 1)
        self.assertNotEqual(value["baseline"]["median_ms"] - value["excluded"]["median_ms"], 1)

    def test_exact_identity_rejects_other_settings_shader_executable_camera_tick_and_state_changes(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            a = report.load_run(fixture(root / "a"), None)
            path = fixture(root / "b", "trees")
            mutations = [lambda b: b["manifest"].update(executable_sha256="different"),
                lambda b: b["manifest"]["renderer_environment"].update(KERN_ADAPTER="other GPU"),
                lambda b: b["manifest"]["shader_provenance"].update(content_sha256="other source"),
                lambda b: b["cpu"][3].update(target_tick=999),
                lambda b: b["cpu"][5].update(render_state_hash="other camera"),
                lambda b: b["captures"][0]["data"]["views"][0].update(position=[9, 9, 9]),
                lambda b: b["captures"][0]["data"]["render_diagnostic"]["source_counts"][0].update(instances=999)]
            for mutate in mutations:
                b = report.load_run(path, "trees")
                mutate(b)
                with self.assertRaises(ValueError):
                    report.compare_runs(a, b, "trees")

    def test_unconfirmed_mask_retained_control_and_missing_view_evidence_fail(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            for i, change in enumerate([{"mask": 0}, {"retained": ["physics"]}, {"views": []}, {"scope": "all_render_passes"}]):
                path = fixture(root / str(i), "cars")
                capture = path / "captures/frame-000008.json"
                data = report.read_json(capture)
                data["render_diagnostic"].update(change)
                save(capture, data)
                with self.assertRaises(ValueError):
                    report.load_run(path, "cars")

    def test_plan_only_emits_explicit_commands_with_one_change_and_no_launch(self):
        with tempfile.TemporaryDirectory() as d, mock.patch.dict(os.environ, {"KERN_ADAPTER": "GPU"}, clear=True), mock.patch.object(report.subprocess, "run") as launch:
            root = Path(d)
            case = root / "case.toml"
            case.write_text("schema=1\n", encoding="utf-8")
            exe = root / "game.exe"
            exe.write_bytes(b"fixture executable, never run")
            env = root / "base.json"
            env.write_text('{"KERN_ADAPTER":"GPU"}', encoding="utf-8-sig")
            out = root / "plan"
            self.assertEqual(report.main(["plan", str(case), "--exe", str(exe), "--env-file", str(env), "--out", str(out), "--categories", "trees,grass"]), 0)
            launch.assert_not_called()
            value = report.read_json(out / "plan.json")
            self.assertEqual(len(value["commands"]), 4)
            environments = [report.read_json(out / f"environment-{name}.json") for name in ("baseline", "trees", "grass", "baseline-repeat")]
            self.assertEqual([e.pop(report.FLAG) for e in environments], ["none", "trees", "grass", "none"])
            self.assertEqual(environments, [{"KERN_ADAPTER": "GPU"}] * 4)
            self.assertEqual((out / "scenario.toml").read_bytes(), case.read_bytes())

    def test_completed_report_preserves_raw_evidence_and_escapes_html(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            a, b, repeat = fixture(root / "a"), fixture(root / "b", "trees"), fixture(root / "repeat")
            out = root / "report"
            self.assertEqual(report.main(["compare", "--baseline", str(a), "--baseline-repeat", str(repeat), "--without", f"trees={b}", "--out", str(out)]), 0)
            data = report.read_json(out / "report.json")
            self.assertEqual(data["baseline_repeat"]["sections"][0]["gpu_covered_interval"]["baseline_minus_excluded"]["median_ms"], 0)
            self.assertEqual(data["comparisons"][0]["baseline"]["manifest"]["executable_sha256"], "same-executable")
            data["comparisons"][0]["category"] = "<script>"
            page = report.render_html(data)
            self.assertNotIn("<script>", page)
            self.assertIn("&lt;script&gt;", page)

    def test_stale_per_view_counter_and_pixel_accounting_are_rejected(self):
        changes = [lambda data: data.update(frame_id=9),
            lambda data: data["render_diagnostic"]["views"][2].update(frame_id=7),
            lambda data: data["render_diagnostic"]["views"][2].update(render_sequence=7),
            lambda data: data["render_diagnostic"]["views"][2].update(view_index=0),
            lambda data: data["render_diagnostic"]["views"][2].update(uncovered_pixels=0),
            lambda data: data["render_diagnostic"]["dedicated_passes"].update(grass_enabled=False)]
        with tempfile.TemporaryDirectory() as d:
            for i, change in enumerate(changes):
                path = fixture(Path(d) / str(i))
                capture = path / "captures/frame-000008.json"
                data = report.read_json(capture)
                change(data)
                save(capture, data)
                with self.assertRaises(ValueError):
                    report.load_run(path, None)

    def test_nonzero_excluded_geometry_or_pixels_cannot_pass_declared_mask(self):
        with tempfile.TemporaryDirectory() as d:
            for index, key in enumerate(("instances", "clusters", "triangles", "submitted_vertices", "visibility_pixels")):
                path = fixture(Path(d) / str(index), "trees")
                capture = path / "captures/frame-000008.json"
                data = report.read_json(capture)
                view = data["render_diagnostic"]["views"][2]
                next(row for row in view["categories"] if row["category"] == "trees")[key] = 1
                if key == "visibility_pixels":
                    view["uncovered_pixels"] -= 1  # Maintain valid accounting: the exclusion itself is broken.
                save(capture, data)
                with self.assertRaisesRegex(ValueError, "excluded trees still"):
                    report.load_run(path, "trees")

    def test_internal_resolution_difference_is_not_a_category_speedup(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            a = report.load_run(fixture(root / "a"), None)
            path = fixture(root / "b", "trees")
            capture = path / "captures/frame-000008.json"
            data = report.read_json(capture)
            data["render_diagnostic"]["views"][1].update(input_size=[8, 8], uncovered_pixels=60)
            save(capture, data)
            b = report.load_run(path, "trees")  # Independently valid capture, different internal resolution.
            with self.assertRaisesRegex(ValueError, "diagnostic input_size differs"):
                report.compare_runs(a, b, "trees")
            data["render_diagnostic"]["views"][1]["output_size"] = [4, 4]
            save(capture, data)
            with self.assertRaisesRegex(ValueError, "output resolution differs"):
                report.load_run(path, "trees")


if __name__ == "__main__":
    unittest.main()
