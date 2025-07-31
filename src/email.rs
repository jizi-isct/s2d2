use crate::extract_addresses;
use anyhow::anyhow;
use worker::{File, FormData, FormEntry};

pub struct Email {
    pub from: String,
    pub to_raw: String,
    pub to: Vec<String>,
    pub subject: String,
    pub text: String,
    pub attachments: Vec<File>,
}
impl Email {
    pub fn from_form_data(form_data: &FormData) -> anyhow::Result<Option<Self>> {
        let FormEntry::Field(from) = form_data.get("from").unwrap() else {
            return Err(anyhow!("Missing 'from' field"));
        };
        let FormEntry::Field(to_raw) = form_data.get("to").unwrap() else {
            return Err(anyhow!("Missing 'to' field"));
        };
        let to = extract_addresses(&*to_raw);
        let FormEntry::Field(subject) = form_data.get("subject").unwrap() else {
            return Err(anyhow!("Missing 'subject' field"));
        };
        if subject.starts_with("[SPAM]") {
            return Ok(None);
        }
        let mut text = match form_data.get("text") {
            Some(FormEntry::Field(text)) => text,
            _ => "本文を取得できませんでした".to_string(),
        };

        if text.chars().count() > 1000 {
            text = text.chars().take(1000).collect::<String>();
            text.push_str("...");
        }

        // Process the multipart form data
        let mut attachments = vec![];
        match form_data.get("attachment-info") {
            Some(FormEntry::Field(attachment_info)) => {
                for (name, _) in json::parse(&*attachment_info).unwrap().entries() {
                    let FormEntry::File(file) = form_data.get(name).unwrap() else {
                        return Err(anyhow!("Missing field: {}", name));
                    };
                    attachments.push(file)
                }
            }
            _ => {}
        };

        Ok(Some(Self {
            from,
            to_raw,
            to,
            subject,
            text,
            attachments,
        }))
    }
}
