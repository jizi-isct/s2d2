use json::object;
use regex::Regex;
use web_sys::{Blob, FormData};
use worker::js_sys::{Array, Uint8Array};
use worker::*;

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();

    console_log!("Request: {:?}", req);

    let spam_score_threshold = env
        .var("spam_score_threshold")?
        .to_string()
        .parse::<f64>()
        .unwrap();

    // Check if the request is a multipart form
    let content_type = req.headers().get("Content-Type")?.unwrap_or_default();

    if content_type.starts_with("multipart/form-data") {
        let form_data = req.form_data().await?;
        let FormEntry::Field(from) = form_data.get("from").unwrap() else {
            return Err(Error::from("Missing 'from' field"));
        };
        let FormEntry::Field(to_raw) = form_data.get("to").unwrap() else {
            return Err(Error::from("Missing 'to' field"));
        };
        let to = extract_addresses(&*to_raw);
        let FormEntry::Field(subject) = form_data.get("subject").unwrap() else {
            return Err(Error::from("Missing 'subject' field"));
        };
        let mut text = match form_data.get("text") {
            Some(FormEntry::Field(text)) => text.to_string(),
            _ => "本文を取得できませんでした".to_string(),
        };
        if text.chars().count() > 100 {
            text = text.chars().take(100).collect::<String>();
            text.push_str("...");
        }
        // Process the multipart form data
        let mut attachments = vec![];
        match form_data.get("attachment-info") {
            Some(FormEntry::Field(attachment_info)) => {
                for (name, _) in json::parse(&*attachment_info).unwrap().entries() {
                    let FormEntry::File(file) = form_data.get(name).unwrap() else {
                        return Err(Error::from(format!("Missing field: {}", name)));
                    };
                    attachments.push(file)
                }
            }
            _ => {}
        };

        // Discord webhook
        let mut fields = vec![
            object! {
                name: "送信者",
                value: from,
                inline: true
            },
            object! {
                name: "宛先",
                value: to_raw,
                inline: true
            },
            object! {
                name: "件名",
                value: subject,
                inline: true
            },
            object! {
                name: "本文",
                value: text,
                inline: false
            },
        ];

        let (color, description) = match form_data.get("spam_score") {
            Some(FormEntry::Field(spam_score)) => {
                let spam_score_f = spam_score.parse::<f64>().unwrap_or(0.0);
                fields.push(object! {
                    name: "スパムスコア",
                    value: spam_score_f.to_string(),
                    inline: true
                });

                if spam_score_f > spam_score_threshold {
                    (
                        0xFF0000,
                        "スパムメールの可能性が高いです。注意してください。",
                    )
                } else {
                    (0x0000FF, "スパムメールの可能性は低いです。")
                }
            }
            _ => (0x000000, ""),
        };

        let mut size = 0;
        // Add attachment information to the fields if there are attachments
        if !attachments.is_empty() {
            let mut attachment_info = String::new();
            for file in &attachments {
                size += file.size();
                if size > 1024 * 1024 * 10 {
                    attachment_info.push_str(&format!(
                        "- {} ({}) サイズが大きすぎるのでメーラーからアクセスしてください\n",
                        file.name(),
                        file.type_()
                    ));
                    continue;
                }
                attachment_info.push_str(&format!("- {} ({})\n", file.name(), file.type_()));
            }
            fields.push(object! {
                name: "添付ファイル",
                value: attachment_info,
                inline: false
            });
        }

        let embed = object! {
            title: "メールを受信しました",
            fields: fields,
            color: color,
            description: description
        };

        // Webhook用Payload
        let payload = object! {
            username: "メール転送",
            avatar_url: "https://github.com/jizi-isct.png",
            embeds: vec![embed]
        };

        // Create FormData
        let form_data = FormData::new()?;
        let mut size = 0;
        form_data.append_with_str("payload_json", &payload.dump())?;
        for (i, attachment) in attachments.into_iter().enumerate() {
            size += attachment.size();
            if size > 1024 * 1024 * 10 {
                continue;
            }
            let name = attachment.name().clone();
            let array = Uint8Array::from(&attachment.bytes().await?[..]);
            let blob_parts = Array::new();
            blob_parts.push(&array.buffer());
            let blob = Blob::new_with_u8_array_sequence(&blob_parts)?;
            form_data.append_with_blob_and_filename(&*format!("files[{}]", i), &blob, &*name)?;
        }

        // create request
        console_log!("Sending webhook...");
        console_log!("{:?}", form_data.get("payload_json"));
        let mut init = RequestInit::new();
        init.with_method(Method::Post);
        init.with_body(Some(form_data.into()));

        let webhook_urls = env.kv("WEBHOOK_URLS")?;
        for to in to {
            console_debug!("Sending webhook to {}", to);
            let webhook_url = match webhook_urls.get(to.as_str()).text().await? {
                Some(url) => url,
                None => match webhook_urls.get("default").text().await? {
                    Some(url) => url,
                    None => {
                        return Err(Error::from("No webhook URL found"));
                    }
                },
            };

            let mut res = send_webhook(&webhook_url, &init).await?;

            if 200 <= res.status_code() && res.status_code() < 300 {
                console_log!("Webhook sent!");
            } else {
                console_error!("Failed: {:?}", res);
                console_error!("{:?}", res.text().await?);
            }
        }

        Response::ok("OK")
    } else {
        console_error!("Invalid Content-Type");
        Response::error("Invalid Content-Type", 400)
    }
}

async fn send_webhook(webhook_url: &str, init: &RequestInit) -> Result<Response> {
    Fetch::Request(Request::new_with_init(webhook_url, init)?)
        .send()
        .await
}

fn extract_addresses(to_header: &str) -> Vec<String> {
    // メールアドレスにマッチする正規表現
    let re = Regex::new(r"[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}").unwrap();
    re.find_iter(to_header)
        .map(|mat| mat.as_str().to_string())
        .collect()
}
