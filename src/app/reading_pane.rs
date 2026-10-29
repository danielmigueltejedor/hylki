//! The reading pane switched off (#475): the list takes the whole window,
//! a selected message is only selected, and mail is read in its own window.

use adw::prelude::*;
use relm4::prelude::*;

use crate::config;
use crate::models::Message;
use crate::ui::message_list::MessageListInput;

use super::conversation::body_key;
use super::model::AppModel;
use super::msg::AppMsg;

impl AppModel {
    /// The messages the router sends here (see `AppModel::route`).
    pub(super) fn on_reading_pane(&mut self, msg: AppMsg) {
        match msg {
            AppMsg::SetReadingPane(on) => self.set_reading_pane(on),
            AppMsg::PopoutSeen { account_id, id } => self.mark_window_card_read(account_id, id),
            _ => unreachable!("not routed to on_reading_pane"),
        }
    }

    fn set_reading_pane(&mut self, on: bool) {
        if self
            .reading_pane_action
            .state()
            .and_then(|v| v.get::<bool>())
            != Some(on)
        {
            self.reading_pane_action.set_state(&on.to_variant());
        }
        if !pref!(self.reading_pane = on) {
            return;
        }
        if on {
            // The reader shows what is selected now. It was never drawn
            // while the pane was off, so it must not look already shown.
            let open = self.current.as_ref().map(|c| (c.account_id, c.id));
            self.blank_reader();
            if let Some(key) = open {
                self.message_list.emit(MessageListInput::SelectAndLoad(key));
            }
        } else {
            // An inline reply is saved or kept in its own window, and the
            // hidden reader lets go of what it was showing. The selection
            // stays, for the keys and menus that act on it.
            self.release_reader_compose();
            self.attachments.clear();
            self.attachments_loading = false;
            self.sync_attachment_drawer();
            self.show_message(None, false);
            self.message_list.emit(MessageListInput::ReclaimFocus);
        }
        self.push_single_line();
    }

    /// A row selected with the pane off: it becomes the message the keys,
    /// menus and actions mean, without being shown, fetched or marked read.
    pub(super) fn select_without_reader(
        &mut self,
        m: Message,
        thread: Vec<Message>,
        sender: &ComponentSender<Self>,
    ) {
        self.release_reader_compose();
        self.attachments.clear();
        self.attachments_loading = false;
        self.sync_attachment_drawer();
        self.listed_thread = if thread.is_empty() {
            vec![(m.account_id, m.id)]
        } else {
            thread.iter().map(|t| (t.account_id, t.id)).collect()
        };
        // A conversation's row means all of it, to Delete and Archive.
        self.current_thread = if thread.len() > 1 { thread } else { Vec::new() };
        self.thread_key = None;
        self.thread_related_pending = false;
        let mut current = m;
        if current.body.is_empty() {
            if let Some(body) = self.body_cache.get(&body_key(&current)) {
                current.body = body.clone();
            }
        }
        self.current = Some(current);
        // Gmail's o or Enter asked for it to be read: in a window.
        if std::mem::take(&mut self.reader_focus_pending) {
            self.open_selected_window(sender);
        }
    }

    /// The selected message, or its conversation, in its own window.
    pub(super) fn open_selected_window(&mut self, sender: &ComponentSender<Self>) {
        let Some(message) = self.current.clone() else {
            return;
        };
        let thread = if self.current_thread.len() > 1 {
            self.current_thread.clone()
        } else {
            Vec::new()
        };
        sender.input(AppMsg::OpenMessageWindow { message, thread });
    }

    /// Opening a message in a window is reading it, while there is no pane
    /// to read it in: a lone message is marked read as the setting says.
    /// A conversation is marked card by card as the window shows them.
    pub(super) fn mark_window_read(&mut self, m: &Message, conversation: bool) {
        if self.reading_pane || conversation || !m.unread {
            return;
        }
        if self.read_mark != config::ReadMark::Manual {
            self.mark_opened_read(m);
        }
    }

    /// A conversation card read through in a message window.
    fn mark_window_card_read(&mut self, account_id: u32, id: u32) {
        if self.reading_pane {
            return;
        }
        let folder = self.popouts.values().find_map(|p| {
            p.members
                .iter()
                .find(|(_, key)| *key == (account_id, id))
                .map(|((_, folder_id, _), _)| *folder_id)
        });
        let Some(folder_id) = folder else {
            return;
        };
        if let Some(m) = self
            .shown_copy_in(account_id, folder_id, id)
            .filter(|m| m.unread)
        {
            self.mark_opened_read(&m);
        }
    }
}
