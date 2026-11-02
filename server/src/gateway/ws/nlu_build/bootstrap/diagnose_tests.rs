//! 失败归因与运维文案的回归：归因决定状态机怎么走，文案决定运维下一句查什么。

use super::*;

#[test]
fn failure_attribution_prefers_environment_over_training_noise() {
    assert_eq!(
        classify_failure("cannot start Docker CLI (docker.sock + CLI required)", ""),
        "docker_unavailable"
    );
    assert_eq!(
        classify_failure(
            "Docker command failed: exit status 1",
            "Dependency image missing; pulling x"
        ),
        "image_missing"
    );
    assert_eq!(
        classify_failure(
            "Docker command exceeded 3600 seconds; training may require operator cleanup",
            ""
        ),
        "timeout"
    );
    assert_eq!(
        classify_failure("Docker command failed: exit status 137", "Killed"),
        "oom"
    );
    assert_eq!(
        classify_failure(
            "Docker command failed: exit status 1",
            "OSError: hfl/minirbt-h288 is not a local folder and is not a valid model identifier"
        ),
        "weights_unavailable"
    );
    assert_eq!(
        classify_failure("candidate file has invalid size: model.onnx", ""),
        "artifact_invalid"
    );
    assert_eq!(
        classify_failure(
            "candidate labels do not match source intent order plus other",
            ""
        ),
        "labels_mismatch"
    );
    assert_eq!(
        classify_failure("Docker command failed: exit status 1", "Traceback"),
        "build_failed"
    );
}

#[test]
fn every_failure_kind_tells_the_operator_what_to_check() {
    // 文案硬约束：每个码都要能回答"该重建 / 该查模型 / 该查环境"，且不得出现禁语。
    for kind in [
        "smoke_bad_class",
        "smoke_negative",
        "smoke_positive",
        "smoke_truncated",
        "smoke_timeout",
        "smoke_missing",
        "model_load_failed",
        "threshold_changed",
        "labels_mismatch",
        "artifact_invalid",
        "config_invalid",
        "config_no_persistence",
        "adopt_io_error",
        // 手工维护：新增 `next_step` 分支必须同步加进这里，否则新码会绕过禁语检查。
        "build_slot_taken_over",
        "docker_unavailable",
        "image_missing",
        "weights_unavailable",
        "timeout",
        "oom",
    ] {
        let text = next_step(kind);
        assert!(
            text.starts_with("该重建") || text.starts_with("该查"),
            "{kind} 的文案没说下一步查什么: {text}"
        );
        for banned in ["验收通过", "准确率", "可用"] {
            assert!(!text.contains(banned), "{kind} 含禁语 {banned}");
        }
    }
}
