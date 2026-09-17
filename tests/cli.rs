use std::path::PathBuf;
use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_auctionclear"))
        .args(args)
        .output()
        .expect("binary runs")
}

fn temp_file(name: &str, contents: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("auctionclear-{}-{name}", std::process::id()));
    std::fs::write(&path, contents).unwrap();
    path
}

#[test]
fn run_prints_price_volume_imbalance_and_fills() {
    let path = temp_file(
        "run.csv",
        "# reference: 100\nid,side,price,qty,time\n1,buy,101,300,0\n2,buy,100,200,1\n3,sell,99,400,2\n4,sell,market,50,3\n",
    );
    let out = cli(&["run", path.to_str().unwrap(), "--venue", "xetra"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("price      100\n"), "{text}");
    assert!(text.contains("volume     450\n"), "{text}");
    assert!(text.contains("imbalance  +50 (buy surplus)\n"), "{text}");
    let fill_of = |id: &str| {
        text.lines()
            .find(|l| l.split_whitespace().next() == Some(id))
            .and_then(|l| l.split_whitespace().last())
            .map(str::to_string)
    };
    assert_eq!(fill_of("2").as_deref(), Some("150"));
    assert_eq!(fill_of("4").as_deref(), Some("50"));

    // --reference overrides the file; --levels prints the candidate table.
    let out = cli(&[
        "run",
        path.to_str().unwrap(),
        "--venue",
        "sse",
        "--reference",
        "0",
        "--levels",
    ]);
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("reference  0\n") && text.contains("eligible"),
        "{text}"
    );
}

#[test]
fn gen_output_runs_under_every_venue() {
    let out = cli(&[
        "gen",
        "--profile",
        "tie-heavy",
        "--orders",
        "15",
        "--seed",
        "3",
    ]);
    assert!(out.status.success());
    let path = temp_file("gen.csv", &String::from_utf8(out.stdout).unwrap());
    for venue in ["sse", "szse", "nasdaq", "xetra"] {
        let out = cli(&["run", path.to_str().unwrap(), "--venue", venue]);
        assert!(
            out.status.success(),
            "{venue}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(String::from_utf8(out.stdout)
            .unwrap()
            .contains("\nprice      "));
    }
}

#[test]
fn bad_input_exits_with_an_error() {
    let out = cli(&["run", "/nonexistent/orders.csv", "--venue", "szse"]);
    assert_eq!(out.status.code(), Some(2));
    let path = temp_file("bad.csv", "1,buy,10,1,0\n");
    let out = cli(&["run", path.to_str().unwrap(), "--venue", "tokyo"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("unknown venue"));
    let path = temp_file("bad2.csv", "1,buy,10,x,0\n");
    let out = cli(&["run", path.to_str().unwrap(), "--venue", "sse"]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("line 1"));
    assert_eq!(cli(&["frobnicate"]).status.code(), Some(2));
}

#[test]
fn venues_lists_every_preset_with_its_rule_text() {
    let text = String::from_utf8(cli(&["venues"]).stdout).unwrap();
    for name in ["sse", "szse", "nasdaq", "xetra"] {
        assert!(text.contains(&format!("{name}\n  chain:")), "{text}");
    }
    assert!(text
        .contains("MaxVolume -> MinAbsImbalance -> ImbalanceSide -> Eligible -> ClampReference"));
}
