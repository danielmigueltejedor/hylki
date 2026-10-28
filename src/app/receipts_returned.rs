//! Read receipts that come back (#520): the sent message shows who read it,
//! and the receipt mail is filed away, since what it says is on the message
//! now.

use relm4::prelude::*;

use crate::config::{self, ReturnedReceipts};
use crate::models::{FolderKind, ReceiptMarks, SenderCheck};
use crate::ui::message_list::MessageListInput;
use crate::ui::message_view::MessageViewInput;
use crate::ui::message_window::MessageWindowInput;
use crate::worker::MailRequest;

use super::conversation::{body_key, BodyKey};
use super::model::AppModel;

impl AppModel {
    /// Read what a sender check says about receipts: that mail the user
    /// sent asked for one, or that this message is one that came back.
    /// `key` is where the message checked is filed.
    pub(super) fn note_receipts(&mut self, key: Option<BodyKey>, check: &SenderCheck) {
        let mut changed = false;
        // A request for a receipt to one of the user's own addresses is on
        // mail they sent: its Sent copy, read like any other.
        if let Some(r) = check.receipt.as_ref().filter(|r| !r.message_id.is_empty()) {
            let known = self
                .read_by
                .asked
                .iter()
                .any(|a| a.eq_ignore_ascii_case(&r.message_id));
            if !known && self.is_own_address(&r.to) {
                self.read_by.asked.push(r.message_id.clone());
                changed = true;
            }
        }
        if let Some(r) = &check.read {
            let known = self.read_by.read.iter().any(|b| {
                b.key.eq_ignore_ascii_case(&r.message_id) && b.by.eq_ignore_ascii_case(&r.by)
            });
            if !known {
                tracing::info!("read receipt for <{}> came back from {}", r.message_id, r.by);
                self.read_by.read.push(config::ReadBy {
                    key: r.message_id.clone(),
                    by: r.by.clone(),
                    at: if r.at > 0 { r.at } else { crate::datefmt::now() },
                });
                changed = true;
            }
            if let Some(k) = key {
                self.file_receipt(k);
            }
        }
        if changed {
            config::save_read_by(&self.read_by);
            self.push_receipt_marks();
        }
    }

    /// Whether any of `addrs` (comma-separated) is one of the user's.
    fn is_own_address(&self, addrs: &str) -> bool {
        let own = self.identities_map();
        addrs.split(',').map(str::trim).any(|addr| {
            own.values()
                .flatten()
                .any(|a| a.eq_ignore_ascii_case(addr))
        })
    }

    /// Mark a receipt that came back read and move it where the setting
    /// says. Only from the Inbox, and never while it is being read: a
    /// message that vanishes from under the reader is worse than one left
    /// where it is.
    fn file_receipt(&mut self, key: BodyKey) {
        let kind = match self.returned_receipts {
            ReturnedReceipts::Keep => return,
            ReturnedReceipts::Trash => FolderKind::Trash,
            ReturnedReceipts::Archive => FolderKind::Archive,
        };
        let (account_id, folder_id, uid) = key;
        if self.receipts_filed.contains(&key)
            || self.folder_kind(account_id, folder_id) != Some(FolderKind::Inbox)
        {
            return;
        }
        let shown = self
            .current
            .iter()
            .chain(self.current_thread.iter())
            .any(|m| body_key(m) == key)
            || self.popouts.values().any(|p| p.message == key);
        let Some(dest) = self.folder_path_for(account_id, kind).filter(|_| !shown) else {
            return;
        };
        self.receipts_filed.insert(key);
        match self.held_message(key) {
            Some(m) => {
                let Some(src) = self.resolve_folder_path(&m) else {
                    return;
                };
                self.set_read(&m, true);
                self.send_to(
                    account_id,
                    MailRequest::MoveMessage {
                        path: src,
                        uid: m.uid,
                        dest,
                    },
                );
                self.discard_message(&m);
            }
            // Read in the background from an Inbox not on screen: the
            // server is told, and the list finds it gone when it next looks.
            None => {
                let Some(src) = self.folder_path(account_id, folder_id) else {
                    return;
                };
                self.send_to(
                    account_id,
                    MailRequest::SetSeen {
                        path: src.clone(),
                        uid,
                        seen: true,
                    },
                );
                self.send_to(account_id, MailRequest::MoveMessage { path: src, uid, dest });
            }
        }
    }

    /// What read_by.toml says, as the list and the readers look it up.
    pub(super) fn receipt_marks(&self) -> ReceiptMarks {
        let mut marks = ReceiptMarks {
            asked: self.read_by.asked.iter().map(|a| a.to_lowercase()).collect(),
            ..Default::default()
        };
        for r in &self.read_by.read {
            let readers = marks.read.entry(r.key.to_lowercase()).or_default();
            if !readers.iter().any(|(by, _)| by.eq_ignore_ascii_case(&r.by)) {
                readers.push((r.by.clone(), r.at));
            }
        }
        super::showcase::showcase_receipt_marks(&mut marks);
        marks
    }

    /// Hand the list, the reader and every pop-out the checkmarks.
    pub(super) fn push_receipt_marks(&self) {
        let marks = self.receipt_marks();
        self.message_list
            .emit(MessageListInput::SetReceiptMarks(marks.clone()));
        self.message_view
            .emit(MessageViewInput::SetReceiptMarks(marks.clone()));
        for p in self.popouts.values() {
            p.controller
                .emit(MessageWindowInput::SetReceiptMarks(marks.clone()));
        }
    }
}
