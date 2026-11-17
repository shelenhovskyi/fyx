//! The counter's contract (crate doc), run on the built component the way Fyx runs it.
//! `fyx-ext test --path extensions/fyx_demo_counter` builds it and sets `FYX_EXT_COMPONENT`; a
//! plain `cargo test` skips these tests.

use fyx_ext_harness::Harness;

fn harness() -> Option<Harness> {
    match Harness::from_env() {
        Ok(harness) => Some(harness),
        Err(error) if std::env::var_os(fyx_ext_harness::COMPONENT_VARIABLE).is_none() => {
            eprintln!("skipped, run `fyx-ext test`: {error}");
            None
        }
        Err(error) => panic!("{error:#}"),
    }
}

#[test]
fn clicks_patch_the_label_and_the_rows_follow_the_count() {
    let Some(mut harness) = harness() else {
        return;
    };
    harness.mount("counter").expect("mount");
    assert_eq!(harness.text("counter", 2), Some("Count: 0"));

    for _ in 0..3 {
        let outcome = harness.click("counter", "Increment").expect("increment");
        assert!(outcome.touched.contains(&2), "{outcome:?}");
    }
    assert_eq!(harness.text("counter", 2), Some("Count: 3"));

    let rows = harness.rows("counter", 7, 0, 10).expect("rows");
    assert_eq!(rows.len(), 3);

    harness.click("counter", "Reset").expect("reset");
    assert_eq!(harness.text("counter", 2), Some("Count: 0"));
    assert!(harness.rows("counter", 7, 0, 10).expect("rows").is_empty());
}
