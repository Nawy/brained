use assert_cmd::Command;

#[test]
fn prints_usage_with_no_args() {
    let mut cmd = Command::cargo_bin("brd").unwrap();
    cmd.assert().failure(); // clap exits non-zero when a subcommand is required but missing
}

#[test]
fn accepts_five_known_subcommands() {
    for sub in ["init", "install", "scan", "mcp", "info"] {
        let mut cmd = Command::cargo_bin("brd").unwrap();
        // stub bodies just need to not fail clap's *parsing* — they may exit non-zero
        // for other reasons (e.g. "not implemented") until later tasks fill them in.
        cmd.arg(sub).arg("--help").assert().success();
    }
}
