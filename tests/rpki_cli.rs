use std::process::Command;

fn assert_invalid_historical_selection(args: &[&str], expected_error: &str) {
    let output = Command::new(env!("CARGO_BIN_EXE_monocle"))
        .args(args)
        .output()
        .expect("monocle CLI should run");

    assert!(
        !output.status.success(),
        "expected a nonzero exit status; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(expected_error),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn rejects_invalid_historical_rpki_selections() {
    assert_invalid_historical_selection(
        &[
            "rpki",
            "aspas",
            "--date",
            "2026-06-28",
            "--source",
            "unknown",
        ],
        "Unknown historical RPKI source",
    );
    assert_invalid_historical_selection(
        &[
            "rpki",
            "aspas",
            "--date",
            "2026-06-28",
            "--source",
            "ripe",
            "--collector",
            "sobornost",
        ],
        "not supported with the RIPE historical RPKI source",
    );
    assert_invalid_historical_selection(
        &[
            "rpki",
            "aspas",
            "--date",
            "2026-06-28",
            "--source",
            "rpkispools",
            "--collector",
            "massars",
        ],
        "Unknown RPKISPOOL collector",
    );
}
