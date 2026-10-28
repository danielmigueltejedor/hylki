//! Read receipts (#520): Message Disposition Notifications, RFC 8098.
//!
//! A message asks for a receipt with a `Disposition-Notification-To`
//! header naming where it should go. The receipt is a `multipart/report`:
//! a sentence for the person, then a `message/disposition-notification`
//! part for their mail app, which says which message was shown, to whom,
//! and whether the receipt was sent by hand or by itself.

use crate::mail::mail_header::unfold;

/// A message's request for a read receipt, read with the sender check and
/// kept in the cache beside it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ReceiptRequest {
    /// Where the receipt goes: every address the header names, bare,
    /// comma-separated.
    pub(crate) to: String,
    /// Whether the header names a single address, and it is the message's
    /// Return-Path. RFC 8098 (2.1) forbids sending a receipt by itself
    /// otherwise: a request that sends the answer somewhere other than
    /// where the mail came from is asked about, whatever the setting says.
    #[serde(default)]
    pub(crate) matches_return_path: bool,
    /// The `Original-Recipient` header, which the report repeats when the
    /// message carried one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) original_recipient: String,
}

/// The value of the first header called `name`, unfolded, as written.
fn raw_header(parsed: &mail_parser::Message, name: &str) -> Option<String> {
    let raw = parsed.raw_message();
    parsed
        .headers()
        .iter()
        .find(|h| h.name().eq_ignore_ascii_case(name))
        .and_then(|h| raw.get(h.offset_start()..h.offset_end()))
        .map(|b| unfold(&String::from_utf8_lossy(b)))
        .filter(|v| !v.is_empty())
}

/// The bare addresses of an address-list header value: what is between
/// angle brackets, or the word with an `@` in it.
fn addresses(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in value.split(',') {
        let addr = match (item.find('<'), item.rfind('>')) {
            (Some(a), Some(b)) if a < b => item[a + 1..b].trim().to_string(),
            _ => item
                .split_whitespace()
                .find(|w| w.contains('@'))
                .unwrap_or_default()
                .trim_matches(|c| c == '"' || c == '(' || c == ')')
                .to_string(),
        };
        if addr.contains('@') && !out.iter().any(|o| o.eq_ignore_ascii_case(&addr)) {
            out.push(addr);
        }
    }
    out
}

/// The read receipt a message asks for, if it asks for one.
pub(crate) fn detect(parsed: &mail_parser::Message) -> Option<ReceiptRequest> {
    let to = addresses(&raw_header(parsed, "Disposition-Notification-To")?);
    if to.is_empty() {
        return None;
    }
    let return_path = raw_header(parsed, "Return-Path")
        .map(|v| addresses(&v))
        .unwrap_or_default();
    let matches_return_path = to.len() == 1
        && return_path
            .first()
            .is_some_and(|rp| rp.eq_ignore_ascii_case(&to[0]));
    Some(ReceiptRequest {
        to: to.join(", "),
        matches_return_path,
        original_recipient: raw_header(parsed, "Original-Recipient").unwrap_or_default(),
    })
}

/// The address type a report field names an address with: `rfc822`, or
/// `utf-8` for an internationalized one (RFC 6533).
fn address_field(addr: &str) -> String {
    let kind = if addr.is_ascii() { "rfc822" } else { "utf-8" };
    format!("{kind};{addr}")
}

/// The fields of the `message/disposition-notification` part (RFC 8098,
/// 3.2): the message `message_id` (bare) was shown to `recipient`. The
/// sending mode says whether the user pressed Send Receipt or the setting
/// sent it; either way the message was opened by hand.
pub(crate) fn report_fields(
    recipient: &str,
    request: &ReceiptRequest,
    message_id: &str,
    automatic: bool,
) -> String {
    let mut out = String::from("Reporting-UA: Hylki\r\n");
    if !request.original_recipient.is_empty() {
        out.push_str(&format!(
            "Original-Recipient: {}\r\n",
            request.original_recipient
        ));
    }
    out.push_str(&format!(
        "Final-Recipient: {}\r\n",
        address_field(recipient)
    ));
    if !message_id.is_empty() {
        out.push_str(&format!("Original-Message-ID: <{message_id}>\r\n"));
    }
    let mode = if automatic {
        "MDN-sent-automatically"
    } else {
        "MDN-sent-manually"
    };
    out.push_str(&format!("Disposition: manual-action/{mode}; displayed\r\n"));
    out
}

/// Whether a part is the machine half of a read receipt: kept out of the
/// attachments, where it would show as a file nobody can open.
pub(crate) fn is_report_part(part: &mail_parser::MessagePart) -> bool {
    use mail_parser::MimeHeaders;
    part.content_type().is_some_and(|c| {
        c.ctype().eq_ignore_ascii_case("message")
            && c.subtype()
                .is_some_and(|s| s.eq_ignore_ascii_case("disposition-notification"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_parser::MessageParser;

    fn request_of(headers: &str) -> Option<ReceiptRequest> {
        let raw = format!("{headers}From: a@example.org\r\nSubject: Hi\r\n\r\nBody\r\n");
        detect(&MessageParser::default().parse(raw.as_bytes()).unwrap())
    }

    #[test]
    fn a_request_names_where_the_receipt_goes() {
        let r = request_of(
            "Return-Path: <ann@example.org>\r\n\
             Disposition-Notification-To: Ann <ann@example.org>\r\n",
        )
        .unwrap();
        assert_eq!(r.to, "ann@example.org");
        assert!(r.matches_return_path);
        assert!(r.original_recipient.is_empty());
    }

    #[test]
    fn a_receipt_sent_elsewhere_is_never_automatic() {
        let r = request_of(
            "Return-Path: <bounce@lists.example.org>\r\n\
             Disposition-Notification-To: ann@example.org\r\n",
        )
        .unwrap();
        assert!(!r.matches_return_path);
        let r = request_of("Disposition-Notification-To: ann@example.org\r\n").unwrap();
        assert!(!r.matches_return_path, "no Return-Path to match");
        let r = request_of(
            "Return-Path: <ann@example.org>\r\n\
             Disposition-Notification-To: ann@example.org, \"Bob\" <bob@example.org>\r\n",
        )
        .unwrap();
        assert_eq!(r.to, "ann@example.org, bob@example.org");
        assert!(!r.matches_return_path, "two addresses");
    }

    #[test]
    fn no_header_or_no_address_is_no_request() {
        assert!(request_of("").is_none());
        assert!(request_of("Disposition-Notification-To: nobody\r\n").is_none());
    }

    #[test]
    fn the_report_names_the_message_and_how_it_was_sent() {
        let r = ReceiptRequest {
            to: "ann@example.org".into(),
            matches_return_path: true,
            original_recipient: "rfc822;me@example.com".into(),
        };
        assert_eq!(
            report_fields("me@example.com", &r, "abc@example.org", false),
            "Reporting-UA: Hylki\r\n\
             Original-Recipient: rfc822;me@example.com\r\n\
             Final-Recipient: rfc822;me@example.com\r\n\
             Original-Message-ID: <abc@example.org>\r\n\
             Disposition: manual-action/MDN-sent-manually; displayed\r\n"
        );
        let auto = report_fields("jé@exemple.fr", &ReceiptRequest::default(), "", true);
        assert!(auto.contains("Final-Recipient: utf-8;jé@exemple.fr\r\n"));
        assert!(auto.contains("MDN-sent-automatically; displayed"));
        assert!(!auto.contains("Original-"));
    }
}
