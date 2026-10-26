//! The parts of a shell session that can be checked without a cluster: what a
//! key sends, and what the screen does with what comes back.

use super::*;

#[test]
fn a_key_sends_what_a_terminal_sends() {
    let plain = Modifiers::NONE;
    // terminals are not keyboards: Enter is a carriage return, Backspace is DEL
    assert_eq!(key_sequence(Key::Enter, plain).unwrap(), b"\r".to_vec());
    assert_eq!(
        key_sequence(Key::Backspace, plain).unwrap(),
        b"\x7f".to_vec()
    );
    assert_eq!(key_sequence(Key::Tab, plain).unwrap(), b"\t".to_vec());
    assert_eq!(key_sequence(Key::Escape, plain).unwrap(), b"\x1b".to_vec());
    assert_eq!(
        key_sequence(Key::ArrowUp, plain).unwrap(),
        b"\x1b[A".to_vec()
    );
    assert_eq!(
        key_sequence(Key::Delete, plain).unwrap(),
        b"\x1b[3~".to_vec()
    );
    // Ctrl+letter is the letter's place in the alphabet: ^C, ^D, ^Z
    assert_eq!(key_sequence(Key::C, Modifiers::CTRL).unwrap(), vec![3]);
    assert_eq!(key_sequence(Key::D, Modifiers::CTRL).unwrap(), vec![4]);
    assert_eq!(key_sequence(Key::Z, Modifiers::CTRL).unwrap(), vec![26]);
    // Ctrl+Arrow is still an arrow: a shell reads it as movement, not as a
    // control code, and there is no such control code for it
    assert_eq!(
        key_sequence(Key::ArrowLeft, Modifiers::CTRL).unwrap(),
        b"\x1b[D".to_vec()
    );
    // a key with nothing to say says nothing
    assert!(key_sequence(Key::Space, plain).is_none());
}

#[test]
fn the_screen_parses_what_a_shell_prints() {
    let mut parser = vt100::Parser::new(4, 24, 100);
    parser.process(b"hello\r\n\x1b[31mred\x1b[0m plain");
    let screen = parser.screen();
    assert!(screen.contents().contains("hello"));
    assert!(screen.contents().contains("red plain"));
    // the colour lands on the cells it was set for, not on the row
    let coloured = screen.cell(1, 0).unwrap();
    assert_eq!(coloured.contents(), "r");
    assert_ne!(coloured.fgcolor(), vt100::Color::Default);
    assert_eq!(screen.cell(1, 4).unwrap().fgcolor(), vt100::Color::Default);
}

#[test]
fn the_cursor_follows_the_text() {
    let mut parser = vt100::Parser::new(4, 24, 100);
    parser.process(b"abc");
    assert_eq!(parser.screen().cursor_position(), (0, 3));
    parser.process(b"\r\n");
    assert_eq!(parser.screen().cursor_position(), (1, 0));
}

#[test]
fn resizing_keeps_the_screen_the_size_it_was_told() {
    let mut parser = vt100::Parser::new(24, 80, 100);
    parser.screen_mut().set_size(10, 40);
    assert_eq!(parser.screen().size(), (10, 40));
    // the scrollback offset is clamped to what exists, which is what keeps the
    // dock's own offset from running off the end
    parser.screen_mut().set_scrollback(5_000);
    assert_eq!(parser.screen().scrollback(), 0);
}
