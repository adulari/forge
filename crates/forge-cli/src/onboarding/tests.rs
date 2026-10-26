use super::*;

#[test]
fn the_suggested_local_model_fits_the_machine_and_stays_under_32b() {
    let big = crate::local::SystemSpecs {
        total_ram_gb: 128.0,
        cpu_cores: 16,
        os: "linux",
        gpu: None,
        apple_silicon: false,
    };
    let pick = host::suggested_model_for_test(&big).expect("a big machine gets a suggestion");
    assert!(pick.memory_gb <= 128.0);
    let tiny = crate::local::SystemSpecs {
        total_ram_gb: 2.0,
        ..big
    };
    assert!(host::suggested_model_for_test(&tiny).is_none());
}

#[test]
fn the_automatic_trigger_is_a_no_op_under_mock_and_after_its_first_attempt() {
    assert!(maybe_open(true).is_ok());
    AUTO_ATTEMPTED.store(true, Ordering::SeqCst);
    assert!(
        maybe_open(false).is_ok(),
        "already attempted in this process"
    );
    AUTO_ATTEMPTED.store(false, Ordering::SeqCst);
}

#[test]
fn forge_setup_refuses_without_a_terminal_and_names_what_the_user_typed() {
    if stdio_is_terminal() {
        return;
    }
    let err = setup("forge init").unwrap_err().to_string();
    assert!(err.starts_with("`forge init` is interactive"), "{err}");
}
