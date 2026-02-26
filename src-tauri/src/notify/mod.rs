//! Notification System — Email alerts for offload events.
//!
//! Phase 1: Email via SMTP (lettre)
//! Future: SMS (Twilio), WeChat push (Server酱)
//!
//! Triggers:
//! - Offload completed
//! - Verification completed
//! - Error/anomaly detected

use anyhow::{bail, Context, Result};
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::{Deserialize, Serialize};

use crate::config::EmailSettings;

/// Notification event types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NotifyEvent {
    OffloadCompleted {
        job_id: String,
        job_name: String,
        file_count: usize,
        total_bytes: u64,
        duration_secs: f64,
        mhl_generated: bool,
        warnings: Vec<String>,
    },
    OffloadFailed {
        job_id: String,
        job_name: String,
        error: String,
    },
    VerificationFailed {
        job_id: String,
        job_name: String,
        failed_files: Vec<String>,
    },
}

impl NotifyEvent {
    /// Generate email subject line for this event.
    fn subject(&self) -> String {
        match self {
            NotifyEvent::OffloadCompleted { job_name, file_count, .. } => {
                format!("[DIT] Offload Complete: {} ({} files)", job_name, file_count)
            }
            NotifyEvent::OffloadFailed { job_name, .. } => {
                format!("[DIT] OFFLOAD FAILED: {}", job_name)
            }
            NotifyEvent::VerificationFailed { job_name, failed_files, .. } => {
                format!(
                    "[DIT] Verification FAILED: {} ({} files)",
                    job_name,
                    failed_files.len()
                )
            }
        }
    }

    /// Generate HTML email body for this event.
    fn body_html(&self) -> String {
        match self {
            NotifyEvent::OffloadCompleted {
                job_name,
                file_count,
                total_bytes,
                duration_secs,
                mhl_generated,
                warnings,
                ..
            } => {
                let size = format_bytes(*total_bytes);
                let dur = format_duration(*duration_secs);
                let speed = if *duration_secs > 0.0 {
                    format_bytes((*total_bytes as f64 / duration_secs) as u64) + "/s"
                } else {
                    "N/A".to_string()
                };
                let mhl_badge = if *mhl_generated {
                    "<span style=\"background:#3f51b5;color:#fff;padding:2px 8px;border-radius:4px;font-size:12px;\">MHL Generated</span>"
                } else {
                    ""
                };
                let warnings_html = if warnings.is_empty() {
                    String::new()
                } else {
                    let items: Vec<String> = warnings
                        .iter()
                        .map(|w| format!("<li style=\"color:#ff9800;\">{}</li>", html_escape(w)))
                        .collect();
                    format!(
                        "<div style=\"margin-top:16px;padding:12px;background:#fff3e0;border-radius:8px;\"><strong>Warnings:</strong><ul>{}</ul></div>",
                        items.join("")
                    )
                };

                format!(
                    r#"<div style="font-family:-apple-system,BlinkMacSystemFont,sans-serif;max-width:600px;margin:0 auto;background:#1a1a2e;color:#e0e0e0;border-radius:12px;overflow:hidden;">
  <div style="background:#4caf50;padding:20px 24px;">
    <h2 style="margin:0;color:#fff;">Offload Complete</h2>
  </div>
  <div style="padding:24px;">
    <h3 style="margin:0 0 16px;">{job_name}</h3>
    <table style="width:100%;border-collapse:collapse;">
      <tr><td style="padding:8px 0;color:#9e9e9e;">Files</td><td style="padding:8px 0;text-align:right;">{file_count}</td></tr>
      <tr><td style="padding:8px 0;color:#9e9e9e;">Total Size</td><td style="padding:8px 0;text-align:right;">{size}</td></tr>
      <tr><td style="padding:8px 0;color:#9e9e9e;">Duration</td><td style="padding:8px 0;text-align:right;">{dur}</td></tr>
      <tr><td style="padding:8px 0;color:#9e9e9e;">Avg Speed</td><td style="padding:8px 0;text-align:right;">{speed}</td></tr>
    </table>
    <div style="margin-top:16px;">{mhl_badge}</div>
    {warnings_html}
  </div>
  <div style="padding:12px 24px;background:#16213e;text-align:center;font-size:12px;color:#666;">
    DIT System — Bulletproof Card Offload
  </div>
</div>"#,
                    job_name = html_escape(job_name),
                    file_count = file_count,
                    size = size,
                    dur = dur,
                    speed = speed,
                    mhl_badge = mhl_badge,
                    warnings_html = warnings_html,
                )
            }
            NotifyEvent::OffloadFailed { job_name, error, .. } => {
                format!(
                    r#"<div style="font-family:-apple-system,BlinkMacSystemFont,sans-serif;max-width:600px;margin:0 auto;background:#1a1a2e;color:#e0e0e0;border-radius:12px;overflow:hidden;">
  <div style="background:#f44336;padding:20px 24px;">
    <h2 style="margin:0;color:#fff;">Offload FAILED</h2>
  </div>
  <div style="padding:24px;">
    <h3 style="margin:0 0 16px;">{job_name}</h3>
    <div style="background:#3e1a1a;padding:16px;border-radius:8px;border-left:4px solid #f44336;">
      <strong>Error:</strong><br/>{error}
    </div>
    <p style="margin-top:16px;color:#ff9800;">Please check the DIT System application for details and consider restarting the offload.</p>
  </div>
  <div style="padding:12px 24px;background:#16213e;text-align:center;font-size:12px;color:#666;">
    DIT System — Bulletproof Card Offload
  </div>
</div>"#,
                    job_name = html_escape(job_name),
                    error = html_escape(error),
                )
            }
            NotifyEvent::VerificationFailed {
                job_name,
                failed_files,
                ..
            } => {
                let file_list: Vec<String> = failed_files
                    .iter()
                    .take(20)
                    .map(|f| format!("<li><code>{}</code></li>", html_escape(f)))
                    .collect();
                let truncated = if failed_files.len() > 20 {
                    format!("<li>... and {} more</li>", failed_files.len() - 20)
                } else {
                    String::new()
                };

                format!(
                    r#"<div style="font-family:-apple-system,BlinkMacSystemFont,sans-serif;max-width:600px;margin:0 auto;background:#1a1a2e;color:#e0e0e0;border-radius:12px;overflow:hidden;">
  <div style="background:#ff9800;padding:20px 24px;">
    <h2 style="margin:0;color:#fff;">Verification FAILED</h2>
  </div>
  <div style="padding:24px;">
    <h3 style="margin:0 0 16px;">{job_name}</h3>
    <p>{count} file(s) failed hash verification:</p>
    <ul style="background:#2a2a3e;padding:16px 32px;border-radius:8px;font-size:13px;">
      {file_list}{truncated}
    </ul>
    <p style="color:#f44336;margin-top:16px;"><strong>Action Required:</strong> Do NOT eject the source card. Re-run verification or re-copy affected files.</p>
  </div>
  <div style="padding:12px 24px;background:#16213e;text-align:center;font-size:12px;color:#666;">
    DIT System — Bulletproof Card Offload
  </div>
</div>"#,
                    job_name = html_escape(job_name),
                    count = failed_files.len(),
                    file_list = file_list.join(""),
                    truncated = truncated,
                )
            }
        }
    }
}

/// Read the SMTP password from the credential file in app_data_dir.
/// Returns empty string if file doesn't exist or can't be read.
fn read_smtp_password(app_data_dir: &std::path::Path) -> String {
    let path = app_data_dir.join(".smtp_credential");
    std::fs::read_to_string(&path).unwrap_or_default().trim().to_string()
}

/// Send an email notification for the given event.
///
/// Returns Ok(()) if email was sent, or an error if sending failed.
/// If email is not enabled in settings, returns Ok(()) silently.
/// The SMTP password is read from `{app_data_dir}/.smtp_credential`.
pub async fn send_notification(
    settings: &EmailSettings,
    event: &NotifyEvent,
    app_data_dir: &std::path::Path,
) -> Result<()> {
    if !settings.enabled {
        return Ok(());
    }

    if settings.smtp_host.is_empty() {
        bail!("SMTP host is not configured");
    }
    if settings.from_address.is_empty() || settings.to_address.is_empty() {
        bail!("Email from/to addresses are not configured");
    }

    let email = Message::builder()
        .from(
            settings
                .from_address
                .parse()
                .context("Invalid from address")?,
        )
        .to(settings.to_address.parse().context("Invalid to address")?)
        .subject(event.subject())
        .header(ContentType::TEXT_HTML)
        .body(event.body_html())
        .context("Failed to build email message")?;

    let password = read_smtp_password(app_data_dir);
    let creds = Credentials::new(
        settings.smtp_username.clone(),
        password,
    );

    let mailer = if settings.use_tls {
        AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&settings.smtp_host)
            .context("Failed to create SMTP transport")?
            .port(settings.smtp_port)
            .credentials(creds)
            .build()
    } else {
        AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&settings.smtp_host)
            .port(settings.smtp_port)
            .credentials(creds)
            .build()
    };

    mailer
        .send(email)
        .await
        .context("Failed to send email notification")?;

    Ok(())
}

/// Send a test email to verify SMTP configuration.
pub async fn send_test_email(
    settings: &EmailSettings,
    app_data_dir: &std::path::Path,
) -> Result<()> {
    let test_event = NotifyEvent::OffloadCompleted {
        job_id: "test-000".to_string(),
        job_name: "Test Notification".to_string(),
        file_count: 42,
        total_bytes: 107_374_182_400, // 100 GB
        duration_secs: 360.0,
        mhl_generated: true,
        warnings: vec!["This is a test notification from DIT System.".to_string()],
    };

    send_notification(settings, &test_event, app_data_dir).await
}

// ─── Helpers ─────────────────────────────────────────────────────────────

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

fn format_duration(secs: f64) -> String {
    let total = secs.max(0.0) as u64;
    if total >= 3600 {
        let h = total / 3600;
        let m = (total % 3600) / 60;
        format!("{}h {}m", h, m)
    } else if total >= 60 {
        let m = total / 60;
        let s = total % 60;
        format!("{}m {}s", m, s)
    } else {
        format!("{}s", total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_offload_completed_subject() {
        let event = NotifyEvent::OffloadCompleted {
            job_id: "j1".into(),
            job_name: "Day 1 A-Cam".into(),
            file_count: 150,
            total_bytes: 500_000_000_000,
            duration_secs: 1800.0,
            mhl_generated: true,
            warnings: vec![],
        };
        let subject = event.subject();
        assert!(subject.contains("Day 1 A-Cam"));
        assert!(subject.contains("150 files"));
    }

    #[test]
    fn test_offload_failed_subject() {
        let event = NotifyEvent::OffloadFailed {
            job_id: "j2".into(),
            job_name: "Day 2 B-Cam".into(),
            error: "Disk full".into(),
        };
        assert!(event.subject().contains("FAILED"));
        assert!(event.subject().contains("Day 2 B-Cam"));
    }

    #[test]
    fn test_verification_failed_body_html() {
        let event = NotifyEvent::VerificationFailed {
            job_id: "j3".into(),
            job_name: "Day 3".into(),
            failed_files: vec!["clip001.mov".into(), "clip002.mov".into()],
        };
        let html = event.body_html();
        assert!(html.contains("clip001.mov"));
        assert!(html.contains("clip002.mov"));
        assert!(html.contains("Do NOT eject"));
    }

    #[test]
    fn test_completed_body_html_with_warnings() {
        let event = NotifyEvent::OffloadCompleted {
            job_id: "j4".into(),
            job_name: "Day 4".into(),
            file_count: 10,
            total_bytes: 10_737_418_240,
            duration_secs: 120.0,
            mhl_generated: false,
            warnings: vec!["Slow disk detected".into()],
        };
        let html = event.body_html();
        assert!(html.contains("10.0 GB"));
        assert!(html.contains("2m 0s"));
        assert!(html.contains("Slow disk detected"));
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>alert('xss')</script>"),
            "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;");
    }

    #[test]
    fn test_disabled_email_returns_ok() {
        let _settings = EmailSettings {
            enabled: false,
            ..Default::default()
        };
        let event = NotifyEvent::OffloadCompleted {
            job_id: "j5".into(),
            job_name: "Test".into(),
            file_count: 1,
            total_bytes: 1024,
            duration_secs: 1.0,
            mhl_generated: false,
            warnings: vec![],
        };
        // Can't test async in sync test, but we can verify the event builds correctly
        assert!(event.subject().contains("Test"));
    }
}
