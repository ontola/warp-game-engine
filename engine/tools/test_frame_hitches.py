"""CPU-only fixture regressions for cadence selection and delayed GPU joins."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("frame_hitches", Path(__file__).with_name("frame-hitches.py"))
hitches = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(hitches)


def cpu(call, start, **extra):
    return {"kind": "cpu_frame", "call_id": call, "frame_id": 100 + call,
            "phase": "gameplay", "hidden": False, "presented": True,
            "focused": True, "start_ms": start, "cpu_frame_ms": 4, "diagnostic_sync": False,
            "sections_ms": {"physics": 1, "present_acquire": 1, "render_encode_submit": 2,
                            "diagnostic_wait": 0.0002},
            "section": "route", "round": 1, "views": 4, "output_size": [3840, 2160], **extra}


def gpu(frame, cull=1, raster=2, **extra):
    return {"kind": "gpu_frame", "frame_id": frame, "covered_interval_ms": 10,
            "gpu_pass_ms": [["foliage page cull", cull], ["foliage page raster", raster]], **extra}


class HitchReport(unittest.TestCase):
    def test_unconditional_diagnostic_wait_checkpoint_is_not_a_diagnostic(self):
        rows = [cpu(i, i * 10) for i in range(1, 6)]
        clean = hitches.analyze(rows)
        self.assertTrue(clean["gameplay_cadence_evidence"])
        self.assertEqual(clean["cadence"]["n"], 4)
        self.assertNotIn("diagnostic", clean["counts"]["events_nonexclusive"])
        rows[2]["diagnostic_sync"] = True
        rows[2]["sections_ms"]["diagnostic_wait"] = 0
        filtered = hitches.analyze(rows)
        self.assertEqual(filtered["cadence"]["n"], 2)
        self.assertEqual(filtered["counts"]["events_nonexclusive"]["diagnostic"], 1)
        self.assertEqual(filtered["counts"]["events_nonexclusive"]["immediately_after_diagnostic"], 1)

    def test_tails_outliers_and_delayed_gpu_are_aligned_to_preceding_cpu(self):
        rows = [cpu(i + 1, value) for i, value in enumerate([0, 10, 20, 30, 80, 90])]
        rows[3]["cpu_frame_ms"] = 45
        rows[3]["sections_ms"]["render_encode_submit"] = 43
        rows += [gpu(105, 99, 99, received_call_id=7),
                 gpu(104, 5, 8, received_call_id=9, gpu_pass_intervals=[
                     {"name": "foliage page raster", "start_ms": 1, "end_ms": 9, "view_index": None}]),
                 gpu(101), gpu(102), gpu(103), gpu(106)]
        result = hitches.analyze(rows)
        self.assertEqual(result["cadence"]["n"], 5)
        self.assertEqual(result["cadence"]["p50_ms"], 10)
        self.assertAlmostEqual(result["cadence"]["p95_ms"], 42)
        self.assertAlmostEqual(result["cadence"]["p99_ms"], 48.4)
        self.assertEqual(result["cadence"]["max_ms"], 50)
        self.assertEqual(result["cadence"]["over_2x_median_n"], 1)
        self.assertEqual(result["cadence"]["over_33_33_ms_n"], 1)
        outlier = result["outliers"][0]
        self.assertEqual(outlier["preceding_frame"]["frame_id"], 104)
        self.assertEqual(outlier["preceding_frame"]["cpu_sections_ms"]["render_encode_submit"], 43)
        self.assertEqual(outlier["preceding_frame"]["gpu"]["foliage_pass_ms"][1], ["foliage page raster", 8])
        self.assertEqual(outlier["following_frame"]["gpu"]["foliage_pass_ms"][1], ["foliage page raster", 99])
        self.assertIsNone(outlier["preceding_frame"]["gpu"]["foliage_pass_intervals"][0]["view_index"])
        correlation = next(row for row in result["associations"] if row["alignment"] == "preceding_frame" and row["metric"] == "render_encode_submit")
        self.assertAlmostEqual(correlation["pearson_r"], 1)
        self.assertEqual(correlation["at_cadence_outliers"]["p50_ms"], 43)

    def test_hidden_diagnostic_run_has_no_gameplay_cadence_evidence(self):
        rows = [{"kind": "frame_profile_header", "hidden_smoke": True}]
        rows += [cpu(i, i * 50, hidden=True, presented=False, diagnostic_sync=True) for i in range(1, 8)]
        result = hitches.analyze(rows)
        self.assertFalse(result["gameplay_cadence_evidence"])
        self.assertEqual(result["cadence"]["n"], 0)
        self.assertIsNone(result["cadence"]["p99_ms"])
        self.assertEqual(result["counts"]["events_nonexclusive"]["diagnostic"], 7)
        self.assertTrue(any("No gameplay-cadence evidence" in note for note in result["warnings"]))

    def test_loading_pause_warmup_diagnostics_readback_and_following_intervals_excluded(self):
        rows = [cpu(1, 0, phase="startup_loading", frame_id=None),
                cpu(2, 400, warmup=True), cpu(3, 500), cpu(4, 510),
                cpu(5, 520, diagnostic_sync=True), cpu(6, 1500), cpu(7, 1510),
                cpu(8, 1520, sections_ms={"diagnostic_readback": 300}),
                cpu(9, 2000), cpu(10, 2010), cpu(11, 2020, paused=True), cpu(12, 2500)]
        result = hitches.analyze(rows)
        self.assertEqual(result["cadence"]["n"], 3)
        self.assertEqual(result["cadence"]["max_ms"], 10)
        self.assertEqual(result["counts"]["events_nonexclusive"]["immediately_after_diagnostic"], 2)

    def test_section_camera_and_resolution_transition_frames_excluded(self):
        rows = [cpu(1, 0, camera_mode=0), cpu(2, 10, camera_mode=0),
                cpu(3, 600, section="new", camera_mode=0), cpu(4, 900, section="new", camera_mode=0),
                cpu(5, 910, section="new", camera_mode=0), cpu(6, 1700, section="new", camera_mode=1),
                cpu(7, 2000, section="new", camera_mode=1), cpu(8, 2010, section="new", camera_mode=1),
                cpu(9, 2500, section="new", camera_mode=1, output_size=[1920, 1080]),
                cpu(10, 2600, section="new", camera_mode=1, output_size=[1920, 1080]),
                cpu(11, 2610, section="new", camera_mode=1, output_size=[1920, 1080])]
        result = hitches.analyze(rows)
        self.assertEqual(result["cadence"]["n"], 4)
        self.assertEqual(result["cadence"]["max_ms"], 10)
        self.assertEqual(len(result["contexts"]), 4)
        for name in ("section_transition", "camera_transition", "configuration_transition"):
            self.assertEqual(result["counts"]["events_nonexclusive"][name], 1)

    def test_gaps_missing_duplicate_orphan_gpu_and_cumulative_drops_are_counted(self):
        rows = [cpu(1, 0), cpu(2, 10), cpu(4, 1000), cpu(5, 1010),
                gpu(101), gpu(101, 999, 999), gpu(999), gpu(102, cull=-1),
                {"kind": "gpu_status", "round": 1, "dropped": 2, "errors": 0, "pending": 3},
                {"kind": "gpu_status", "round": 1, "dropped": 3, "errors": 0, "pending": 2},
                {"kind": "gpu_status", "round": 2, "dropped": 1, "errors": 1, "pending": 1},
                {"kind": "frame_profile_footer", "writer_dropped_records": 7}]
        result = hitches.analyze(rows)
        self.assertEqual(result["cadence"]["n"], 2)
        self.assertEqual(result["counts"]["cpu_missing_calls_between_records"], 1)
        self.assertEqual(result["counts"]["gpu"]["orphan_records"], 1)
        self.assertEqual(result["counts"]["gpu"]["duplicate_records"], 1)
        self.assertEqual(result["counts"]["gpu"]["invalid_records"], 1)
        self.assertEqual(result["counts"]["gpu"]["missing_selected_frames"], 3)
        self.assertEqual(result["counts"]["gpu"]["observed_dropped_samples"], 4)
        self.assertEqual(result["counts"]["writer_dropped_records"], 7)
        association = next(row for row in result["associations"] if row["metric"] == "foliage page raster" and row["alignment"] == "preceding_frame")
        self.assertEqual(association["paired_n"], 1)
        self.assertIsNone(association["pearson_r"])

    def test_unknown_visibility_and_bad_ids_do_not_invent_cadence(self):
        rows = [cpu(1, 0), cpu(2, 10)]
        for row in rows:
            del row["hidden"]
        self.assertFalse(hitches.analyze(rows)["gameplay_cadence_evidence"])
        with self.assertRaisesRegex(ValueError, "unique increasing"):
            hitches.analyze([cpu(1, 0), cpu(1, 10)])
        with self.assertRaisesRegex(ValueError, "frame IDs"):
            hitches.analyze([cpu(1, 0), cpu(2, 10, frame_id=101)])

    def test_manual_camera_cut_and_additional_warmup_are_repeatable(self):
        rows = [cpu(i, i * 10) for i in range(1, 12)]
        result = hitches.analyze(rows, skip_frames=2, camera_transition_calls=[6])
        self.assertEqual(result["cadence"]["n"], 4)
        self.assertEqual(result["counts"]["excluded_frames_by_reason_nonexclusive"]["skip_frames"], 4)
        with self.assertRaisesRegex(ValueError, "recorded CPU call"):
            hitches.analyze(rows, camera_transition_calls=[99])

    def test_reader_and_cli_write_json_without_overwriting_input(self):
        with tempfile.TemporaryDirectory() as temp:
            source, output = Path(temp) / "frames.jsonl", Path(temp) / "report.json"
            source.write_text("\n".join(json.dumps(cpu(i, i * 10)) for i in range(1, 5)), encoding="utf-8-sig")
            self.assertEqual(hitches.main([str(source), "--out", str(output)]), 0)
            result = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(result["cadence"]["n"], 3)
            self.assertEqual(len(result["input"]["sha256"]), 64)
            self.assertEqual(hitches.main([str(source), "--out", str(source), "--force"]), 2)


if __name__ == "__main__":
    unittest.main()
