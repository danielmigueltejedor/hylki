//! The On This Computer account (#552) in Settings → Accounts: added with
//! one button, and edited for what it has, its name, look and folders.

use super::*;

/// The account's mark, where another account shows its provider's.
pub(super) const LOCAL_ICON: &str = "computer-symbolic";

/// Where its folders are, home written as `~`, for the account list.
pub(super) fn shown_dir() -> String {
    let Some(dir) = crate::config::local_mail_dir() else {
        return String::new();
    };
    match dirs::home_dir().and_then(|h| dir.strip_prefix(h).ok().map(|p| p.to_path_buf())) {
        Some(rest) => format!("~/{}", rest.display()),
        None => dir.display().to_string(),
    }
}

impl AccountsWindow {
    /// Add the On This Computer account; there is one at most.
    pub(super) fn on_add_local(
        &mut self,
        widgets: &AccountsWindowWidgets,
        sender: ComponentSender<Self>,
    ) {
        if self.accounts.iter().any(AccountConfig::is_local) {
            return;
        }
        let account = crate::config::local_account();
        self.accounts.push(account.clone());
        self.rebuild_account_list(&widgets.accounts.accounts_list, &sender);
        let _ = sender.output(AccountsOutput::Saved {
            original_email: None,
            account: Box::new(account),
        });
    }
}

/// What the editor shows of an On This Computer account: no server, no
/// sign-in, nothing to send or sync; its name, look and folders stay.
/// For any other account the groups this hides come back, and the rows
/// the provider and protocol decide on are left to them.
pub(super) fn show_local_state(widgets: &AccountsWindowWidgets, local: bool) {
    let f = &widgets.form;
    for group in [
        &f.aliases_group,
        &f.signatures_group,
        &f.syncing_group,
        &f.openpgp_group,
    ] {
        group.set_visible(!local);
    }
    // Sent, Drafts and Templates are for mail written here, and none is;
    // a Gravatar is looked up by an address, and there is none.
    for row in [
        f.gravatar_row.upcast_ref::<gtk::Widget>(),
        f.folder_sent_row.upcast_ref(),
        f.folder_sent_copy_row.upcast_ref(),
        f.server_saves_row.upcast_ref(),
        f.folder_drafts_row.upcast_ref(),
        f.folder_templates_row.upcast_ref(),
    ] {
        row.set_visible(!local);
    }
    if !local {
        return;
    }
    for w in [
        f.provider_row.upcast_ref::<gtk::Widget>(),
        f.name_row.upcast_ref(),
        f.email_row.upcast_ref(),
        f.protocol_row.upcast_ref(),
        f.host_row.upcast_ref(),
        f.port_row.upcast_ref(),
        f.smtp_row.upcast_ref(),
        f.smtp_port_row.upcast_ref(),
        f.user_row.upcast_ref(),
        f.pass_row.upcast_ref(),
        f.jmap_token_row.upcast_ref(),
        f.oauth_client_id_row.upcast_ref(),
        f.oauth_secret_row.upcast_ref(),
        f.oauth_auth_url_row.upcast_ref(),
        f.oauth_token_url_row.upcast_ref(),
        f.oauth_scope_row.upcast_ref(),
        f.ms_advanced.upcast_ref(),
        f.oauth_signin_btn.upcast_ref(),
        f.oauth_status.upcast_ref(),
        f.goa_hint.upcast_ref(),
        f.smtp_separate_row.upcast_ref(),
        f.smtp_user_row.upcast_ref(),
        f.smtp_pass_row.upcast_ref(),
        f.tls_mismatch_row.upcast_ref(),
        f.test_btn.upcast_ref(),
        f.test_result.upcast_ref(),
        f.reply_copies_row.upcast_ref(),
        f.goa_banner.upcast_ref(),
        f.quota_group.upcast_ref(),
        f.graph_group.upcast_ref(),
        f.keyring_note.upcast_ref(),
    ] {
        w.set_visible(false);
    }
    while let Some(child) = f.provider_mark.first_child() {
        f.provider_mark.remove(&child);
    }
    let icon = gtk::Image::from_icon_name(LOCAL_ICON);
    icon.set_pixel_size(56);
    f.provider_mark.append(&icon);
}
