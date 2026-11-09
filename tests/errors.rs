use kittx::{Error, ErrorKind, Recovery};
use std::error::Error as _;
#[test]
fn structured_error_context_display_and_cause() {
    let mut error = Error::new(
        ErrorKind::Initialization { step: 2 },
        "failed initialization",
    );
    error.partial = b"?\r>".to_vec();
    error.cause = Some(Box::new(Error::new(
        ErrorKind::Rejected,
        "unsupported command",
    )));
    error.recovery = Recovery::ReconnectRequired;
    assert!(error.to_string().contains("step: 2"));
    assert!(error.source().unwrap().to_string().contains("Rejected"));
    let clone = error.clone();
    assert_eq!(clone, error);
    assert_eq!(
        Error::new(ErrorKind::Timeout, "deadline").recovery,
        Recovery::NotNeeded
    );
}
