use std::fs;
use std::path::{Path, PathBuf};
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

/// Check if the frame extracted at a timestamp is black using FFmpeg's blackframe filter.
fn is_black_frame(video_path: &str, timestamp: f64) -> Result<bool> {
    let output = Command::new("ffmpeg")
        .args([
            "-ss", &format!("{:.3}", timestamp),
            "-i", video_path,
            "-t", "1",
            "-vf", "blackframe=99:32",
            "-an",
            "-f", "null",
            "-",
        ])
        .output()
        .with_context(|| "Failed to run ffmpeg for blackframe detection")?;

    Ok(String::from_utf8_lossy(&output.stderr).contains("blackframe"))
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
        eprintln!("Please provide a file or directory.");
        std::process::exit(1);
    }

    let input_path = Path::new(&args[1]);

    if input_path.is_dir() {
        for entry in fs::read_dir(input_path)? {
            let path = entry?.path();
            if path.is_file() && is_video_file(&path) {
                let output_image = path.with_extension("jpg");
                println!("Processing: {}", path.display());
                if let Err(e) = create_thumbnail_mosaic(
                    path.to_str().unwrap(),
                    output_image.to_str().unwrap(),
                    3, 3, 9
                ) {
                    eprintln!("Failed to process {}: {}", path.display(), e);
                }
            }
        }
    } else if input_path.is_file() {
        let output_image = format!("{}_tn.jpg", input_path.to_string_lossy());
        println!("Processing: {}", input_path.display());
        create_thumbnail_mosaic(&args[1], &output_image, 3, 3, 9)?;
    } else {
        eprintln!("Invalid input path.");
        std::process::exit(1);
    }

    Ok(())
}

/// Check if a file is a video based on extension.
fn is_video_file(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase().as_str(),
        "mp4" | "mov" | "avi" | "mkv" | "webm" | "m4v" | "wmv" | "mpg" | "mpeg" | "ts"
    )
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
    let duration = get_video_duration(video_path)?;
    let interval = duration / total_frames as f64;

    // === Extract evenly spaced thumbnails with retry ===
    for i in 0..total_frames {
        let mut timestamp = interval * i as f64;
        let max_attempts = 5;
        let mut attempt = 0;

        let output_file = temp_dir.path().join(format!("thumb_{:03}.jpg", i));
        let output_file_str = output_file.to_str().unwrap();

        loop {
            Command::new("ffmpeg")
                .args([
                    "-ss", &format!("{:.3}", timestamp),
                    "-i", video_path,
                    "-frames:v", "1",
                    "-q:v", "2",
                    "-y",
                    output_file_str,
                ])
                .status()
                .with_context(|| format!("Failed to extract thumbnail at {:.3}s", timestamp))?;

            if !is_black_frame(video_path, timestamp)? || attempt >= max_attempts {
                break;
            }

            attempt += 1;
            timestamp += 2.0; // Try 2s later
        }
    }

    // === Create mosaic ===
    let mosaic_temp = temp_dir.path().join("mosaic_raw.jpg");
    let input_pattern = temp_dir.path().join("thumb_%03d.jpg");

    Command::new("ffmpeg")
        .args([
            "-f", "image2",
            "-i", input_pattern.to_str().unwrap(),
            "-filter_complex",
            &format!("tile={}x{}", cols, rows),
            "-y",
            mosaic_temp.to_str().unwrap(),
        ])
        .status()
        .with_context(|| "Failed to create mosaic with ffmpeg")?;

    // === Metadata ===
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
    let filename = Path::new(video_path).file_name().unwrap().to_string_lossy();
    let font_path = find_default_font().ok_or_else(|| anyhow::anyhow!("No usable system font found for drawtext"))?;
    let filesize_mb = get_filesize_mb(video_path)?;

    // === Text Overlay ===
    let raw_text = format!("File:{} Size:{:.2} MB Resolution:({})", filename, filesize_mb, resolution);
    let escaped_text = escape_ffmpeg_drawtext_text(&raw_text);
    let escaped_font_path = escape_ffmpeg_drawtext_text(&font_path);

    let drawtext_filter = format!(
        "drawtext=fontfile='{}':text='{}':x=10:y=10:fontsize=96:fontcolor=white:box=1:boxcolor=black@0.5",
        escaped_font_path, escaped_text
    );

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
