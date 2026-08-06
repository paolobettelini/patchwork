use resend_rs::Resend;
use resend_rs::types::CreateEmailBaseOptions;

use crate::config::EmailConfig;

#[derive(Clone)]
pub(crate) struct EmailSender {
    client: Resend,
    from_address: String,
}

impl EmailSender {
    pub(crate) fn new(config: EmailConfig) -> Self {
        Self {
            client: Resend::new(&config.resend_api_key),
            from_address: config.from_address,
        }
    }

    pub(crate) async fn send_verification_code(
        &self,
        recipient: &str,
        nickname: &str,
        code: &str,
        expires_in_minutes: i64,
    ) -> Result<(), String> {
        let html = verification_email_html(nickname, code, expires_in_minutes);
        let text = format!(
            "Hi {nickname},\n\nYour Patchwork verification code is {code}.\n\nIt expires in {expires_in_minutes} minutes. If you did not request this account, ignore this email."
        );
        let email = CreateEmailBaseOptions::new(
            self.from_address.as_str(),
            [recipient],
            "Verify your Patchwork account",
        )
        .with_html(&html)
        .with_text(&text);

        self.client
            .emails
            .send(email)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

fn verification_email_html(nickname: &str, code: &str, expires_in_minutes: i64) -> String {
    let nickname = escape_html(nickname);
    let code = escape_html(code);
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Verify your Patchwork account</title>
</head>
<body style="margin:0;background:#101318;color:#f4f7f8;font-family:Arial,sans-serif;">
  <table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="background:#101318;padding:32px 16px;">
    <tr><td align="center">
      <table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="max-width:560px;background:#191d24;border:1px solid #343a46;border-radius:8px;overflow:hidden;">
        <tr><td style="height:6px;background:linear-gradient(90deg,#02a9a9 0 25%,#fdb22c 25% 50%,#fd614e 50% 75%,#6268c8 75% 100%);"></td></tr>
        <tr><td style="padding:32px;">
          <p style="margin:0 0 8px;color:#02c5c5;font-size:13px;font-weight:700;text-transform:uppercase;">Patchwork account</p>
          <h1 style="margin:0;color:#f4f7f8;font-size:28px;line-height:1.2;">Verify your email</h1>
          <p style="margin:18px 0 0;color:#b7bec9;font-size:16px;line-height:1.6;">Hi <strong style="color:#f4f7f8;">{nickname}</strong>, copy this six-digit code into the Patchwork registration page.</p>
          <div style="margin:26px 0;padding:20px;border:1px dashed #596273;border-radius:8px;background:#11151b;text-align:center;">
            <span style="color:#fdb22c;font-family:monospace;font-size:36px;font-weight:700;letter-spacing:8px;">{code}</span>
          </div>
          <p style="margin:0;color:#b7bec9;font-size:14px;line-height:1.6;">The code expires in {expires_in_minutes} minutes and can be used once. If you did not request this account, you can ignore this email.</p>
        </td></tr>
      </table>
    </td></tr>
  </table>
</body>
</html>"#
    )
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_email_escapes_dynamic_values() {
        let html = verification_email_html("<Paolo>", "123456", 10);
        assert!(html.contains("&lt;Paolo&gt;"));
        assert!(!html.contains("<Paolo>"));
        assert!(html.contains("123456"));
    }
}
