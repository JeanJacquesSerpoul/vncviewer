//! Mapping of DOM keyboard events to X11 keysyms (what RFB KeyEvent carries).

/// Returns the keysym for a DOM `KeyboardEvent` given its `key` and `code`.
pub fn keysym(key: &str, code: &str) -> Option<u32> {
    // Physical position matters for modifiers and the numeric keypad.
    let by_code = match code {
        "ShiftLeft" => Some(0xffe1),
        "ShiftRight" => Some(0xffe2),
        "ControlLeft" => Some(0xffe3),
        "ControlRight" => Some(0xffe4),
        "AltLeft" => Some(0xffe9),
        "AltRight" if key == "AltGraph" => Some(0xfe03),
        "AltRight" => Some(0xffea),
        "MetaLeft" | "OSLeft" => Some(0xffeb),
        "MetaRight" | "OSRight" => Some(0xffec),
        "NumpadEnter" => Some(0xff8d),
        _ => None,
    };
    if by_code.is_some() {
        return by_code;
    }
    if let Some(n) = code.strip_prefix("Numpad") {
        let mut chars = key.chars();
        // With NumLock on, digits and operators come from the keypad.
        if let (Some(_), None) = (chars.next(), chars.next()) {
            let ks = match n {
                "Decimal" => Some(0xffae),
                "Add" => Some(0xffab),
                "Subtract" => Some(0xffad),
                "Multiply" => Some(0xffaa),
                "Divide" => Some(0xffaf),
                "Equal" => Some(0xffbd),
                d => d
                    .parse::<u32>()
                    .ok()
                    .filter(|d| *d <= 9)
                    .map(|d| 0xffb0 + d),
            };
            if ks.is_some() {
                return ks;
            }
        }
    }

    let named = match key {
        "Backspace" => 0xff08,
        "Tab" => 0xff09,
        "Enter" => 0xff0d,
        "Pause" => 0xff13,
        "ScrollLock" => 0xff14,
        "Escape" => 0xff1b,
        "Home" => 0xff50,
        "ArrowLeft" => 0xff51,
        "ArrowUp" => 0xff52,
        "ArrowRight" => 0xff53,
        "ArrowDown" => 0xff54,
        "PageUp" => 0xff55,
        "PageDown" => 0xff56,
        "End" => 0xff57,
        "PrintScreen" => 0xff61,
        "Insert" => 0xff63,
        "ContextMenu" => 0xff67,
        "NumLock" => 0xff7f,
        "CapsLock" => 0xffe5,
        "Delete" => 0xffff,
        "Shift" => 0xffe1,
        "Control" => 0xffe3,
        "Alt" => 0xffe9,
        "AltGraph" => 0xfe03,
        "Meta" | "OS" | "Super" => 0xffeb,
        "Clear" => 0xff0b,
        _ => 0,
    };
    if named != 0 {
        return Some(named);
    }
    if let Some(n) = key.strip_prefix('F').and_then(|n| n.parse::<u32>().ok())
        && (1..=35).contains(&n)
    {
        return Some(0xffbe + n - 1);
    }

    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(char_keysym(c)),
        // "Dead", "Unidentified", "Process", multi-character IME output...
        _ => None,
    }
}

/// Keysym of a printable character.
pub fn char_keysym(c: char) -> u32 {
    let cp = c as u32;
    match cp {
        0x20..=0x7e | 0xa0..=0xff => cp,
        _ => 0x0100_0000 | cp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn printable() {
        assert_eq!(keysym("a", "KeyA"), Some(0x61));
        assert_eq!(keysym("A", "KeyA"), Some(0x41));
        assert_eq!(keysym(" ", "Space"), Some(0x20));
        assert_eq!(keysym("é", "Digit2"), Some(0xe9));
        assert_eq!(keysym("€", "KeyE"), Some(0x0100_20ac));
    }

    #[test]
    fn named_and_modifiers() {
        assert_eq!(keysym("Enter", "Enter"), Some(0xff0d));
        assert_eq!(keysym("Shift", "ShiftRight"), Some(0xffe2));
        assert_eq!(keysym("AltGraph", "AltRight"), Some(0xfe03));
        assert_eq!(keysym("F12", "F12"), Some(0xffc9));
        assert_eq!(keysym("Dead", "BracketLeft"), None);
    }

    #[test]
    fn numpad() {
        assert_eq!(keysym("7", "Numpad7"), Some(0xffb7));
        assert_eq!(keysym("+", "NumpadAdd"), Some(0xffab));
        // NumLock off: the key reports its navigation meaning.
        assert_eq!(keysym("Home", "Numpad7"), Some(0xff50));
        assert_eq!(keysym("Enter", "NumpadEnter"), Some(0xff8d));
    }
}
