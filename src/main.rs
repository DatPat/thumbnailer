use std::fs;
use std::path::Path;
use std::process::Command;
use anyhow::{Context, Result};
use tempfile::tempdir;
use std::env;

/// Find a default system font path for use in FFmpeg's drawtext.
fn find_default_font() -> Option<String> {
    let font_paths = vec![
        // Linux
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
        // macOS
        "/System/Library/Fonts/SFNSDisplay.ttf",
        "/Library/Fonts/Arial.ttf",
        // Windows
        "C:/Windows/Fonts/arial.ttf",
        "C:/Windows/Fonts/segoeui.ttf",
    ];

    font_paths.into_iter().find(|path| fs::metadata(path).is_ok()).map(String::from)
}

/// Get file size in megabytes.
fn get_filesize_mb(path: &str) -> Result<f64> {
    let size_bytes = fs::metadata(path)?.len();
    Ok(size_bytes as f64 / 1_000_000.0)
}

/// Get video duration in seconds using ffprobe.
fn get_video_duration(video_path: &str) -> Result<f64> {
    let output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            video_path,
        ])
        .output()
        .with_context(|| "Failed to get video duration with ffprobe")?;

    let duration_str = String::from_utf8_lossy(&output.stdout);
    let duration: f64 = duration_str.trim().parse()
        .with_context(|| format!("Failed to parse video duration: {}", duration_str))?;

    Ok(duration)
}

/// Escape text for FFmpeg drawtext filter.
fn escape_ffmpeg_drawtext_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// Main entry point.
fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    
    if args.len() < 2 {
        eprintln!("Please provide a file name.");
        std::process::exit(1);
    }
    
    let input_video = &args[1]; // Adjust this path as needed
    let output_image = format!("{}_tn.jpg", input_video);
    let rows = 3;
    let cols = 3;
    let total_thumbs = rows * cols;

    println!("Video Thumbnail Creator (C) 1994 by Trash Corp");
    create_thumbnail_mosaic(&input_video, &output_image, rows, cols, total_thumbs)?;
    println!("Mosaic created: {}", &output_image);

    Ok(())
}

/// Create a thumbnail mosaic from video and overlay metadata text.
fn create_thumbnail_mosaic(
    video_path: &str,
    output_image: &str,
    rows: usize,
    cols: usize,
    total_frames: usize,
) -> Result<()> {
    let temp_dir = tempdir()?;

    // === Extract evenly spaced thumbnails ===
    let duration = get_video_duration(video_path)?;
    let interval = duration / total_frames as f64;

    for i in 0..total_frames {
        let timestamp = interval * i as f64;
        let output_file = temp_dir.path().join(format!("thumb_{:03}.jpg", i));
        let output_file_str = output_file.to_str().unwrap();

        Command::new("ffmpeg")
            .args([
                "-ss", &format!("{:.3}", timestamp),
                "-i", video_path,
                "-frames:v", "1",
                "-q:v", "2",
                output_file_str,
            ])
            .status()
            .with_context(|| format!("Failed to extract thumbnail at {:.3}s", timestamp))?;
    }

    // === Create mosaic image ===
    let mosaic_temp = temp_dir.path().join("mosaic_raw.jpg");
    let input_pattern = temp_dir.path().join("thumb_%03d.jpg");

    Command::new("ffmpeg")
        .args([
            "-f", "image2",
            "-i", input_pattern.to_str().unwrap(),
            "-filter_complex",
            &format!("tile={}x{}", cols, rows),
            mosaic_temp.to_str().unwrap(),
        ])
        .status()
        .with_context(|| "Failed to create mosaic with ffmpeg")?;

    // === Extract video metadata ===
    let ffprobe_output = Command::new("ffprobe")
        .args([
            "-v", "error",
            "-select_streams", "v:0",
            "-show_entries", "stream=width,height",
            "-of", "csv=s=x:p=0",
            video_path,
        ])
        .output()
        .with_context(|| "Failed to run ffprobe for resolution")?;

    let resolution = String::from_utf8_lossy(&ffprobe_output.stdout).trim().to_string();
    let filename = Path::new(video_path)
        .file_name()
        .unwrap()
        .to_string_lossy();

    let font_path = find_default_font()
        .ok_or_else(|| anyhow::anyhow!("No usable system font found for drawtext"))?;

    let filesize_mb = get_filesize_mb(video_path)?;

    // === Prepare overlay text ===
    let raw_text = format!(
        "File:{} Size:{:.2} MB Resolution:({})",
        filename, filesize_mb, resolution
    );
    let escaped_text = escape_ffmpeg_drawtext_text(&raw_text);
    let escaped_font_path = escape_ffmpeg_drawtext_text(&font_path);

    let drawtext_filter = format!(
        "drawtext=fontfile='{}':text='{}':x=10:y=10:fontsize=96:fontcolor=white:box=1:boxcolor=black@0.5",
        escaped_font_path, escaped_text
    );

    // === Apply drawtext overlay ===
    Command::new("ffmpeg")
        .args([
            "-i", mosaic_temp.to_str().unwrap(),
            "-vf", &drawtext_filter,
            "-y", output_image,
        ])
        .status()
        .with_context(|| "Failed to overlay text on mosaic")?;

    Ok(())
}
