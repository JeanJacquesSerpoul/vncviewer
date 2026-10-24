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
        if let Some(board) = st.board()
            && let Err(e) = board.set_text(text)
        {
            log::warn!("cannot write the clipboard: {e}");
        }
    }

    /// Returns the local clipboard text, if any. Each session filters out what
    /// it already exchanged. May block briefly: call it off the main thread.
    pub fn local_text(&self) -> Option<String> {
        let mut st = self.inner.lock().unwrap();
        let text = st.board()?.get_text().ok()?;
        (!text.is_empty()).then_some(text)
    }
}
