//! Synchronisation between the local clipboard and the remote one.

use std::sync::Mutex;

#[derive(Default)]
pub struct ClipboardSync {
    inner: Mutex<State>,
}

#[derive(Default)]
struct State {
    // Kept alive for the whole run: on Linux the clipboard content is served
    // by this instance, and dropping it would lose what the server sent.
    board: Option<arboard::Clipboard>,
    /// Last text exchanged in either direction, to avoid echoing it back.
    last: Option<String>,
}

impl State {
    fn board(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.board.is_none() {
            match arboard::Clipboard::new() {
                Ok(b) => self.board = Some(b),
                Err(e) => log::warn!("clipboard unavailable: {e}"),
            }
        }
        self.board.as_mut()
    }
}

impl ClipboardSync {
    /// Text copied on the remote side: put it in the local clipboard.
    pub fn set_from_server(&self, text: String) {
        let mut st = self.inner.lock().unwrap();
        if st.last.as_deref() == Some(text.as_str()) {
            return;
        }
        if let Some(board) = st.board()
            && let Err(e) = board.set_text(text.clone())
        {
            log::warn!("cannot write the clipboard: {e}");
        }
        st.last = Some(text);
    }

    /// Returns the local clipboard text if it changed since the last exchange.
    /// May block briefly: call it off the main thread.
    pub fn take_local_if_changed(&self) -> Option<String> {
        let mut st = self.inner.lock().unwrap();
        let text = st.board()?.get_text().ok()?;
        if text.is_empty() || st.last.as_deref() == Some(text.as_str()) {
            return None;
        }
        st.last = Some(text.clone());
        Some(text)
    }
}
