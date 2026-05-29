mod email;

use crate::email::Email;
use json::object;
use regex::Regex;
use web_sys::{Blob, FormData};
use worker::js_sys::{Array, Uint8Array};
use worker::*;

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    console_error_panic_hook::set_once();

    let spam_score_threshold = env
        .var("spam_score_threshold")?
        .to_string()
        .parse::<f64>()
        .unwrap();

    // Check if the request is a multipart form
    let content_type = req.headers().get("Content-Type")?.unwrap_or_default();

    if content_type.starts_with("multipart/form-data") {
        let form_data = req.form_data().await?;
        let Some(email) = Email::from_form_data(&form_data).unwrap() else {
            console_log!("The email was rejected.");
            return Response::ok("the email was rejected");
        };

        // Discord webhook
        let mut fields = vec![
            object! {
                name: "送信者",
                value: email.from,
                inline: true
            },
            object! {
                name: "宛先",
                value: email.to_raw,
                inline: true
            },
            object! {
                name: "件名",
                value: email.subject,
                inline: true
            },
            object! {
                name: "本文",
                value: email.text,
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
        if !email.attachments.is_empty() {
            let mut attachment_info = String::new();
            for file in &email.attachments {
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

        let payload_json = payload.dump();
        let attachments = collect_webhook_attachments(email.attachments).await?;

        let webhook_urls = env.kv("WEBHOOK_URLS")?;
        for to in email.to {
            let urls = match webhook_urls.get(to.as_str()).text().await? {
                Some(urls) => parse_webhook_urls(&urls),
                None => match webhook_urls.get("default").text().await? {
                    Some(urls) => parse_webhook_urls(&urls),
                    None => Vec::new(),
                },
            };

            if urls.is_empty() {
                return Err(Error::from("No webhook URL found"));
            }

            for webhook_url in urls {
                let init = create_webhook_request_init(&payload_json, &attachments)?;
                let mut res = send_webhook(&webhook_url, &init).await?;

                if !(200 <= res.status_code() && res.status_code() < 300) {
                    console_error!("Failed: {:?}", res);
                    console_error!("{}", res.text().await?);
                    console_error!("{}", payload_json);
                    return Err(Error::from("Failed to send webhook"));
                }
            }
        }

        Response::ok("OK")
    } else {
        console_error!("Invalid Content-Type");
        Err(Error::from("Invalid Content-Type"))
    }
}

async fn collect_webhook_attachments(
    attachments: Vec<worker::File>,
) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    let mut size = 0;

    for attachment in attachments {
        size += attachment.size();
        if size > 1024 * 1024 * 10 {
            continue;
        }

        files.push((attachment.name().clone(), attachment.bytes().await?));
    }

    Ok(files)
}

fn create_webhook_request_init(
    payload_json: &str,
    attachments: &[(String, Vec<u8>)],
) -> Result<RequestInit> {
    let form_data = FormData::new()?;
    form_data.append_with_str("payload_json", payload_json)?;

    for (i, (name, bytes)) in attachments.iter().enumerate() {
        let array = Uint8Array::from(&bytes[..]);
        let blob_parts = Array::new();
        blob_parts.push(&array.buffer());
        let blob = Blob::new_with_u8_array_sequence(&blob_parts)?;
        form_data.append_with_blob_and_filename(&format!("files[{}]", i), &blob, name)?;
    }

    let mut init = RequestInit::new();
    init.with_method(Method::Post);
    init.with_body(Some(form_data.into()));

    Ok(init)
}

fn parse_webhook_urls(webhook_urls: &str) -> Vec<String> {
    webhook_urls
        .split(',')
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(ToString::to_string)
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_webhook_urls_splits_comma_separated_values() {
        assert_eq!(
            parse_webhook_urls(
                "https://example.com/a, https://example.com/b,,https://example.com/c "
            ),
            vec![
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string(),
                "https://example.com/c".to_string(),
            ]
        );
    }

    #[test]
    fn parse_webhook_urls_ignores_empty_values() {
        assert!(parse_webhook_urls(" , ,, ").is_empty());
    }
}
