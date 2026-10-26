"""CPU-only measurement-contract regressions; no game, GPU or dependencies."""
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("frame_budget", Path(__file__).with_name("frame-budget.py"))
budget = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(budget)


def cpu(call, frame, start, *, phase="gameplay", presented=True, work=4.0, sections=None):
    return {"kind": "cpu_frame", "call_id": call, "frame_id": frame, "phase": phase,
            "start_ms": start, "end_ms": start + work, "cpu_frame_ms": work,
            "sections_ms": sections if sections is not None else {"physics": 1.0, "render_encode_submit": work - 1},
            "cadence_ms": None, "presented": presented, "diagnostic_sync": False, "error": None}


def gpu(frame, passes, covered=None):
    return {"kind": "gpu_frame", "frame_id": frame, "gpu_pass_ms": passes,
            "instrumented_gpu_ms": sum(p[1] for p in passes), "covered_interval_ms": covered}


class MeasurementContract(unittest.TestCase):
    def test_cpu_execution_excludes_missing_samples_and_never_uses_wall_time(self):
        records=[cpu(1,1,0,work=40),cpu(2,2,40,work=40),cpu(3,3,80,work=40),cpu(4,4,120,work=40)]
        records[0]['cpu_execution_ms']=0.0
        records[1]['cpu_execution_ms']=10.0
        records[2]['cpu_execution_ms']=None
        result=budget.analyze(records)
        self.assertEqual(result['cpu_execution']['n'],2)
        self.assertEqual(result['cpu_execution']['mean_ms'],5)
        self.assertEqual(result['cpu_execution_missing_selected'],2)
        self.assertEqual(result['cpu_work']['mean_ms'],40)
        self.assertEqual([row['cpu_execution_ms'] for row in result['timeline']],[0,10,None,None])
        self.assertIn('CPU main-thread execution',budget.render_html(result))
        old=budget.analyze([cpu(1,1,0,work=40)])
        self.assertIsNone(old['cpu_execution']['mean_ms'])
        self.assertEqual(old['cpu_execution']['n'],0)

    def test_async_join_overlap_missing_and_conditional_pass_denominator(self):
        records = [cpu(1, 10, 0), cpu(2, 11, 8), cpu(3, 12, 28),
                   gpu(12, [["shade", 8], ["raster", 2]], 9),
                   gpu(10, [["shade", 6], ["shadow", 4]], 7),
                   gpu(99, [["unrelated", 100]], 100)]
        result = budget.analyze(records)
        self.assertEqual(result["counts"]["gpu_matched_selected"], 2)
        self.assertEqual(result["counts"]["gpu_missing_selected"], 1)
        self.assertEqual(result["counts"]["gpu_orphan_records"], 1)
        self.assertEqual(result["gpu_covered_interval"]["mean_ms"], 8)
        self.assertEqual(result["gpu_pass_duration_sum"]["mean_ms"], 10)
        self.assertAlmostEqual(result["cadence"]["observed_entry_rate_fps"], 2000 / 28)
        self.assertEqual([r["gpu_covered_ms"] for r in result["timeline"]], [7, None, 9])
        shadow = next(r for r in result["gpu_contributors"] if r["name"] == "shadow")
        self.assertEqual((shadow["observed_n"], shadow["denominator_n"], shadow["mean_ms"]), (1, 2, 2))

    def test_loading_pause_unpresented_and_record_gaps_do_not_invent_gameplay_cadence(self):
        records = [cpu(1, 1, 0), cpu(2, None, 8, phase="stage_loading", work=500),
                   cpu(3, 2, 600), cpu(4, 3, 608), cpu(6, 4, 700),
                   cpu(7, 5, 708, phase="paused"), cpu(8, 6, 716),
                   cpu(9, 7, 724, presented=False), cpu(10, 8, 732)]
        result = budget.analyze(records)
        self.assertEqual(result["cadence"]["n"], 1)
        self.assertEqual(result["cadence"]["mean_ms"], 8)
        self.assertEqual(result["phases"]["stage_loading"]["cpu_work"]["mean_ms"], 500)
        self.assertEqual(result["counts"]["selected_rendered_frames"], 7)
        self.assertEqual(result["counts"]["selected_presented_frames"], 6)

    def test_warmup_requires_both_ends_of_interval_inside_selection(self):
        records = [cpu(1, 1, 0), cpu(2, 2, 500), cpu(3, 3, 508), cpu(4, 4, 517)]
        result = budget.analyze(records, skip_frames=2)
        self.assertEqual(result["cadence"]["n"], 1)
        self.assertEqual(result["cadence"]["mean_ms"], 9)
        self.assertEqual(result["counts"]["selected_rendered_frames"], 2)

    def test_bad_partition_is_not_silently_used_as_additive_cpu_cost(self):
        records = [cpu(1, 1, 0, work=4, sections={"nested_total": 4, "nested_subsection": 3}),
                   cpu(2, 2, 8, work=6)]
        result = budget.analyze(records)
        self.assertEqual(result["cpu_work"]["n"], 2)
        self.assertEqual(result["counts"]["cpu_invalid_section_partitions"], 1)
        self.assertTrue(all(r["denominator_n"] == 1 for r in result["cpu_contributors"]))

    def test_pass_sum_without_covered_interval_never_produces_gpu_fps(self):
        result = budget.analyze([cpu(1, 1, 0), gpu(1, [["a", 10], ["b", 10]])])
        self.assertIsNone(result["gpu_covered_interval"]["p95_ms"])
        self.assertEqual(result["gpu_pass_duration_sum"]["mean_ms"], 20)
        self.assertIsNone(result["cadence"]["observed_entry_rate_fps"])
        self.assertNotIn("fps", result["gpu_pass_duration_sum"])

    def test_legacy_clamped_frame_time_is_not_cadence_and_missing_is_not_zero(self):
        records = [{"kind": "frame", "rendered_frame": 1, "frame_time_ms": 100,
                    "gpu_pass_ms": [["shade", 12]], "timings_available": True},
                   {"kind": "frame", "rendered_frame": 2, "frame_time_ms": 1,
                    "gpu_pass_ms": [], "timings_available": False}]
        result = budget.analyze(records)
        self.assertEqual(result["source_format"], "legacy_gpu_profile")
        self.assertEqual(result["counts"]["gpu_missing_selected"], 1)
        self.assertEqual(result["gpu_pass_duration_sum"]["mean_ms"], 12)
        self.assertIsNone(result["cadence"]["observed_entry_rate_fps"])
        self.assertEqual(result["cpu_work"]["n"], 0)

    def test_percentiles_and_small_sample_denominators(self):
        stat = budget.stats([1, 2, 3, 4], 2.5)
        self.assertEqual(stat["median_ms"], 2.5)
        self.assertAlmostEqual(stat["p95_ms"], 3.85)
        self.assertAlmostEqual(stat["p99_ms"], 3.97)
        self.assertEqual(stat["over_budget_fraction"], .5)
        self.assertIsNone(budget.stats([], 8.33)["over_budget_fraction"])

    def test_drops_are_exposed_and_duplicate_gpu_does_not_change_means(self):
        records = [cpu(1, 1, 0), gpu(1, [["a", 4]], 4), gpu(1, [["a", 900]], 900),
                   {"kind": "gpu_status", "pending": 3, "dropped": 2, "errors": 0},
                   {"kind": "frame_profile_footer", "writer_dropped_records": 5}]
        result = budget.analyze(records)
        self.assertEqual(result["gpu_covered_interval"]["mean_ms"], 4)
        self.assertEqual(result["counts"]["gpu_duplicate_records"], 1)
        self.assertEqual(result["gpu_status_last"]["pending"], 3)
        self.assertEqual(result["footer"]["writer_dropped_records"], 5)
        self.assertTrue(any("dropped records" in message for message in result["warnings"]))

    def test_duplicate_cpu_ids_rejected_instead_of_joining_different_runs(self):
        with self.assertRaisesRegex(ValueError, "unique and increasing"):
            budget.analyze([cpu(1, 1, 0), cpu(1, 2, 8)])

    def test_diagnostic_filter_breaks_cadence_instead_of_hiding_the_stall(self):
        records = [cpu(1, 1, 0), cpu(2, 2, 8), cpu(3, 3, 508), cpu(4, 4, 516)]
        records[1]["diagnostic_sync"] = True
        full = budget.analyze(records)
        clean = budget.analyze(records, exclude_diagnostics=True)
        self.assertEqual(full["cadence"]["n"], 3)
        self.assertEqual(full["cadence"]["max_ms"], 500)
        self.assertEqual(clean["cadence"]["n"], 1)
        self.assertEqual(clean["cadence"]["mean_ms"], 8)
        self.assertEqual(clean["counts"]["excluded_diagnostic_rendered_frames"], 1)
        self.assertEqual(clean["counts"]["cadence_excluded_after_diagnostic"], 1)

    def test_capture_on_last_warmup_frame_does_not_leak_into_clean_cadence(self):
        records = [cpu(1, 1, 0), cpu(2, 2, 2400), cpu(3, 3, 2410), cpu(4, 4, 2450)]
        records[0].update(warmup=True, diagnostic_sync=True)
        result = budget.analyze(records, exclude_diagnostics=True)
        self.assertEqual(result["cadence"]["n"], 2)
        self.assertEqual(result["cadence"]["max_ms"], 40)  # Ordinary over-budget interval remains.
        self.assertEqual(result["counts"]["cadence_excluded_after_diagnostic"], 1)

    def test_actual_gpu_size_overrides_claim_of_requested_cpu_size(self):
        row = cpu(1, 1, 0)
        row.update(views=1, output_size=[3840, 2160])
        measured = gpu(1, [["shade", 2]], 2)
        measured.update(views=1, output_size=[3840, 2019])
        report = budget.analyze([row, measured])
        self.assertEqual(report["actual_gpu_configurations"][0]["output_size"], [3840, 2019])
        self.assertEqual(report["counts"]["cpu_gpu_resolution_mismatch_frames"], 1)

    def test_ring_counts_survive_round_reset_without_adding_repeated_polls(self):
        records = [cpu(1, 1, 0),
                   {"kind": "gpu_status", "round": 1, "dropped": 2, "errors": 1},
                   {"kind": "gpu_status", "round": 1, "dropped": 2, "errors": 1},
                   {"kind": "gpu_status", "round": 2, "dropped": 0, "errors": 0},
                   {"kind": "gpu_status", "round": 2, "dropped": 3, "errors": 0}]
        result = budget.analyze(records)
        self.assertEqual(result["gpu_observed_drops"], 5)
        self.assertEqual(result["gpu_observed_errors"], 1)
        self.assertEqual(len(result["gpu_status_epochs"]), 2)

    def test_backward_clock_does_not_fall_back_to_plausible_fake_cadence(self):
        records = [cpu(1, 1, 10), cpu(2, 2, 9)]
        records[1]["cadence_ms"] = 8
        self.assertEqual(budget.analyze(records)["cadence"]["n"], 0)

    def test_sections_exclude_each_warmup_and_do_not_blend_rendering_modes(self):
        records = []
        for i, (section, work, warmup) in enumerate([
            ("noon", 90, True), ("noon", 4, False), ("noon", 6, False),
            ("sunrise", 200, True), ("sunrise", 12, False), ("sunrise", 16, False)
        ], 1):
            row = cpu(i, i, i * 20, work=work)
            row.update(section=section, warmup=warmup)
            records.extend([row, gpu(i, [["shade", work]], work + 1)])
        result = budget.analyze(records)
        self.assertTrue(result["grouped_only"])
        self.assertIsNone(result["cpu_work"]["median_ms"])
        self.assertEqual([s["cpu_work"]["mean_ms"] for s in result["sections"]], [5, 14])
        self.assertEqual([s["cadence"]["n"] for s in result["sections"]], [1, 1])
        self.assertEqual([s["counts"]["warmup_calls"] for s in result["sections"]], [1, 1])
        selected = budget.analyze(records, section="sunrise")
        self.assertEqual(selected["gpu_covered_interval"]["mean_ms"], 15)
        skipped = budget.analyze(records, skip_frames=1)
        self.assertEqual([s["cpu_work"]["mean_ms"] for s in skipped["sections"]], [6, 16])
        self.assertTrue(all(s["cadence"]["n"] == 0 for s in skipped["sections"]))
        with self.assertRaisesRegex(ValueError, "not present"):
            budget.analyze(records, section="absent")

    def test_cli_writes_portable_report_escapes_metadata_and_preserves_existing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, out = root / "profile.jsonl", root / "report"
            records = [{"kind": "frame_profile_header", "scope": "<script>alert(1)</script>"},
                       cpu(1, 1, 0), gpu(1, [["shade", 2]], 3),
                       {"kind": "frame_profile_footer", "writer_dropped_records": 0}]
            source.write_text("\n".join(json.dumps(r) for r in records), encoding="utf-8")
            self.assertEqual(budget.main([str(source), "--out", str(out)]), 0)
            saved = json.loads((out / "summary.json").read_text(encoding="utf-8"))
            self.assertEqual(saved["counts"]["gpu_matched_selected"], 1)
            page = (out / "report.html").read_text(encoding="utf-8")
            self.assertNotIn("<script>", page)
            self.assertIn("&lt;script&gt;", page)
            self.assertNotIn('<script src=', page)
            self.assertEqual(budget.main([str(source), "--out", str(out)]), 2)

    def test_truncated_or_nonfinite_json_fails_explicitly(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "bad.jsonl"
            for data in ['{"kind":"cpu_frame",', '{"value": NaN}']:
                source.write_text(data, encoding="utf-8")
                with self.assertRaises(ValueError):
                    budget.read_jsonl(source)


if __name__ == "__main__":
    unittest.main()
