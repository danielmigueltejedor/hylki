//! Composing → Grammar (#452): checking what is written with a LanguageTool
//! server. Saved to `grammar.toml` as it is changed, as the Translation
//! page saves its own; the Premium key goes to the keyring when it is
//! applied.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::i18n::{i18n, i18n_f};
use crate::services::grammar::{self, Settings};

pub(in crate::ui::preferences) fn group() -> adw::PreferencesGroup {
    let settings = Rc::new(RefCell::new(grammar::load()));
    let group = adw::PreferencesGroup::new();
    group.set_title(&i18n("Grammar"));
    group.set_description(Some(&i18n(
        "What you write goes to the server as you type, without the quote or the signature.",
    )));

    let check_row = adw::SwitchRow::new();
    check_row.set_title(&i18n("Check grammar and style"));
    check_row.set_subtitle(&i18n("Right-click a marked phrase for suggestions."));
    check_row.set_active(settings.borrow().enabled);
    group.add(&check_row);

    let url_row = adw::EntryRow::new();
    url_row.set_title(&i18n("LanguageTool server"));
    url_row.set_text(&settings.borrow().url);
    group.add(&url_row);

    let user_row = adw::EntryRow::new();
    user_row.set_title(&i18n("Username (LanguageTool Premium)"));
    user_row.set_text(&settings.borrow().username);
    group.add(&user_row);

    let key_row = adw::PasswordEntryRow::new();
    key_row.set_title(&i18n("API key (LanguageTool Premium)"));
    key_row.set_show_apply_button(true);
    group.add(&key_row);

    let test_row = adw::ActionRow::new();
    test_row.set_title(&i18n("Check the settings"));
    test_row.set_subtitle(&i18n("Sends a test sentence to the server."));
    test_row.set_subtitle_lines(3);
    let test = gtk::Button::with_label(&i18n("Check"));
    test.set_valign(gtk::Align::Center);
    test_row.add_suffix(&test);
    group.add(&test_row);

    let rows: [gtk::Widget; 4] = [
        url_row.clone().upcast(),
        user_row.clone().upcast(),
        key_row.clone().upcast(),
        test_row.clone().upcast(),
    ];
    let shape = move |on: bool| {
        for r in &rows {
            r.set_sensitive(on);
        }
    };
    shape(settings.borrow().enabled);

    // The key is only read for an account that has one, the first time the
    // page is shown: the Settings window is built in advance and must not
    // wait on the keyring, which may wait on its user (#466).
    {
        let loaded = std::cell::Cell::new(false);
        let (settings, key_row) = (settings.clone(), key_row.clone());
        group.connect_map(move |_| {
            if loaded.replace(true) || settings.borrow().username.trim().is_empty() {
                return;
            }
            let key_row = key_row.clone();
            crate::config::keyring_then(grammar::load_key, move |key| {
                if key_row.text().is_empty() {
                    key_row.set_text(&key);
                }
            });
        });
    }

    let save = {
        let settings = settings.clone();
        move |f: &dyn Fn(&mut Settings)| {
            f(&mut settings.borrow_mut());
            grammar::save(&settings.borrow());
        }
    };
    {
        let save = save.clone();
        check_row.connect_active_notify(move |row| {
            let on = row.is_active();
            save(&|s| s.enabled = on);
            shape(on);
        });
    }
    {
        let save = save.clone();
        url_row.connect_changed(move |row| {
            let url = row.text().trim().to_string();
            save(&|s| s.url = url.clone());
        });
    }
    {
        let save = save.clone();
        user_row.connect_changed(move |row| {
            let user = row.text().trim().to_string();
            save(&|s| s.username = user.clone());
        });
    }
    key_row.connect_apply(move |row| {
        let key = row.text().to_string();
        crate::config::keyring_later(move || grammar::store_key(&key));
    });
    test.connect_clicked(move |b| {
        let s = settings.borrow().clone();
        let key = key_row.text().to_string();
        // Checking is applying: the key that works is the one kept.
        {
            let key = key.clone();
            crate::config::keyring_later(move || grammar::store_key(&key));
        }
        b.set_sensitive(false);
        test_row.set_subtitle(&i18n("Checking…"));
        let (b, test_row) = (b.clone(), test_row.clone());
        glib::spawn_future_local(async move {
            let result = gtk::gio::spawn_blocking(move || grammar::check_server(&s, key.trim()))
                .await
                .unwrap_or_else(|_| Err(i18n("The check stopped unexpectedly.")));
            test_row.set_subtitle(&match result {
                Ok(server) => i18n_f("It works: {server} answered.", &[("server", &server)]),
                Err(why) => why,
            });
            b.set_sensitive(true);
        });
    });
    group
}
