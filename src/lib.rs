use json::object;
use web_sys::{Blob, FormData};
use worker::*;
use worker::js_sys::{Array, Uint8Array};

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    console_log!("Request: {:?}", req);

    console_error_panic_hook::set_once();

    // Check if the request is a multipart form
    let content_type = req.headers().get("Content-Type")?.unwrap_or_default();

    if content_type.starts_with("multipart/form-data") {
        let form_data = req.form_data().await?;
        let FormEntry::Field(from) = form_data.get("from").unwrap() else {
            return Err(Error::from("Missing 'from' field"));
        };
        let FormEntry::Field(to) = form_data.get("to").unwrap() else {
            return Err(Error::from("Missing 'to' field"));
        };
        let FormEntry::Field(subject) = form_data.get("subject").unwrap() else {
            return Err(Error::from("Missing 'subject' field"));
        };
        let FormEntry::Field(text) = form_data.get("text").unwrap() else {
            return Err(Error::from("Missing 'text' field"));
        };
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
            },
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
                value: to,
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

                if spam_score_f
                    > env
                        .var("spam_score_threshold")?
                        .to_string()
                        .parse::<f64>()
                        .unwrap_or(5.0)
                {
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
                    attachment_info.push_str(&format!("- {} ({}) サイズが大きすぎるのでメーラーからアクセスしてください\n", file.name(), file.type_()));
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

        let mut res = Fetch::Request(Request::new_with_init(&*env.secret("WEBHOOK_URL")?.to_string(), &init)?)
            .send()
            .await?;

        if 200 <= res.status_code() && res.status_code() < 300 {
            console_log!("Webhook sent!");
            Response::ok("Webhook sent!")
        } else {
            console_error!("Failed: {}", res.text().await?);
            Response::error(format!("Failed: {}", res.text().await?), res.status_code())
        }
    } else {
        console_error!("Invalid Content-Type");
        Response::error("Invalid Content-Type", 400)
    }
}
