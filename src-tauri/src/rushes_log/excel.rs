//! Excel (.xlsx) export for Rushes Log reports.
//!
//! Uses `rust_xlsxwriter` (pure Rust, no C dependencies) to generate
//! formatted spreadsheets with:
//! - Bold header row with grey background
//! - Alternating row colors
//! - Red highlight for failed entries
//! - Summary row at the bottom
//! - Auto column widths

use anyhow::{Context, Result};
use rust_xlsxwriter::{Color, Format, Workbook};
use std::path::Path;

use super::{format_bytes, format_duration, RushesLogReport};

/// Export a rushes log report to an Excel (.xlsx) file.
pub fn export_xlsx(report: &RushesLogReport, output_path: &Path) -> Result<String> {
    let mut workbook = Workbook::new();

    // Sheet name: shoot date
    let sheet_name = format!("Rushes {}", report.shoot_date);
    let worksheet = workbook.add_worksheet();
    worksheet
        .set_name(&sheet_name)
        .context("Failed to set sheet name")?;

    // ── Formats ──
    let header_format = Format::new()
        .set_bold()
        .set_background_color(Color::RGB(0xD9D9D9))
        .set_border_bottom(rust_xlsxwriter::FormatBorder::Thin)
        .set_font_size(11.0);

    let normal_format = Format::new().set_font_size(10.0);

    let alt_format = Format::new()
        .set_font_size(10.0)
        .set_background_color(Color::RGB(0xF5F5F5));

    let failed_format = Format::new()
        .set_font_size(10.0)
        .set_background_color(Color::RGB(0xFFD9D9))
        .set_font_color(Color::RGB(0xCC0000));

    let summary_format = Format::new()
        .set_bold()
        .set_font_size(10.0)
        .set_border_top(rust_xlsxwriter::FormatBorder::Thin);

    let num_format = Format::new()
        .set_font_size(10.0)
        .set_align(rust_xlsxwriter::FormatAlign::Right);

    let num_alt_format = Format::new()
        .set_font_size(10.0)
        .set_background_color(Color::RGB(0xF5F5F5))
        .set_align(rust_xlsxwriter::FormatAlign::Right);

    let num_failed_format = Format::new()
        .set_font_size(10.0)
        .set_background_color(Color::RGB(0xFFD9D9))
        .set_font_color(Color::RGB(0xCC0000))
        .set_align(rust_xlsxwriter::FormatAlign::Right);

    // ── Headers ──
    let headers = [
        "Thumb",
        "Reel",
        "Camera",
        "Model",
        "Clips",
        "First Clip",
        "Last Clip",
        "Size",
        "Duration",
        "Speed (MB/s)",
        "Status",
        "MHL",
        "Resolution",
        "Frame Rate",
        "Codec",
        "Color Space",
        "Timecode",
        "Source",
        "Destinations",
        "Start Time",
        "End Time",
    ];

    // Column widths (approximate)
    let col_widths: &[f64] = &[
        12.0, 12.0, 12.0, 14.0, 6.0, 24.0, 24.0, 10.0, 10.0, 10.0, 10.0, 5.0, 12.0, 10.0, 14.0, 12.0,
        14.0, 28.0, 30.0, 20.0, 20.0,
    ];

    for (col, header) in headers.iter().enumerate() {
        worksheet
            .write_string_with_format(0, col as u16, *header, &header_format)
            .context("Failed to write header")?;
        if col < col_widths.len() {
            worksheet.set_column_width(col as u16, col_widths[col])?;
        }
    }

    // ── Data Rows ──
    for (idx, entry) in report.entries.iter().enumerate() {
        let row = (idx + 1) as u32;
        let is_failed = entry.backup_status == "Failed" || entry.failed_files > 0;
        let is_alt = idx % 2 == 1;

        // Set row height to accommodate thumbnail
        worksheet.set_row_height(row, 45.0)?;

        let txt_fmt = if is_failed {
            &failed_format
        } else if is_alt {
            &alt_format
        } else {
            &normal_format
        };

        let n_fmt = if is_failed {
            &num_failed_format
        } else if is_alt {
            &num_alt_format
        } else {
            &num_format
        };

        // Insert thumbnail if available (Col 0)
        if let Some(ref thumb_path) = entry.thumbnail_path {
            if Path::new(thumb_path).exists() {
                if let Ok(image) = rust_xlsxwriter::Image::new(thumb_path) {
                    let image = image.set_scale_width(0.18).set_scale_height(0.18);
                    let _ = worksheet.insert_image(row, 0, &image);
                }
            }
        }

        worksheet.write_string_with_format(row, 1, &entry.reel_name, txt_fmt)?;
        worksheet.write_string_with_format(row, 2, &entry.camera_brand, txt_fmt)?;
        worksheet.write_string_with_format(row, 3, &entry.camera_model, txt_fmt)?;
        worksheet.write_number_with_format(row, 4, entry.clip_count as f64, n_fmt)?;
        worksheet.write_string_with_format(row, 5, &entry.first_clip, txt_fmt)?;
        worksheet.write_string_with_format(row, 6, &entry.last_clip, txt_fmt)?;
        worksheet.write_string_with_format(row, 7, format_bytes(entry.total_size), n_fmt)?;
        worksheet.write_string_with_format(
            row,
            8,
            format_duration(entry.duration_seconds),
            n_fmt,
        )?;
        worksheet.write_number_with_format(
            row,
            9,
            (entry.avg_speed_mbps * 10.0).round() / 10.0,
            n_fmt,
        )?;
        worksheet.write_string_with_format(row, 10, &entry.backup_status, txt_fmt)?;
        worksheet.write_string_with_format(
            row,
            11,
            if entry.mhl_verified { "Yes" } else { "No" },
            txt_fmt,
        )?;
        worksheet.write_string_with_format(
            row,
            12,
            entry.resolution.as_deref().unwrap_or(""),
            txt_fmt,
        )?;
        worksheet.write_string_with_format(
            row,
            13,
            entry.frame_rate.as_deref().unwrap_or(""),
            n_fmt,
        )?;
        worksheet.write_string_with_format(
            row,
            14,
            entry.codec.as_deref().unwrap_or(""),
            txt_fmt,
        )?;
        worksheet.write_string_with_format(
            row,
            15,
            entry.color_space.as_deref().unwrap_or(""),
            txt_fmt,
        )?;
        worksheet.write_string_with_format(
            row,
            16,
            entry.timecode_range.as_deref().unwrap_or(""),
            txt_fmt,
        )?;
        worksheet.write_string_with_format(row, 17, &entry.source_path, txt_fmt)?;
        worksheet.write_string_with_format(row, 18, entry.dest_paths.join("; "), txt_fmt)?;
        worksheet.write_string_with_format(row, 19, &entry.started_at, txt_fmt)?;
        worksheet.write_string_with_format(row, 20, &entry.completed_at, txt_fmt)?;
    }

    // ── Summary Row ──
    let summary_row = (report.entries.len() + 2) as u32;
    worksheet.write_string_with_format(summary_row, 1, "TOTAL", &summary_format)?;
    worksheet.write_string_with_format(
        summary_row,
        2,
        format!("{} reels", report.summary.total_reels),
        &summary_format,
    )?;
    worksheet.write_string_with_format(summary_row, 3, "", &summary_format)?;
    worksheet.write_number_with_format(
        summary_row,
        4,
        report.summary.total_clips as f64,
        &summary_format,
    )?;
    worksheet.write_string_with_format(summary_row, 5, "", &summary_format)?;
    worksheet.write_string_with_format(summary_row, 6, "", &summary_format)?;
    worksheet.write_string_with_format(
        summary_row,
        7,
        format_bytes(report.summary.total_size),
        &summary_format,
    )?;
    worksheet.write_string_with_format(
        summary_row,
        8,
        format_duration(report.summary.total_duration_seconds),
        &summary_format,
    )?;

    if !report.summary.cameras_used.is_empty() {
        worksheet.write_string_with_format(
            summary_row + 1,
            0,
            format!("Cameras: {}", report.summary.cameras_used.join(", ")),
            &normal_format,
        )?;
    }

    worksheet.write_string_with_format(
        summary_row + 2,
        0,
        format!("Generated by DIT Pro — {}", report.generated_at),
        &normal_format,
    )?;

    // Save
    workbook
        .save(output_path)
        .with_context(|| format!("Failed to save Excel file to {:?}", output_path))?;

    Ok(output_path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rushes_log::{RushesLogEntry, RushesLogSummary};

    #[test]
    fn test_export_xlsx_basic() {
        let report = RushesLogReport {
            generated_at: "2026-03-09 10:00:00".to_string(),
            shoot_date: "2026-03-09".to_string(),
            entries: vec![RushesLogEntry {
                job_id: "j1".to_string(),
                job_name: "Test".to_string(),
                reel_name: "A001".to_string(),
                camera_brand: "ARRI".to_string(),
                camera_model: "ALEXA Mini".to_string(),
                clip_count: 10,
                first_clip: "A001C001.ari".to_string(),
                last_clip: "A001C010.ari".to_string(),
                source_path: "/Volumes/CARD".to_string(),
                total_size: 10_737_418_240,
                total_files: 10,
                completed_files: 10,
                failed_files: 0,
                duration_seconds: 300.0,
                avg_speed_mbps: 34.1,
                backup_status: "Verified".to_string(),
                mhl_verified: true,
                dest_paths: vec!["/Volumes/SSD1".to_string()],
                started_at: "2026-03-09 09:00:00".to_string(),
                completed_at: "2026-03-09 09:05:00".to_string(),
                resolution: Some("4096x2160".to_string()),
                frame_rate: Some("23.976".to_string()),
                codec: Some("Apple ProRes".to_string()),
                color_space: Some("bt709".to_string()),
                timecode_range: Some("01:00:00:00".to_string()),
                thumbnail_path: None,
                }],            summary: RushesLogSummary {
                total_reels: 1,
                total_clips: 10,
                total_size: 10_737_418_240,
                total_duration_seconds: 300.0,
                cameras_used: vec!["ARRI".to_string()],
            },
        };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.xlsx");
        let result = export_xlsx(&report, &path);
        assert!(result.is_ok());
        assert!(path.exists());
        assert!(std::fs::metadata(&path).unwrap().len() > 0);
    }
}
