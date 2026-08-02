use super::*;

#[test]
fn unicode_input_and_backspace_preserve_scalar_boundaries() {
    let mut input = SecretInput::new();
    input.push('a').unwrap();
    input.push('é').unwrap();
    input.push('🔒').unwrap();
    assert_eq!(input.character_count(), 3);
    assert!(input.backspace());
    assert_eq!(input.character_count(), 2);
    let response = input.finish();
    response.expose(|bytes| assert_eq!(bytes, "aé".as_bytes()));
}

#[test]
fn fixed_capacity_rejects_overflow_without_partial_input() {
    let mut input = SecretInput::new();
    for _ in 0..MAX_SECRET_BYTES {
        input.push('x').unwrap();
    }
    assert_eq!(input.character_count(), MAX_SECRET_BYTES);
    assert_eq!(input.push('y'), Err(Error::Full));
    assert_eq!(input.character_count(), MAX_SECRET_BYTES);
}

#[test]
fn decoded_fragment_overflow_is_atomic() {
    let mut input = SecretInput::new();
    input.push_text(&"x".repeat(500)).unwrap();
    assert_eq!(input.push_text("ééééééé"), Err(Error::Full));
    assert_eq!(input.character_count(), 500);
    let response = input.finish();
    response.expose(|bytes| assert_eq!(bytes, vec![b'x'; 500]));
}

#[test]
fn clear_erases_content_and_reuses_the_bounded_allocation() {
    let mut input = SecretInput::new();
    input.push('s').unwrap();
    input.push('e').unwrap();
    input.clear();
    assert!(input.is_empty());
    assert!(!input.backspace());
    input.push('n').unwrap();
    input.finish().expose(|bytes| assert_eq!(bytes, b"n"));
}

#[test]
fn diagnostics_never_include_secret_or_length() {
    let mut input = SecretInput::new();
    for character in "private password".chars() {
        input.push(character).unwrap();
    }
    let input_debug = format!("{input:?}");
    let response = input.finish();
    let response_debug = format!("{response:?}");
    for debug in [input_debug, response_debug] {
        assert!(!debug.contains("private password"));
        assert!(!debug.contains("16"));
        assert!(debug.contains("<redacted>"));
    }
}
