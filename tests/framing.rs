use kittx::{
    ErrorKind,
    framing::{Decoder, FrameEvent},
};

#[test]
fn all_split_boundaries_crlf_null_echo_and_prompt() {
    let transcript = b"AT RV\r\r\n12.\x006V\r\n>";
    for split in 0..=transcript.len() {
        let mut d = Decoder::new(13, 10, 128).unwrap();
        let mut events = Vec::new();
        for chunk in [&transcript[..split], &transcript[split..]] {
            for &b in chunk {
                if let Some(e) = d.push(b).unwrap() {
                    events.push(e);
                }
            }
        }
        assert_eq!(
            events,
            vec![
                FrameEvent::Line("AT RV".into()),
                FrameEvent::Line("12.6V".into()),
                FrameEvent::Prompt
            ]
        );
        assert!(d.pending().is_empty());
    }
}
#[test]
fn custom_terminators_limits_and_unfinished_sleep_ack() {
    let mut d = Decoder::new(b';', b'|', 3).unwrap();
    for b in b"OK" {
        assert!(d.push(*b).unwrap().is_none());
    }
    assert_eq!(d.pending(), b"OK");
    d.clear();
    assert_eq!(d.push(b'A').unwrap(), None);
    assert_eq!(d.push(b';').unwrap(), Some(FrameEvent::Line("A".into())));
    assert_eq!(d.push(b'|').unwrap(), None);
    assert_eq!(d.push(b'>').unwrap(), Some(FrameEvent::Prompt));
    for b in b"123" {
        d.push(*b).unwrap();
    }
    assert_eq!(d.push(b'4').unwrap_err().kind, ErrorKind::BufferOverflow);
    d.clear();
    assert!(d.push(0xFF).is_ok());
    assert!(d.push(b';').is_err());
    for (cr, lf, limit) in [
        (0, 10, 2),
        (b'>', 10, 2),
        (13, b'>', 2),
        (13, 13, 2),
        (13, 10, 0),
    ] {
        assert!(Decoder::new(cr, lf, limit).is_err());
    }
}
#[test]
fn identifier_with_prompt_character_is_not_split() {
    let mut d = Decoder::new(13, 10, 64).unwrap();
    let mut out = Vec::new();
    for b in b"ABC>EFGHIJKL\r>" {
        if let Some(e) = d.push(*b).unwrap() {
            out.push(e);
        }
    }
    assert_eq!(
        out,
        vec![FrameEvent::Line("ABC>EFGHIJKL".into()), FrameEvent::Prompt]
    );
}
